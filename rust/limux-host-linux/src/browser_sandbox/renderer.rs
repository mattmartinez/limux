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
//! The check retries with that setting, Limux keeps it for its own WebKit, and
//! the result is remembered so later launches skip the attempt that aborts
//! (each abort leaves a crash record).

use super::BrowserSandbox;
use std::fs;
use std::path::PathBuf;
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
    let memory = Memory::locate();
    let outcome = decide(probe, user_override, memory.as_ref());
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

fn decide(probe: Probe, user_override: bool, memory: Option<&Memory>) -> Outcome {
    let shm: &[(&str, &str)] = &[(FORCE_SHM, "1")];
    if user_override {
        log("a WebKit renderer variable is set; checking the browser as configured");
        return Outcome {
            result: probe(&[]),
            shm_forced: false,
        };
    }
    if let Some(memory) = memory.filter(|memory| memory.recalls_shm()) {
        log("this WebKit and GPU setup needed shared-memory frames before; checking with them");
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
                    "failed with shared-memory frames ({}); forgetting that and checking the default",
                    first_line(&error)
                ));
                memory.forget();
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
        let outcome = decide(&|env| fake.run(env), false, Some(&memory));
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
        let outcome = decide(&|env| fake.run(env), false, Some(&memory));
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
        decide(&|env| fake.run(env), false, Some(&changed));
        assert_eq!(fake.shm_calls(), [false, true]);
        assert!(changed.recalls_shm());
    }

    #[test]
    fn a_remembered_fallback_that_fails_is_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let memory = memory(&dir);
        memory.remember();
        let fake = FakeProbe::new(false);
        let outcome = decide(&|env| fake.run(env), false, Some(&memory));
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
        let outcome = decide(&probe, false, Some(&memory));
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
        let outcome = decide(&|env| fake.run(env), true, Some(&memory));
        assert!(!outcome.shm_forced);
        assert_eq!(fake.shm_calls(), [false]);
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
