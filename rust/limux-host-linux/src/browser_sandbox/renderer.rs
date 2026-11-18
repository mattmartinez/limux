//! Keep the sandboxed browser working when WebKit cannot set up the GPU's GBM
//! display inside its sandbox.
//!
//! WebKitGTK's sandbox binds `/dev/nvidia0` and `/dev/nvidiactl` only. When the
//! display runs on another NVIDIA GPU (`/dev/nvidia1`, ...), the web process
//! cannot create its GBM EGL display and aborts. With
//! `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1` it hands frames over in shared memory
//! instead, starts, stays sandboxed and draws correctly.
//! (`WEBKIT_DMABUF_RENDERER_DISABLE_GBM=1` also passes the check, but pages
//! then draw as garbage or not at all.)
//!
//! Each abort leaves a crash record, so Limux avoids the attempt that aborts:
//! - when a connected display runs on an NVIDIA GPU other than `/dev/nvidia0`,
//!   the check starts with that setting;
//! - otherwise it checks the default, and on this exact abort retries with the
//!   setting and remembers that for the WebKit version, NVIDIA driver and GPUs,
//!   so later launches start with it.
//!
//! Limux then keeps the setting for its own WebKit.

use super::BrowserSandbox;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) const FORCE_SHM: &str = "WEBKIT_DMABUF_RENDERER_FORCE_SHM";
/// WebKit's message when its web process cannot create the GBM EGL display.
const GBM_FAILURE: &str = "Could not create GBM EGL display";
/// Renderer settings a user may choose. With any of them set, Limux checks the
/// browser as configured and changes nothing.
const USER_RENDERER_VARS: [&str; 4] = [
    FORCE_SHM,
    "WEBKIT_DMABUF_RENDERER_DISABLE_GBM",
    "WEBKIT_DISABLE_DMABUF_RENDERER",
    "WEBKIT_WEB_RENDER_DEVICE_FILE",
];

static SHM_FORCED_BY_LIMUX: AtomicBool = AtomicBool::new(false);

type Probe<'a> = &'a dyn Fn(&[(&str, &str)]) -> Result<BrowserSandbox, String>;

/// Run the browser check, falling back to shared-memory frames as described
/// above.
pub(super) fn check(probe: Probe) -> Result<BrowserSandbox, String> {
    let user_override = USER_RENDERER_VARS
        .iter()
        .any(|name| std::env::var_os(name).is_some());
    let unbound_display_gpu = display_on_unbound_nvidia_gpu(
        Path::new("/sys/class/drm"),
        Path::new("/proc/driver/nvidia/gpus"),
    );
    let memory = Memory::locate();
    let outcome = decide(probe, user_override, unbound_display_gpu, memory.as_ref());
    if outcome.shm_forced {
        std::env::set_var(FORCE_SHM, "1");
        SHM_FORCED_BY_LIMUX.store(true, Ordering::Relaxed);
    }
    outcome.result
}

/// What shells started by Limux get instead of the setting Limux made for its
/// own WebKit: "0" is WebKit's default, so their programs see no change.
pub(super) fn terminal_child_env() -> Option<(String, String)> {
    SHM_FORCED_BY_LIMUX
        .load(Ordering::Relaxed)
        .then(|| (FORCE_SHM.to_string(), "0".to_string()))
}

struct Outcome {
    result: Result<BrowserSandbox, String>,
    shm_forced: bool,
}

fn log(message: &str) {
    eprintln!("limux: browser check: {message}");
}

fn first_line(error: &str) -> &str {
    error.lines().next().unwrap_or_default()
}

/// `unbound_display_gpu` is the NVIDIA device minor of a display GPU the
/// sandbox cannot open (see `display_on_unbound_nvidia_gpu`).
fn decide(
    probe: Probe,
    user_override: bool,
    unbound_display_gpu: Option<u32>,
    memory: Option<&Memory>,
) -> Outcome {
    let shm: &[(&str, &str)] = &[(FORCE_SHM, "1")];
    if user_override {
        log("a WebKit renderer variable is set; checking the browser as configured");
        return Outcome {
            result: probe(&[]),
            shm_forced: false,
        };
    }
    let remembered = memory.filter(|memory| memory.recalls_shm());
    if unbound_display_gpu.is_some() || remembered.is_some() {
        match unbound_display_gpu {
            Some(minor) => log(&format!(
                "the display runs on /dev/nvidia{minor}, which WebKit's sandbox does not open; checking with shared-memory frames"
            )),
            None => log(
                "this WebKit and GPU setup needed shared-memory frames before; checking with them",
            ),
        }
        match probe(shm) {
            Ok(sandbox) => {
                log("works with shared-memory frames");
                return Outcome {
                    result: Ok(sandbox),
                    shm_forced: true,
                };
            }
            Err(error) => {
                log(&format!(
                    "failed with shared-memory frames ({}); checking the default",
                    first_line(&error)
                ));
                if let Some(memory) = remembered {
                    memory.forget();
                }
            }
        }
    }
    match probe(&[]) {
        Err(error) if error.contains(GBM_FAILURE) => {
            log("WebKit could not set up the GPU's GBM display in its sandbox; checking again with shared-memory frames");
            match probe(shm) {
                Ok(sandbox) => {
                    log("works with shared-memory frames; Limux's browser uses them from now on");
                    if let Some(memory) = memory {
                        memory.remember();
                    }
                    Outcome {
                        result: Ok(sandbox),
                        shm_forced: true,
                    }
                }
                Err(retry_error) => {
                    log(&format!(
                        "also failed with shared-memory frames: {}",
                        first_line(&retry_error)
                    ));
                    Outcome {
                        result: Err(error),
                        shm_forced: false,
                    }
                }
            }
        }
        result => Outcome {
            result,
            shm_forced: false,
        },
    }
}

/// The NVIDIA device minor of a GPU that drives a connected display but that
/// WebKit's sandbox does not open (any minor but 0), if there is one. Reads the
/// connectors under `drm` (`/sys/class/drm/cardN-*/status`), each card's PCI
/// address (`cardN/device`), and that GPU's minor under `nvidia_gpus`
/// (`/proc/driver/nvidia/gpus/<address>/information`).
fn display_on_unbound_nvidia_gpu(drm: &Path, nvidia_gpus: &Path) -> Option<u32> {
    let mut connectors: Vec<_> = fs::read_dir(drm).ok()?.flatten().collect();
    connectors.sort_by_key(|entry| entry.file_name());
    connectors.into_iter().find_map(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        let (card, _) = name.split_once('-')?;
        if !card.starts_with("card") {
            return None;
        }
        let status = fs::read_to_string(entry.path().join("status")).ok()?;
        if status.trim() != "connected" {
            return None;
        }
        let device = fs::read_link(drm.join(card).join("device")).ok()?;
        let address = device.file_name()?;
        let information = fs::read_to_string(nvidia_gpus.join(address).join("information")).ok()?;
        let minor = information
            .lines()
            .find_map(|line| line.strip_prefix("Device Minor:"))?
            .trim()
            .parse::<u32>()
            .ok()?;
        (minor != 0).then_some(minor)
    })
}

/// A cache file that holds the setup (WebKit version, NVIDIA driver, GPUs) that
/// needed shared-memory frames. A change to any of them makes Limux check the
/// default again.
struct Memory {
    path: PathBuf,
    setup: String,
}

impl Memory {
    fn locate() -> Option<Self> {
        Some(Self {
            path: dirs::cache_dir()?.join("limux").join("browser-force-shm"),
            setup: current_setup(),
        })
    }

    fn recalls_shm(&self) -> bool {
        fs::read_to_string(&self.path).is_ok_and(|saved| saved == self.setup)
    }

    fn remember(&self) {
        let written = self
            .path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&self.path, &self.setup));
        if let Err(error) = written {
            log(&format!(
                "could not remember the fallback in {}: {error}",
                self.path.display()
            ));
        }
    }

    fn forget(&self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn current_setup() -> String {
    let driver = fs::read_to_string("/proc/driver/nvidia/version")
        .ok()
        .and_then(|version| version.lines().next().map(str::to_string))
        .unwrap_or_default();
    let mut gpus: Vec<String> = fs::read_dir("/proc/driver/nvidia/gpus")
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    gpus.sort();
    format!(
        "webkit {}.{}.{}\nnvidia {driver}\ngpus {}\n",
        webkit6::functions::major_version(),
        webkit6::functions::minor_version(),
        webkit6::functions::micro_version(),
        gpus.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const GBM_ABORT: &str = "Browser sandbox unavailable. WebKit startup check exited with exit status: 1.\nCould not create GBM EGL display: EGL_NOT_INITIALIZED. Aborting...";

    fn sandboxed() -> BrowserSandbox {
        BrowserSandbox { sandboxed: true }
    }

    /// A probe that fails with the GBM abort unless shared-memory frames are
    /// forced, and records which calls forced them.
    struct FakeProbe {
        calls: RefCell<Vec<bool>>,
        shm_works: bool,
    }

    impl FakeProbe {
        fn new(shm_works: bool) -> Self {
            Self {
                calls: RefCell::default(),
                shm_works,
            }
        }

        fn run(&self, env: &[(&str, &str)]) -> Result<BrowserSandbox, String> {
            let shm = env.contains(&(FORCE_SHM, "1"));
            self.calls.borrow_mut().push(shm);
            if shm && self.shm_works {
                Ok(sandboxed())
            } else if shm {
                Err("Browser sandbox unavailable. something else".to_string())
            } else {
                Err(GBM_ABORT.to_string())
            }
        }

        fn shm_calls(&self) -> Vec<bool> {
            self.calls.borrow().clone()
        }
    }

    fn memory(dir: &tempfile::TempDir) -> Memory {
        Memory {
            path: dir.path().join("limux/browser-force-shm"),
            setup: "webkit 2.54.1\nnvidia test\ngpus a b\n".to_string(),
        }
    }

    #[test]
    fn a_gbm_abort_is_retried_with_shared_memory_and_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let memory = memory(&dir);
        let fake = FakeProbe::new(true);
        let outcome = decide(&|env| fake.run(env), false, None, Some(&memory));
        assert_eq!(outcome.result, Ok(sandboxed()));
        assert!(outcome.shm_forced);
        assert_eq!(fake.shm_calls(), [false, true]);
        assert!(memory.recalls_shm());
    }

    #[test]
    fn a_remembered_setup_skips_the_attempt_that_aborts() {
        let dir = tempfile::tempdir().unwrap();
        let memory = memory(&dir);
        memory.remember();
        let fake = FakeProbe::new(true);
        let outcome = decide(&|env| fake.run(env), false, None, Some(&memory));
        assert!(outcome.shm_forced);
        assert_eq!(fake.shm_calls(), [true]);
    }

    #[test]
    fn a_changed_setup_checks_the_default_again() {
        let dir = tempfile::tempdir().unwrap();
        memory(&dir).remember();
        let changed = Memory {
            setup: "webkit 2.56.0\nnvidia test\ngpus a b\n".to_string(),
            ..memory(&dir)
        };
        let fake = FakeProbe::new(true);
        decide(&|env| fake.run(env), false, None, Some(&changed));
        assert_eq!(fake.shm_calls(), [false, true]);
        assert!(changed.recalls_shm());
    }

    #[test]
    fn a_remembered_fallback_that_fails_is_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let memory = memory(&dir);
        memory.remember();
        let fake = FakeProbe::new(false);
        let outcome = decide(&|env| fake.run(env), false, None, Some(&memory));
        assert_eq!(outcome.result, Err(GBM_ABORT.to_string()));
        assert!(!outcome.shm_forced);
        assert_eq!(fake.shm_calls(), [true, false, true]);
        assert!(!memory.recalls_shm());
    }

    #[test]
    fn other_failures_are_not_retried() {
        let dir = tempfile::tempdir().unwrap();
        let memory = memory(&dir);
        let calls = RefCell::new(0);
        let probe = |_: &[(&str, &str)]| {
            *calls.borrow_mut() += 1;
            Err::<BrowserSandbox, _>(
                "Browser sandbox unavailable. bwrap: no user namespaces".to_string(),
            )
        };
        let outcome = decide(&probe, false, None, Some(&memory));
        assert!(outcome.result.is_err());
        assert!(!outcome.shm_forced);
        assert_eq!(*calls.borrow(), 1);
        assert!(!memory.recalls_shm());
    }

    #[test]
    fn a_users_renderer_setting_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let memory = memory(&dir);
        memory.remember();
        let fake = FakeProbe::new(true);
        let outcome = decide(&|env| fake.run(env), true, None, Some(&memory));
        assert!(!outcome.shm_forced);
        assert_eq!(fake.shm_calls(), [false]);
    }

    #[test]
    fn a_display_on_an_unbound_nvidia_gpu_never_tries_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let memory = memory(&dir);
        let fake = FakeProbe::new(true);
        let outcome = decide(&|env| fake.run(env), false, Some(1), Some(&memory));
        assert_eq!(outcome.result, Ok(sandboxed()));
        assert!(outcome.shm_forced);
        assert_eq!(fake.shm_calls(), [true]);
    }

    #[test]
    fn if_shared_memory_fails_there_the_default_is_still_checked() {
        let fake = FakeProbe::new(false);
        let outcome = decide(&|env| fake.run(env), false, Some(1), None);
        assert!(outcome.result.is_err());
        assert_eq!(fake.shm_calls(), [true, false, true]);
    }

    /// One card of a fake GPU setup:
    /// `(card, pci address, nvidia minor or None, [(connector, status)])`.
    type FakeCard<'a> = (&'a str, &'a str, Option<u32>, &'a [(&'a str, &'a str)]);

    /// A fake `/sys/class/drm` and `/proc/driver/nvidia/gpus`.
    fn fake_gpus(cards: &[FakeCard]) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let drm = root.path().join("drm");
        let nvidia = root.path().join("nvidia");
        fs::create_dir_all(&drm).unwrap();
        for (card, address, minor, connectors) in cards {
            fs::create_dir_all(drm.join(card)).unwrap();
            std::os::unix::fs::symlink(
                format!("../../devices/pci0000:00/{address}"),
                drm.join(card).join("device"),
            )
            .unwrap();
            if let Some(minor) = minor {
                fs::create_dir_all(nvidia.join(address)).unwrap();
                fs::write(
                    nvidia.join(address).join("information"),
                    format!("Model: \t\t GPU\nDevice Minor: \t {minor}\n"),
                )
                .unwrap();
            }
            for (connector, status) in *connectors {
                let path = drm.join(format!("{card}-{connector}"));
                fs::create_dir_all(&path).unwrap();
                fs::write(path.join("status"), format!("{status}\n")).unwrap();
            }
        }
        (root, drm, nvidia)
    }

    #[test]
    fn finds_a_display_on_the_second_nvidia_gpu() {
        // Monitors on the second NVIDIA GPU (minor 1), none on the first.
        let (_root, drm, nvidia) = fake_gpus(&[
            (
                "card0",
                "0000:04:00.0",
                Some(0),
                &[("DP-4", "disconnected")],
            ),
            (
                "card1",
                "0000:0e:00.0",
                Some(1),
                &[("DP-1", "connected"), ("DP-2", "connected")],
            ),
        ]);
        assert_eq!(display_on_unbound_nvidia_gpu(&drm, &nvidia), Some(1));
    }

    #[test]
    fn a_display_on_nvidia0_or_another_vendor_is_fine() {
        let (_root, drm, nvidia) = fake_gpus(&[
            ("card0", "0000:04:00.0", Some(0), &[("DP-1", "connected")]),
            (
                "card1",
                "0000:0e:00.0",
                Some(1),
                &[("DP-2", "disconnected")],
            ),
            ("card2", "0000:10:00.0", None, &[("eDP-1", "connected")]),
        ]);
        assert_eq!(display_on_unbound_nvidia_gpu(&drm, &nvidia), None);
        assert_eq!(
            display_on_unbound_nvidia_gpu(&drm.join("missing"), &nvidia),
            None
        );
    }

    #[test]
    fn shells_get_webkits_default_back_only_after_limux_forced_shared_memory() {
        SHM_FORCED_BY_LIMUX.store(false, Ordering::Relaxed);
        assert_eq!(terminal_child_env(), None);
        SHM_FORCED_BY_LIMUX.store(true, Ordering::Relaxed);
        assert_eq!(
            terminal_child_env(),
            Some((FORCE_SHM.to_string(), "0".to_string()))
        );
        SHM_FORCED_BY_LIMUX.store(false, Ordering::Relaxed);
    }

    #[test]
    fn the_setup_names_webkit_and_the_nvidia_gpus() {
        let setup = current_setup();
        assert!(setup.starts_with("webkit "));
        assert!(setup.contains("\nnvidia "));
        assert!(setup.contains("\ngpus "));
    }
}
