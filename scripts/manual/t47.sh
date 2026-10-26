#!/usr/bin/env bash
# T47 close confirmations and exited panes. All state belongs to this guide.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo"
results="$repo/target/manual/t47.txt"
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
check() { local label=$1; shift; if "$@"; then record "PASS  (auto) $label"; else record "FAIL  (auto) $label"; fi; }
mise exec -- cargo build --locked --release -p shoal-gui -p shoal-cli -p shoal-daemon --bins --example call
record "GUI built: $(stat -c '%y' target/release/shoal-gui)"
ln -sfn ../share "$repo/target/share"
sandbox=$(mktemp -d "$repo/target/manual/t47-env.XXXXXX")
umask 077
mkdir -p "$sandbox"/{home,runtime,config/shoal,state,cache,data,bin}
wayland=${WAYLAND_DISPLAY:-wayland-1}
[[ $wayland == /* ]] || wayland="${XDG_RUNTIME_DIR:?}/$wayland"
test_env=(env HOME="$sandbox/home" XDG_RUNTIME_DIR="$sandbox/runtime"
    XDG_CONFIG_HOME="$sandbox/config" XDG_STATE_HOME="$sandbox/state"
    XDG_CACHE_HOME="$sandbox/cache" XDG_DATA_HOME="$sandbox/data"
    WAYLAND_DISPLAY="$wayland" PATH="$sandbox/bin:$repo/target/release:$PATH" SHOAL_LOG=debug)
run() { "${test_env[@]}" "$@"; }
launch() { "${test_env[@]}" "$@" & launched_pid=$!; }
printf '#!/bin/sh\nexit 1\n' > "$sandbox/bin/systemctl"
chmod +x "$sandbox/bin/systemctl"
config="$sandbox/config/shoal/config.toml"
configure() {
    printf '[shell]\nprogram = "%s"\n[prompt]\nmode = "shell"\n[behavior]\nconfirm_close_running = %s\n' "$1" "$2" > "$config"
}
configure /bin/bash true
printf "PS1='T47> '\n" > "$sandbox/home/.bashrc"
printf 'T47 unchanged\n' > "$sandbox/home/note.txt"
gui_pid='' daemon_pid=''
wait_child() {
    if ! python3 - "$1" <<'PY'
import os, select, sys
try:
    fd = os.pidfd_open(int(sys.argv[1]))
except ProcessLookupError:
    sys.exit(0)
try:
    sys.exit(0 if select.select([fd], [], [], 5)[0] else 1)
finally:
    os.close(fd)
PY
    then record "FAIL  Child $1 did not exit within five seconds"; return 1; fi
    wait "$1" 2>/dev/null || true
}
cleanup() {
    if [[ -n $gui_pid ]]; then kill "$gui_pid" 2>/dev/null || true; wait_child "$gui_pid" || true; fi
    if [[ -n $daemon_pid ]]; then kill "$daemon_pid" 2>/dev/null || true; wait_child "$daemon_pid" || true; fi
}
trap cleanup EXIT
ulimit -c 0
launch target/release/shoald > "$sandbox/daemon.log" 2>&1
daemon_pid=$launched_pid
deadline=$((SECONDS + 5))
until run target/release/shoal daemon status >/dev/null 2>&1; do
    if ((SECONDS >= deadline)); then record 'FAIL  Daemon startup timed out'; exit 1; fi
done
json() { run target/release/shoal ls --json; }
call() { run target/release/examples/call "$@"; }
session_id() { json | jq -er --arg name "$1" '.workspace.sessions[] | select(.name == $name) | .id'; }
exists() { json | jq -e --arg id "$1" '.workspace.panes | has($id)' >/dev/null; }
absent() { json | jq -e --arg id "$1" '.workspace.panes | has($id) | not' >/dev/null; }
capture_has_error() { run target/release/shoal capture "$1" | rg -q 'T47 shell startup failed'; }
program_running() { json | jq -e '.workspace.panes | any(.program == "nvim" and .status.state == "running")' >/dev/null; }
start_nvim() {
    local pane=$1 deadline=$((SECONDS + 5))
    until run target/release/shoal capture "$pane" | rg -q 'T47>'; do
        if ((SECONDS >= deadline)); then record 'FAIL  Shell prompt timed out'; exit 1; fi
    done
    run target/release/shoal send "$pane" 'nvim --clean ~/note.txt\n'
    deadline=$((SECONDS + 5))
    until json | jq -e --arg p "$pane" '.workspace.panes[$p].program == "nvim"' >/dev/null; do
        if ((SECONDS >= deadline)); then record 'FAIL  nvim metadata timed out'; exit 1; fi
    done
}
run target/release/shoal new -n close-checks >/dev/null
session=$(session_id close-checks)
pane=$(json | jq -er --arg s "$session" '.workspace.sessions[] | select(.id == $s) | .tabs[0].focused')
start_nvim "$pane"
launch target/release/shoal-gui --session "$session" > "$sandbox/gui.log" 2>&1
gui_pid=$launched_pid
record "Sandbox: $sandbox; session: $session; nvim pane: $pane"

say '1. In the test window press Ctrl+Shift+W. While the bar is open, type iUNWANTED, then press Escape.'
ask 'It asked nvim is running. Close it? Enter / Esc; Escape kept nvim open and none of that text reached its buffer?'
check 'cancel kept the pane' exists "$pane"

say '2. Press Ctrl+Shift+O to split, then Ctrl+Shift+Enter to zoom the new shell. Press Ctrl+Alt+W.'
ask 'Closing the tab asks about nvim even though it is hidden behind zoom?'
say 'Press Escape, then middle-click the tab.'
ask 'Middle-click asks the same question and Escape keeps both panes?'
say 'Press Escape if needed. The guide now focuses nvim. Press Ctrl+Shift+W, then Enter.'
call pane.focus "{\"pane\":\"$pane\"}" >/dev/null
ask 'Enter closed only nvim, leaving the other shell and the window open?'
check 'confirmed pane close removed nvim' absent "$pane"

say '3. The guide starts nvim in a new tab and disables running-program confirmation in the live config.'
created=$(call tab.create "{\"session\":\"$session\"}")
pane=$(jq -er '.pane' <<< "$created")
start_nvim "$pane"
configure /bin/bash false
say 'Allow up to one second for the live config reload, then press Ctrl+Alt+W.'
ask 'Ctrl+Alt+W closes that nvim tab immediately and returns to the shell tab?'
check 'disabled confirmation closed the running tab' absent "$pane"

say '4. Create shell-only and running sessions to check the switcher.'
run target/release/shoal new -n shell-only >/dev/null
run target/release/shoal new -n busy >/dev/null
busy=$(session_id busy)
busy_pane=$(json | jq -er --arg s "$busy" '.workspace.sessions[] | select(.id == $s) | .tabs[0].focused')
start_nvim "$busy_pane"
say 'Open Ctrl+Shift+S, type shell-only, press Ctrl+Backspace.'
ask 'Even with running-program confirmation disabled, it asks Close session shell-only? Enter / Esc?'
say 'Press Escape twice. Running-program confirmation is now enabled again.'
configure /bin/bash true
say 'Open Ctrl+Shift+S, type busy, press Ctrl+Backspace.'
ask 'The session confirmation names busy and says nvim is running?'
say 'Press Escape once (back to list), then Ctrl+Backspace and Enter to close busy. Press Escape to dismiss.'
ask 'Escape preserved busy; Enter then removed it while close-checks stayed open?'
check 'busy was closed' test "$(json | jq --arg s "$busy" '[.workspace.sessions[] | select(.id == $s)] | length')" = 0

say '5. A bad shell program will start, print an error, and exit with code 7.'
cat > "$sandbox/bin/bad-shell" <<'SHELL'
#!/bin/sh
printf 'T47 shell startup failed: deliberate fixture error\n' >&2
exit 7
SHELL
chmod +x "$sandbox/bin/bad-shell"
configure "$sandbox/bin/bad-shell" true
ask 'Ready to open the bad shell with the new config?'
created=$(call tab.create "{\"session\":\"$session\"}")
failed=$(jq -er '.pane' <<< "$created")
deadline=$((SECONDS + 5))
until json | jq -e --arg p "$failed" '.workspace.panes[$p].status.code == 7' >/dev/null; do
    if ((SECONDS >= deadline)); then record 'FAIL  Early exit status timed out'; exit 1; fi
done
ask 'The pane stays visible with the startup error, Process exited with code 7, and a Close pane button?'
check 'retained output is capturable' capture_has_error "$failed"
say 'Click Close pane on the error view.'
ask 'The error pane closes immediately, leaving the original shell tab?'
check 'error pane was closed' absent "$failed"
configure /bin/bash true

say '6. In the remaining shell type nvim --clean, then close the WINDOW with your compositor close shortcut.'
ask 'The window closed without asking about nvim?'
wait_child "$gui_pid"
gui_pid=''
check 'window close kept a running program in the daemon' program_running
record "Results: $results"
say "Finished. Evidence stays in $sandbox. Only this test's processes are stopped."
! rg -q '^FAIL ' "$results"
