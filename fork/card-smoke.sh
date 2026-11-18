#!/bin/bash
# Live check of the workspace-card features on the debug build, fully isolated:
# its own Xvfb display, D-Bus session, HOME/XDG folders and socket, so the
# installed Limux and its saved workspaces are never touched.
#
#   fork/card-smoke.sh [out-dir]
#
# Env: CARD_SMOKE_HOST / CARD_SMOKE_CLI to test another build (for example the
# installed /usr/libexec/limux/limux-host and /usr/bin/limux, with CARD_SMOKE_LD_PATH=),
# CARD_SMOKE_SEED_SESSION to start from a copy of a saved session.json.
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${1:-$ROOT/.tools/card-smoke}
RUN=$(mktemp -d /tmp/limux-card-smoke.XXXXXX)
log() { echo "[$(date +%T)] card-smoke: $*"; }
fail=0
check() { if eval "$2"; then echo "PASS  $1"; else echo "FAIL  $1"; fail=1; fi; }

if [ -z "${CARD_SMOKE_INSIDE:-}" ]; then
  mkdir -p "$OUT"
  # A private bus with no service activation, the way scripts/tests/visual-smoke-test.sh does it.
  cat > "$RUN/dbus.conf" <<'DBUS'
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
DBUS
  export CARD_SMOKE_INSIDE=1 CARD_SMOKE_OUT="$OUT" CARD_SMOKE_RUN="$RUN"
  export GTK_USE_PORTAL=0 GTK_A11Y=none GIO_USE_VFS=local GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1
  unset DBUS_SESSION_BUS_ADDRESS DBUS_STARTER_ADDRESS DBUS_STARTER_BUS_TYPE DISPLAY WAYLAND_DISPLAY \
    SSH_AUTH_SOCK GNOME_KEYRING_CONTROL XDG_ACTIVATION_TOKEN DESKTOP_STARTUP_ID GSK_RENDERER
  exec xvfb-run -a -s "-screen 0 1400x900x24 -nolisten tcp" \
    dbus-run-session --config-file="$RUN/dbus.conf" -- bash "$0"
fi
OUT=$CARD_SMOKE_OUT RUN=$CARD_SMOKE_RUN
export HOME="$RUN/home" XDG_DATA_HOME="$RUN/data" XDG_STATE_HOME="$RUN/state" \
  XDG_CONFIG_HOME="$RUN/config" XDG_CACHE_HOME="$RUN/cache" XDG_RUNTIME_DIR="$RUN/runtime"
mkdir -p "$HOME" "$XDG_DATA_HOME/limux" "$XDG_STATE_HOME" "$XDG_CONFIG_HOME/ghostty" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
export GDK_BACKEND=x11 GDK_SCALE=1 GTK_THEME=Adwaita:dark LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
export LIMUX_SOCKET="$RUN/limux.sock" LIMUX_SOCKET_PATH="$RUN/limux.sock" LIMUX_SOCKET_MODE=localUser
export LD_LIBRARY_PATH="${CARD_SMOKE_LD_PATH-$ROOT/ghostty/zig-out/lib}"
export GHOSTTY_RESOURCES_DIR="$ROOT/ghostty/zig-out/share/ghostty" TERMINFO="$ROOT/ghostty/zig-out/share/terminfo"
export SHELL=/bin/sh ENV=/dev/null
unset LIMUX_WORKSPACE_ID LIMUX_SURFACE_ID LIMUX_PANE_ID LIMUX_TAB_ID
printf 'command = /bin/sh\nshell-integration = none\nfont-size = 12\n' > "$XDG_CONFIG_HOME/ghostty/config"
# Optionally start from a copy of a real saved session (read-only use of the original).
if [ -n "${CARD_SMOKE_SEED_SESSION:-}" ]; then cp "$CARD_SMOKE_SEED_SESSION" "$XDG_DATA_HOME/limux/session.json"; fi

# a git repo for the branch row
REPO="$HOME/work/demo-repo"
mkdir -p "$REPO" && git -C "$REPO" init -q -b card-demo && git -C "$REPO" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init

CLI="${CARD_SMOKE_CLI:-$ROOT/target/debug/limux-cli}"
HOST="${CARD_SMOKE_HOST:-$ROOT/target/debug/limux}"
HOST_PID=""
# Start the host and remember its PID; stop only that PID, never by name, so a
# Limux the user has open is never touched.
start_host() {
  cd "$REPO" || exit 1
  "$HOST" > "$RUN/$1" 2>&1 &
  HOST_PID=$!
  cd - >/dev/null || exit 1
}
stop_host() {
  [ -n "$HOST_PID" ] || return 0
  kill -TERM "$HOST_PID" 2>/dev/null
  for _ in $(seq 1 50); do kill -0 "$HOST_PID" 2>/dev/null || break; sleep 0.2; done
  HOST_PID=""
}
cli() { "$CLI" --socket "$LIMUX_SOCKET" "$@"; }
log "start: host on $DISPLAY, run dir $RUN"
start_host host.log
for _ in $(seq 1 100); do cli identify >/dev/null 2>&1 && break; sleep 0.2; done
cli identify >/dev/null 2>&1 || { log "FAILED: host did not answer"; tail -20 "$RUN/host.log"; exit 1; }
sleep 1.5
WS=$(cli --json list-workspaces | jq -r '.workspaces[0].ref')
log "workspace: $WS"
cli new-workspace --cwd "$REPO" >/dev/null 2>&1; sleep 1
WS2=$(cli --json list-workspaces | jq -r '.workspaces[-1].ref')

shot() {
  local win
  win=$(xdotool search --onlyvisible --class limux 2>/dev/null | head -1)
  [ -n "$win" ] || win=root
  sleep 0.6
  import -window "${win}" "$OUT/$1.png" 2>/dev/null || import -window root "$OUT/$1.png"
  echo "  screenshot $OUT/$1.png"
}
shot 01-plain

cli set-status --workspace "$WS2" claude Running --icon bolt.fill --color '#4C8DFF' --priority 10 >/dev/null
cli set-status --workspace "$WS2" program "Bash: run the unit tests" --icon bolt.fill --priority 2 >/dev/null
cli set-status --workspace "$WS2" pr "PR #503 open" --icon arrow.triangle.pull --color green --priority -10 >/dev/null
cli set-description --workspace "$WS2" "PROD DEPLOYS STILL PENDING" >/dev/null
cli notify --workspace "$WS2" --body "Bedrock deploy-gate check needs your input" "Claude needs you" >/dev/null 2>&1
shot 02-card-full

J=$(cli --json list-workspaces | jq -c --arg ws "$WS2" '.workspaces[] | select(.ref == $ws)')
check "list-workspaces shows the description" "[ \"\$(jq -r .description <<<'$J')\" = 'PROD DEPLOYS STILL PENDING' ]"
check "status rows come back sorted by priority" "[ \"\$(jq -r '[.statuses[].key] | join(\",\")' <<<'$J')\" = 'claude,program,pr' ]"
check "git branch of the workspace folder is reported" "[ \"\$(jq -r .git_branch <<<'$J')\" = 'card-demo' ]"
check "list-status text output" "cli list-status --workspace '$WS2' | grep -q 'claude	Running'"

cli set-status --workspace "$WS2" claude "Needs input" --icon bell.fill --color amber --priority 10 >/dev/null
check "a key is replaced, not duplicated" "[ \"\$(cli --json list-status --workspace '$WS2' | jq '[.statuses[] | select(.key==\"claude\")] | length')\" = 1 ]"
shot 03-needs-input

check "markup in a value is shown as text, not rendered" "cli set-status --workspace '$WS2' evil '<b>bold</b> & <img src=x>' >/dev/null"
check "bad colour is refused" "! cli set-status --workspace '$WS2' k v --color nope >/dev/null 2>&1"
check "bad key is refused" "! cli set-status --workspace '$WS2' 'bad key' v >/dev/null 2>&1"
shot 04-escaped
cli clear-status --workspace "$WS2" evil >/dev/null

# the hook path: a Claude Stop event from inside the workspace sets Idle
echo '{"hook_event_name":"Stop","session_id":"smoke-1","cwd":"'"$REPO"'"}' |
  LIMUX_WORKSPACE_ID="$WS2" LIMUX_SURFACE_ID="x" LIMUX_PR_STATUS_DISABLED=1 "$CLI" --socket "$LIMUX_SOCKET" --json hooks claude stop >/dev/null 2>&1
check "Claude Stop hook sets the claude row to Idle" "[ \"\$(cli --json list-status --workspace '$WS2' | jq -r '.statuses[] | select(.key==\"claude\") | .value')\" = Idle ]"
echo '{"hook_event_name":"UserPromptSubmit","session_id":"smoke-1","prompt":"go"}' |
  LIMUX_WORKSPACE_ID="$WS2" LIMUX_SURFACE_ID="x" "$CLI" --socket "$LIMUX_SOCKET" --json hooks claude prompt-submit >/dev/null 2>&1
check "prompt submit sets Running" "[ \"\$(cli --json list-status --workspace '$WS2' | jq -r '.statuses[] | select(.key==\"claude\") | .value')\" = Running ]"
echo '{"hook_event_name":"SessionEnd","session_id":"smoke-1"}' |
  LIMUX_WORKSPACE_ID="$WS2" LIMUX_SURFACE_ID="x" "$CLI" --socket "$LIMUX_SOCKET" --json hooks claude session-end >/dev/null 2>&1
check "session end removes the claude row" "[ \"\$(cli --json list-status --workspace '$WS2' | jq '[.statuses[] | select(.key==\"claude\")] | length')\" = 0 ]"
echo '{"hook_event_name":"Stop","session_id":"smoke-2"}' |
  env -u LIMUX_WORKSPACE_ID LIMUX_SURFACE_ID="" "$CLI" --socket "$LIMUX_SOCKET" --json hooks claude stop >/dev/null 2>&1
check "a hook from outside a Limux workspace adds no row" "[ \"\$(cli --json list-status --workspace '$WS' | jq '.statuses | length')\" = 0 ]"

cli clear-status --workspace "$WS2" >/dev/null
check "clear-status with no key clears every row" "[ \"\$(cli --json list-status --workspace '$WS2' | jq '.statuses | length')\" = 0 ]"

# refresh-pr end to end with a stand-in gh that replays gh's real answers
# (the sandbox has no keyring, so the real gh would only get a 401 here)
mkdir -p "$RUN/bin"
cat > "$RUN/bin/gh" <<'GH'
#!/bin/sh
case "$FAKE_GH" in
  open) echo '{"number":503,"state":"OPEN"}' ;;
  none) echo 'no pull requests found for branch "main"' >&2; exit 1 ;;
  *) echo 'HTTP 401: Requires authentication' >&2; exit 1 ;;
esac
GH
chmod +x "$RUN/bin/gh"
FAKE_GH=open PATH="$RUN/bin:$PATH" cli refresh-pr --workspace "$WS2" --cwd "$REPO" >/dev/null
check "refresh-pr shows an open PR as the pr row" "[ \"\$(cli --json list-status --workspace '$WS2' | jq -r '.statuses[] | select(.key==\"pr\") | .value')\" = 'PR #503 open' ]"
FAKE_GH=fail PATH="$RUN/bin:$PATH" cli refresh-pr --workspace "$WS2" --cwd "$REPO" >/dev/null
check "a failed gh call leaves the pr row alone" "[ \"\$(cli --json list-status --workspace '$WS2' | jq '[.statuses[] | select(.key==\"pr\")] | length')\" = 1 ]"
FAKE_GH=none PATH="$RUN/bin:$PATH" cli refresh-pr --workspace "$WS2" --cwd "$REPO" >/dev/null
check "a branch without a PR clears the pr row" "[ \"\$(cli --json list-status --workspace '$WS2' | jq '[.statuses[] | select(.key==\"pr\")] | length')\" = 0 ]"

# the description survives a restart; status rows do not
cli set-status --workspace "$WS2" claude Running >/dev/null
sleep 2.5
stop_host
start_host host2.log
for _ in $(seq 1 100); do cli identify >/dev/null 2>&1 && break; sleep 0.2; done
sleep 1.5
J2=$(cli --json list-workspaces | jq -c '[.workspaces[] | select(.description != null)]')
check "description restored after restart" "[ \"\$(jq -r '.[0].description' <<<'$J2')\" = 'PROD DEPLOYS STILL PENDING' ]"
check "status rows are not restored" "[ \"\$(jq '.[0].statuses | length' <<<'$J2')\" = 0 ]"
shot 05-after-restart

# A restored agent tab ends in a login shell that must still report its folder,
# so cd moves the card's path and branch. Real bash with Ghostty's default shell
# integration, one workspace whose tab resumes a stand-in agent (/usr/bin/true).
stop_host
printf 'font-size = 12\n' > "$XDG_CONFIG_HOME/ghostty/config"
printf 'PS1="smoke> "\n' > "$HOME/.bashrc"
printf '[ -f ~/.bashrc ] && . ~/.bashrc\n' > "$HOME/.bash_profile"
S="$XDG_DATA_HOME/limux/session.json"
jq --arg h "$HOME" '.workspaces = [.workspaces[0]] | .workspaces[0].cwd = $h | .workspaces[0].folder_path = $h
  | .workspaces[0].layout.tabs = [.workspaces[0].layout.tabs[0] | .cwd = $h | .agent = {kind: "codex",
    session_id: "smoke-session", cwd: $h, restore_on_startup: true, launch_command: {executable: "/usr/bin/true",
    arguments: ["/usr/bin/true"], cwd: $h, environment: {}, captured_at: 1}}]
  | .workspaces[0].layout.active_tab_id = .workspaces[0].layout.tabs[0].id' "$S" > "$S.new" && mv "$S.new" "$S"
export SHELL=/usr/bin/bash
unset ENV
start_host host3.log
for _ in $(seq 1 100); do cli identify >/dev/null 2>&1 && break; sleep 0.2; done
sleep 4
cli send "cd $REPO" >/dev/null && cli send-key Enter >/dev/null
sleep 2
cli send 'echo "posix=$(set -o | awk '"'"'$1=="posix"{print $2}'"'"') prompt=[$PS1] hook=$(declare -F __ghostty_precmd)" > "$HOME/restored-shell.txt"' >/dev/null
cli send-key Enter >/dev/null
sleep 1.5
J3=$(cli --json list-workspaces | jq -c '.workspaces[0]')
check "restored agent tab: cd moves the card's folder" "[ \"\$(jq -r .cwd <<<'$J3')\" = '$REPO' ]"
check "restored agent tab: cd shows the branch" "[ \"\$(jq -r .git_branch <<<'$J3')\" = 'card-demo' ]"
check "restored agent tab: shell left POSIX mode and read its startup files" \
  "grep -qxF 'posix=off prompt=[smoke> ] hook=__ghostty_precmd' '$HOME/restored-shell.txt'"

# Like cmux, a workspace nobody named takes its focused terminal's title; bash's
# prompt sets that to the folder (\w). The path row then hides its echo.
check "unnamed workspace takes the terminal's title" "[ \"\$(jq -r .title <<<'$J3')\" = '~/work/demo-repo' ]"
shot 06-restored-agent-tab
# An agent's spinner frame is dropped, so its animated title is one name.
cli send 'printf "\033]2;\342\240\202 Smoke task\007"; sleep 4' >/dev/null && cli send-key Enter >/dev/null
sleep 1.5
check "a spinner frame is dropped from the title" \
  "[ \"\$(cli --json list-workspaces | jq -r '.workspaces[0].title')\" = 'Smoke task' ]"
sleep 3
# A name the user chose stays, and is saved as theirs.
cli rename-workspace --workspace "$(jq -r .ref <<<"$J3")" "Pinned name" >/dev/null
cli send 'cd ~' >/dev/null && cli send-key Enter >/dev/null
sleep 1.5
check "a name the user chose survives a title change" \
  "[ \"\$(cli --json list-workspaces | jq -r '.workspaces[0].title')\" = 'Pinned name' ]"
check "the session saves the name as the user's" "[ \"\$(jq -r '.workspaces[0].named_by_user' '$S')\" = true ]"
shot 07-named-workspace

stop_host
sleep 1
grep -i -E 'panic|critical|error' "$RUN"/host*.log | grep -v -i -E 'Gtk-WARNING|libEGL|MESA|dbind|portal' | head -5
log "exit: $([ $fail = 0 ] && echo 'all checks passed' || echo 'SOME CHECKS FAILED'); screenshots in $OUT"
rm -rf "$RUN"
exit $fail
