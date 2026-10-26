#!/usr/bin/env bash
# T46 recovery and banners. Every daemon, socket, config and HOME is disposable.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo"
results="$repo/target/manual/t46.txt"
mkdir -p "$(dirname "$results")"
: > "$results"
record() { printf '%s\n' "$*" | tee -a "$results"; }
say() { printf '\n\e[1m%s\e[0m\n' "$*"; }
ask() {
    local answer note
    while true; do
        if ! read -r -p "$1 [y/n] " answer; then record "FAIL  Input ended: $1"; exit 1; fi
        case $answer in
            y|Y) record "PASS  $1"; return ;;
            n|N) read -r -p 'What went wrong? ' note || note='Input ended'; record "FAIL  $1 — $note"; return ;;
        esac
    done
}
mise exec -- cargo build --locked --release -p shoal-gui -p shoal-cli -p shoal-daemon --bins --example call
record "GUI built: $(stat -c '%y' target/release/shoal-gui)"
ln -sfn ../share "$repo/target/share"
sandbox=$(mktemp -d "$repo/target/manual/t46-env.XXXXXX")
umask 077
mkdir -p "$sandbox"/{home,runtime,upstream,config/shoal,state,cache,data,bin}
wayland=${WAYLAND_DISPLAY:-wayland-1}
[[ $wayland == /* ]] || wayland="${XDG_RUNTIME_DIR:?}/$wayland"
test_env=(env HOME="$sandbox/home" XDG_RUNTIME_DIR="$sandbox/runtime"
    XDG_CONFIG_HOME="$sandbox/config" XDG_STATE_HOME="$sandbox/state"
    XDG_CACHE_HOME="$sandbox/cache" XDG_DATA_HOME="$sandbox/data"
    WAYLAND_DISPLAY="$wayland" PATH="$sandbox/bin:$repo/target/release:$PATH" SHOAL_LOG=debug)
run() { "${test_env[@]}" "$@"; }
launch() { "${test_env[@]}" "$@" & launched_pid=$!; }
# No test ever reaches the user's systemd service.
cat > "$sandbox/bin/systemctl" <<'SYSTEMCTL'
#!/usr/bin/env bash
exit 1
SYSTEMCTL
chmod +x "$sandbox/bin/systemctl"
config="$sandbox/config/shoal/config.toml"
good_config() { printf '[shell]\nprogram = "/bin/bash"\n[prompt]\nmode = "shell"\n[keybinds]\n"ctrl+shift+y" = "tab.new"\n' > "$config"; }
good_config
printf "PS1='T46> '\n" > "$sandbox/home/.bashrc"
gui_pid='' daemon_pid='' peer_pid=''
wait_child() {
    if ! python3 - "$1" <<'PYTHON'
import os, select, sys
try:
    fd = os.pidfd_open(int(sys.argv[1]))
except ProcessLookupError:
    sys.exit(0)
try:
    sys.exit(0 if select.select([fd], [], [], 5)[0] else 1)
finally:
    os.close(fd)
PYTHON
    then record "FAIL  Child $1 did not exit within five seconds"; return 1; fi
    wait "$1" 2>/dev/null || true
}
close_gui() { if [[ -n $gui_pid ]]; then kill "$gui_pid" 2>/dev/null || true; wait_child "$gui_pid"; gui_pid=''; fi; }
cleanup() {
    close_gui || true
    [[ -z $peer_pid ]] || kill -USR1 "$peer_pid" 2>/dev/null || true
    # The restart button can spawn a detached daemon. Match its actual
    # executable and sandbox HOME before signaling; never use old PIDs.
    python3 - "$repo/target/release/shoald" "$repo/target/release/shoal-gui" "$sandbox/home" <<'PYTHON'
import os, pathlib, signal, sys
for proc in pathlib.Path('/proc').iterdir():
    if not proc.name.isdecimal(): continue
    try:
        if os.readlink(proc / 'exe') in sys.argv[1:3] and ('HOME=' + sys.argv[3]).encode() in (proc / 'environ').read_bytes().split(b'\0'):
            os.kill(int(proc.name), signal.SIGTERM)
    except OSError: pass
PYTHON
    for child in $(jobs -pr); do wait_child "$child" || true; done
}
trap cleanup EXIT
ulimit -c 0
ready() {
    local deadline=$((SECONDS + 5))
    until run target/release/shoal daemon status >/dev/null 2>&1; do
        if ((SECONDS >= deadline)); then record 'FAIL  Daemon startup timed out'; exit 1; fi
    done
}
peer_ready() {
    local mode=$1 deadline=$((SECONDS + 5))
    until run python3 scripts/manual/t46-peer.py probe "$mode" "$sandbox/runtime/shoal/shoald.sock" "${peer_pid:-0}"; do
        if ((SECONDS >= deadline)); then record "FAIL  $mode handshake timed out"; exit 1; fi
    done
}
start() { launch target/release/shoald >> "$sandbox/daemon.log" 2>&1; daemon_pid=$launched_pid; ready; }
start
run target/release/shoal new -n recovery >/dev/null
session=$(run target/release/shoal ls --json | jq -er '.workspace.sessions[] | select(.name == "recovery") | .id')
run target/release/shoal send "$session" 'echo saved-before-crash\n'
run target/release/shoal daemon persist >/dev/null
open_gui() { launch target/release/shoal-gui --session "$session" >> "$sandbox/gui.log" 2>&1; gui_pid=$launched_pid; }
open_gui
record "Sandbox: $sandbox; session: $session"
ask 'The test window shows saved-before-crash and a T46 prompt?'
# This is a crash fixture, never a restart/recovery mechanism.
kill -KILL "$daemon_pid"
wait_child "$daemon_pid"
daemon_pid=''
ask 'The window shows Reconnecting to shoald… with no old terminal text?'
start # The guide plays the manager; the window must not auto-start.
ask 'The same window restored the pane and saved marker and accepts typing?'
run target/release/shoal daemon stop >/dev/null
ask 'After an intentional stop, Reconnecting remains until a daemon is started?'
if [[ ! -S $sandbox/runtime/shoal/shoald.sock ]]; then record 'PASS  Stopped daemon stayed stopped'; else record 'FAIL  Daemon started without a request'; fi
start
ask 'The same window reconnected after the guide started the daemon?'
printf '[shell]\nprogram = 42\n' > "$config"
ask 'A one-line banner names config.toml:2 and its reason, while the last good bindings still work?'
say 'Dismiss that banner, then create a tab with the custom Ctrl+Shift+Y binding to check the retained config.'
ask 'Dismiss hides the banner and the terminal still works?'
good_config
close_gui
run target/release/shoal daemon stop >/dev/null

for mode in core version protocol stubborn; do
    say "Mismatch fixture: $mode. Nothing restarts before you click."
    if [[ $mode == core || $mode == version ]]; then
        launch env XDG_RUNTIME_DIR="$sandbox/upstream" target/release/shoald >> "$sandbox/daemon.log" 2>&1; daemon_pid=$launched_pid
        deadline=$((SECONDS + 5))
        until run env XDG_RUNTIME_DIR="$sandbox/upstream" target/release/shoal daemon status >/dev/null 2>&1; do
            if ((SECONDS >= deadline)); then record 'FAIL  Upstream startup timed out'; exit 1; fi
        done
        launch python3 scripts/manual/t46-peer.py "$mode" "$sandbox/runtime/shoal/shoald.sock" "$sandbox/upstream/shoal/shoald.sock" >> "$sandbox/peer.log" 2>&1; peer_pid=$launched_pid
    else
        launch python3 scripts/manual/t46-peer.py "$mode" "$sandbox/runtime/shoal/shoald.sock" >> "$sandbox/peer.log" 2>&1; peer_pid=$launched_pid
    fi
    peer_ready "$mode"
    open_gui
    if [[ $mode == version ]]; then
        ask 'Version banner explains running programs will end, offers Restart daemon, and panes still show?'
    else
        ask 'Build banner explains running programs will end, offers Restart daemon, and panes stay blank?'
    fi
    say 'Click Restart daemon.'
    if [[ $mode == stubborn ]]; then
        ask 'After five seconds it says shoald did not stop, asks for shoal daemon stop, and stays there?'
        kill -USR1 "$peer_pid"
        wait_child "$peer_pid"
        peer_pid=''
    else
        ask 'Restart restored panes in the same window and removed the build banner?'
        peer_ready compatible
        wait_child "$peer_pid"
        peer_pid=''
        run target/release/shoal daemon stop >/dev/null
    fi
    close_gui
done
record "Results: $results"
say "Finished. Evidence stays in $sandbox. Only this test's processes are stopped."
! rg -q '^FAIL ' "$results"
