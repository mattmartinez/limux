# Fork divergence map

Where `mattmartinez/limux` left `am-will/limux`, and what a sync must not overwrite.
Read this before `fork/sync-upstream.sh` or a hand merge. The script aborts on the
first conflict; this file is the resolution.

## Where the fork started

| | |
|---|---|
| Merge base | `32dd2a37f8596f1de35bbf4b54d980ed76fa28b6` |
| Upstream commit | Merge pull request #271, `ci(gtk): retain native crash dumps and backtraces` |
| Date | 2026-11-16 19:59 -0500 |
| Version at the split | `0.1.35` (`Cargo.toml`). The fork has not bumped it. |
| First fork commit | `5b8b897` (2026-11-17) Workspace cards |
| Fork tip when this was written | `623f1ad` on `origin/main` |
| Upstream tip when this was written | `cfc090c` (`upstream/main`), tag `v0.1.36` plus 21 unreleased commits. 57 commits ahead of the base; the fork is 16 ahead. |

Nothing before `32dd2a3` is a fork change. `git diff 32dd2a3..main` is the whole fork.

Remotes: `origin` is this fork, `upstream` is `am-will/limux` (push disabled).
Merge, never rebase, so `main` is never rewritten. Land the result as a PR onto
this fork's `main`, not a force-push, and not a PR to `am-will` unless the
change is the `card-status` subset (no `fork/`, no Ghostty pin move).

`AGENTS.md` and `CLAUDE.md` open with "This checkout is a fork". A sync that conflicts on either file must put that section back. It is how the next agent finds this map.

## What the fork is

cmux-style sidebar cards on top of Limux, plus the Ghostty and browser-check
changes those cards need. Upstream has none of this. A sync takes upstream's
commits everywhere they do not delete or replace the behavior below.

Card order, top to bottom (`window.rs`, `build_sidebar_row` vbox):

1. title (`limux-ws-name`)
2. description (`limux-ws-description`) — saved with the session
3. latest notification (`limux-notify-msg`) — upstream's row, kept
4. status rows (`limux-ws-status`) — memory only, highest priority first; the `pr` key is not here
5. branch, then path, on one row visually but the branch is its own label (`limux-ws-branch`) above the path line
6. pull-request row (`pr` key only), after the path

Chrome (`c54b8a2`, do not restore the old pills):

- Sidebar and window chrome are `#000000` (`workspace_color::CHROME_BLACK`). Selected card fill is `#3e90f7` (`SELECTION_FILL`), not Libadwaita's accent and not the workspace color.
- A workspace color is a 3px left rail. Selection stays the standard blue.
- Committed title is 12px, weight 500 (`BASE_CSS` `.limux-ws-name`). Card text is light regardless of system theme because the card background is always dark. Path is 11px; `card_css` secondary lines are `0.85em`.
- `show_workspace_indicators` defaults to `false`. The settings row "Workspace indicators on the top bar" is gone. Indicator buttons stay hidden. Do not re-show them because upstream toggles `show_workspace_indicators`.

Uncommitted worktree edits, not on `main` and not part of the committed contract: titles `#ffffff` 13px weight 700; secondary lines `#8a8a8a`, `#d0d0d0` when selected. A sync must not revert those if they are still dirty. Do not commit them as part of an upstream merge.

## Fork-only files

These do not exist upstream. A clean merge keeps them. If a merge deletes one, that is a mistake.

| File | What it owns |
|---|---|
| `rust/limux-host-linux/src/workspace_card.rs` | Status map, limits, `parse_card_request`, `status_row_markup`, `git_branch_for` (reads `.git/HEAD`, worktrees included, no `git` process), `workspace_title_from_terminal`, `card_css`. |
| `rust/limux-host-linux/src/program_status.rs` | OSC 7501 store. Key `program_status`. One workspace row from the most urgent tab. Working/blocked end at the next prompt or process exit; done/error stay until the tab is focused. |
| `rust/limux-host-linux/src/window/description_dialog.rs` | "Edit Description…" |
| `rust/limux-host-linux/src/browser_sandbox/renderer.rs` | GBM / second-NVIDIA browser check. See below. |
| `rust/limux-cli/src/card.rs` | `set-status`, `clear-status`, `list-status`, `set-description`, `refresh-pr`, hook lifecycle rows. |
| `hooks/omp-limux-card.ts` | omp extension: notification with the start of the reply, then `refresh-pr`. No-op outside Limux. |
| `fork/` | `sync-upstream.sh`, `card-smoke.sh` (live checks), `PKGBUILD` (`limux-card`), this map. |

## Behavior that must survive

### Cards and the socket

Host-only methods, also on the GTK bridge and the CLI: `workspace.set_status`, `workspace.clear_status`, `workspace.list_status`, `workspace.set_description`. CLI names are `set-status`, `clear-status`, `list-status`, `set-description`, `refresh-pr`. Flags match `cmux set-status`.

Limits, in `workspace_card.rs`: 16 rows, key 32 chars, value 160, description 200, priority within ±1000. Icons: a few cmux SF Symbol names, or up to four characters. Color: `#rrggbb` or a name.

`limux --json list-workspaces` includes `description`, `statuses`, `git_branch`. Those three fields are fork payload. Upstream's `group_id` / `group_ref` cleanup (a local instead of two `group_of` calls) can land; do not drop the three fork fields to take it.

Status rows are not persisted. `description` is, on `WorkspaceState`. `named_by_user` is too.

Hooks (`limux hooks setup`) write fallback Running or Needs input rows, then
clear them when the turn or session ends. The card shows one built-in activity
line, preferring OSC 7501 unless a hook has a more urgent request for input.
User-written statuses and PRs stay visible. Hook notifications retain the
unread marker and desktop alert but skip the card preview. Ignore Claude's
passive idle prompt, not real permission alerts.
Hooks refresh PRs after a turn. Opt out: `LIMUX_CARD_STATUS_DISABLED=1`,
`LIMUX_PR_STATUS_DISABLED=1`.

### Titles follow the focused tab

`refresh_workspace_title` in `window.rs`. A workspace nobody named takes the title of the tab it last had focused (tab name, else the terminal title), passed through `workspace_title_from_terminal`: breaks become spaces, other controls and bidi overrides are refused, cut at 256 with an ellipsis, a lone spinner frame is dropped. `named_by_user` blocks that. A session saved before the field existed counts as user-named unless the name is `default_workspace_name(folder_path)` (`workspace_named_by_user`).

The card's title label wraps at the available width, including long words. It
also shows the current workspace name as a plain-text tooltip after either a
manual rename or an automatic title change.

`pane.rs` keeps the complete terminal title in `automatic_title`. The 22-character
preview belongs only on the tab label, never in the workspace title or tooltip.
Deduplicate OSC title changes against the complete title and still call
`callbacks.on_title_changed`, including when only the suffix changes.
`LIMUX_DEBUG_TITLES=1` logs the workspace-title decision.

Automatic title-only changes are safe to ignore when another window edits or deletes a workspace. Keep true session conflicts recoverable. Reading a card notification clears its unread styling, not its message.

### Path follows the shell

`refresh_workspace_location`. Once a shell reports a directory, the card shows that, not the folder the workspace was opened in. Branch is `git_branch_for` of that directory. `on_pwd_changed` in `window.rs` must still call `refresh_workspace_location`. Upstream's "skip if cwd unchanged" / `try_borrow` is fine around that call. An idle defer is how the fork avoids borrowing `state` re-entrantly; keep a path that refreshes the card after the cwd write.

### OSC 7501 and Ghostty

Submodule pin on the fork: `9f647e126082f93ba36b0d03f282f3fd5f4a8769` from `mattmartinez/ghostty`, branch `osc7501-program-status`. Four commits on upstream's pin `9fb2c115` (still upstream's pin as of `cfc090c`; upstream had not moved Ghostty):

- `5cbdddfd` libghostty-vt: program status protocol
- `cacbc059` OSC 7501 reaches the embedder
- `b7923ba6` ABI size and loop fix
- `9f647e12` `ProgramStatus.C` named for Zig 0.16

`.gitmodules` url is `https://github.com/mattmartinez/ghostty.git`. The AUR source workflow reads the URL and pin from the same Limux commit; the unused Flatpak manifest still names `bvolpato/ghostty`. Do not revert the url or the pin to take upstream's. Inside `ghostty/`, `origin` is bvolpato and `mattmartinez` is the fork.

The four commits change Ghostty's action ABI. Fork bindings assert `ghostty_action_u` = 48 and `ghostty_action_s` = 56. Upstream's tree, against the old pin, asserts 24 and 32. Taking upstream's sizes will not match this Ghostty. Keep 48/56. Upstream scrollbar asserts (`ghostty_surface_scrollbar_s`, size 32) can be added if that struct is in the bindings; they are not a reason to shrink the action union.

Rust side: `program_status_report` in `terminal.rs`, `GHOSTTY_ACTION_PROGRAM_STATUS` in `ghostty_action_cb`, `TerminalCallbacks.on_program_status`, `PaneCallbacks.on_program_status`. On tab exit the fork sends `Report::prompt_start()` so working/blocked rows die with the process. Upstream replaced that `on_close` with exit-hold (`ExitSeen::{Watched,Driven,Unseen}`, callback arg `exit` not `code`). Keep both: clear program status, then run upstream's seen/driven/unseen exit. `LIMUX_DEBUG_PROGRAM_STATUS=1` logs each report. Terminfo lists `Pst`. Limux answers the protocol's support query.

When upstream does move its Ghostty pin, stop. Rebase the four commits onto the new pin inside `ghostty/`, rebuild with `scripts/build-ghostty.sh`, push `osc7501-program-status` to `mattmartinez`, then commit the new pointer. Do not point the submodule at upstream's pin to make the merge clean.

### Restored agent tabs keep shell integration

`RestorableAgentState::resume_command(shell_integration)` and `wrap_restored_agent_command`. When Ghostty's `shell-integration` is not `none`, the restored command ends in `BASH_INTEGRATED_LOGIN_SHELL` (`GHOSTTY_BASH_INJECT=1`), not a plain login shell. `CURRENT_SHELL_INTEGRATION_ENABLED` and `CURRENT_AUTOSTART_SHELL_KIND` in `terminal.rs` are what the host reads at init and on `GHOSTTY_ACTION_RELOAD_CONFIG`. Upstream's side of those two hunks is empty. Keep the stores.

`limux hooks setup omp` installs `hooks/omp-limux-card.ts`; it registers the
exact transcript on `session_start` and `session_switch`. The `omp` hook store
is keyed by tab identity (not cwd or transcript), and restore passes the
registered path to `omp --resume`. Preserve the binding across window
shutdown and do not guess from launch flags or select the most recent chat.

The **Save Sessions** toolbar button is available in both header layouts. It
snapshots agents descended from that GTK process, then saves that window's
state directly through the same persistence queue as `session.save`. Do not
route the button's save over a socket another window may already own.

### Browser check: GBM and a second NVIDIA GPU

`renderer.rs`. WebKit's sandbox opens `/dev/nvidia0` and `/dev/nvidiactl` only. A display on `/dev/nvidia1` or later aborts with `Could not create GBM EGL display`. The check must not make that attempt:

- display on an unbound NVIDIA GPU: start with `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1`
- otherwise try the default, and on that exact abort retry with the setting and remember it for this WebKit, driver, and GPUs
- a user-set renderer variable (`FORCE_SHM`, `WEBKIT_DMABUF_RENDERER_DISABLE_GBM`, `WEBKIT_DISABLE_DMABUF_RENDERER`, `WEBKIT_WEB_RENDER_DEVICE_FILE`) means check as configured and change nothing

`WEBKIT_DMABUF_RENDERER_DISABLE_GBM=1` passes the probe and then draws garbage. Do not substitute it.

The host calls `renderer::check`, which calls `probe::check_with(env)`. Tests call `check_with` directly. Shells Limux starts must not inherit the setting: `terminal_child_env()` returns `WEBKIT_DMABUF_RENDERER_FORCE_SHM=0` when Limux itself forced it, chained from `terminal_child_environment_overrides` in `main.rs` into pane and gui-hook env.

Upstream replaced the synchronous `availability()` with an async probe (`status_nonblocking_or_start`, `wait_for_availability_blocking`, `register_main_thread_waiter`, `check_with_cancel`). Callers on upstream's side already use that API. A sync keeps the async API and routes the probe through `renderer::check`, so cancellation and the deferred startup still exist, and the GBM/NVIDIA decision still wraps the probe. Dropping `renderer.rs` or calling `probe::check_with_cancel` from the host directly reintroduces the abort.

## Shared files, and the rule on conflict

Upstream has also edited these since `32dd2a3`. A conflict here is expected. The rule is: take upstream's new behavior, then put the fork seam back. Never resolve by taking one side of the hunk.

| File | Fork seam | Upstream, as of `cfc090c` | Keep |
|---|---|---|---|
| `rust/limux-cli/src/main.rs` | `mod card`; subcommands above | `mod session_snapshot`; `session.save` | both modules |
| `rust/limux-protocol/src/methods.rs` | four card methods; array lengths 16 / 54 / 182 | `session.save`; lengths 13 / 51 / 179 | both method sets; recount, do not trust either length |
| `rust/limux-ghostty-sys/src/lib.rs` | action sizes 48 / 56; `ghostty_action_program_status_s` | scrollbar asserts; action sizes 24 / 32 | fork sizes; add scrollbar asserts |
| `rust/limux-host-linux/src/browser_sandbox.rs` | `availability` via `renderer::check`; `terminal_child_env` | async probe module | async API, probe body goes through `renderer::check`; keep `terminal_child_env` |
| `browser_sandbox/probe.rs` | `check_with(&[(&str, &str)])` | `check_with_cancel`, `LaunchEnv` | env injection and cancel; `renderer` is the host entry |
| `pane.rs` | `on_program_status`; full automatic title and title callback; status clear on exit | exit-hold `on_close`; title dedup | both, but deduplicate and store the full title, not the tab preview |
| `terminal.rs` | shell-integration atomics; `program_status_report` | scrollbar, motion, scrollback helpers inserted in the same gap | both insertions |
| `window.rs` | card payload fields; indicators forced off; `refresh_workspace_location` from pwd | `group_id` local; indicator visibility flag; pwd change detection | fork fields and hidden indicators; upstream dedup around the refresh |
| `window/session_persistence.rs` | `named_by_user`, `description`, `group_id` | `favorites.contains` | fork fields; either favorite lookup if it is the same set |
| `layout_state.rs` | `named_by_user`, `description`, `resume_command(shell_integration)` | may touch session types nearby | fork fields and the integrated login tail |
| `control_bridge.rs` | card method dispatch | `session.save` and other RPCs | card arms stay |
| `main.rs` (host) | `terminal_child_env` in the child env chain | likely untouched | chain stays |
| `app_config.rs`, `settings_editor.rs`, `workspace_color.rs`, `window/sidebar_groups.rs` | chrome commit | may touch settings or sidebar | no indicator pills; `CHROME_BLACK` / `SELECTION_FILL`; color is a rail |
| `Cargo.lock`, `README.md` | card section under Agent integrations | version bumps, other docs | keep the Workspace cards section; take upstream's version bump only if the package build is updated to match |

Test files that only gained `on_program_status: Box::new(|_, _| {})` or `named_by_user: None` (`eager_restore_tests`, `pane_close_tests`, `session_store_tests`, `tab_move_tests`, `activity_tests`, `ssh_reconnect_tests`, `tab_overflow_tests`) must keep those fields or the new structs will not build. They are not behavior.

## Check after a sync

`fork/card-smoke.sh` is the fork's live check (isolated under Xvfb). `scripts/check.sh` is upstream's gate. Both should pass. A Ghostty rebuild is required only if the submodule pointer moved. `makepkg` rewrites `fork/PKGBUILD`'s `pkgver=`; `sync-upstream.sh` checks the committed file back out. Do that by hand if packaging outside the script.
