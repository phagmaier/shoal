#!/usr/bin/env bash
# T44 guided session checks, isolated from every real Shoal session.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo"
results="$repo/target/manual/t44.txt"
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
# Resolve pinned tools before replacing HOME for the test processes.
mise exec -- cargo build --locked --release -p shoal-gui -p shoal-cli -p shoal-daemon --bins --example call
record "GUI built: $(stat -c '%y' target/release/shoal-gui)"
ln -sfn ../share "$repo/target/share"
sandbox=$(mktemp -d "$repo/target/manual/t44-env.XXXXXX")
umask 077
mkdir -p "$sandbox"/{home,runtime,config/shoal,state,cache,data}
if [[ -n ${WAYLAND_DISPLAY:-} && $WAYLAND_DISPLAY != /* ]]; then
    export WAYLAND_DISPLAY="${XDG_RUNTIME_DIR:?}/$WAYLAND_DISPLAY"
fi
export HOME="$sandbox/home" XDG_RUNTIME_DIR="$sandbox/runtime"
export XDG_CONFIG_HOME="$sandbox/config" XDG_STATE_HOME="$sandbox/state"
export XDG_CACHE_HOME="$sandbox/cache" XDG_DATA_HOME="$sandbox/data"
export PATH="$repo/target/release:$PATH" SHOAL_LOG=debug
printf '[shell]\nprogram = "/bin/bash"\n[prompt]\nmode = "shell"\n' > "$sandbox/config/shoal/config.toml"
printf "PS1='T44> '\n" > "$sandbox/home/.bashrc"
daemon_pid=''
cleanup() {
    # window.new creates detached children. Match both the executable and
    # this disposable HOME; never close another checkout's or user's window.
    python3 - "$repo/target/release/shoal-gui" "$sandbox/home" <<'PY'
import os, pathlib, signal, sys
for proc in pathlib.Path('/proc').iterdir():
    if not proc.name.isdecimal():
        continue
    try:
        if os.readlink(proc / 'exe') != sys.argv[1]:
            continue
        if ('HOME=' + sys.argv[2]).encode() in (proc / 'environ').read_bytes().split(b'\0'):
            os.kill(int(proc.name), signal.SIGTERM)
    except (OSError, ProcessLookupError):
        pass
PY
    [[ -z $daemon_pid ]] || kill "$daemon_pid" 2>/dev/null || true
    wait 2>/dev/null || true
}
trap cleanup EXIT
ulimit -c 0
target/release/shoald > "$sandbox/daemon.log" 2>&1 & daemon_pid=$!
deadline=$((SECONDS + 5))
until target/release/shoal daemon status >/dev/null 2>&1; do
    if ((SECONDS >= deadline)); then record 'FAIL  Daemon startup timed out'; exit 1; fi
done
json() { target/release/shoal ls --json; }
for name in main backend build-tools logs; do target/release/shoal new -n "$name" >/dev/null; done
main=$(json | jq -er '.workspace.sessions[] | select(.name == "main") | .id')
build=$(json | jq -er '.workspace.sessions[] | select(.name == "build-tools") | .id')
target/release/examples/call tab.create "{\"session\":\"$build\"}" >/dev/null
target/release/shoal-gui --session "$main" > "$sandbox/gui.log" 2>&1 &
record "Sandbox: $sandbox; first session: $main"
visible() { json | jq -er --arg name "$1" '.workspace.sessions[] | select(.name == $name) | .windows == 1'; }
count() { json | jq '.workspace.sessions | length'; }
name_exists() { json | jq -e --arg name "$1" '.workspace.sessions | any(.name == $name)' >/dev/null; }

say '1. Click the T44 window, then click its session chip.'
ask 'A popover sits under the chip, with main, backend, build-tools (2 tabs), logs and New session?'
say 'Press Escape. Open it with Ctrl+Shift+S, type bdt, then Enter.'
ask 'Fuzzy filtering leaves build-tools, and Enter switches to its two tabs?'
check 'build-tools is now on screen' visible build-tools
check 'main is off screen' test "$(json | jq -r --arg s "$main" '.workspace.sessions[] | select(.id == $s) | .windows')" = 0

say '2. Open Ctrl+Shift+S, press F2, type work-界, then Escape.'
ask 'Escape cancels the rename and returns to the list?'
check 'cancelled rename kept build-tools' name_exists build-tools
say 'Press F2 again, type work-界, then Enter.'
ask 'The session chip and window title now show work-界?'
check 'Unicode session rename reached the daemon' name_exists work-界
say 'Press F2 and try renaming the selected session to backend, then Enter.'
ask 'The popover explains that backend is taken and keeps the edit open?'
say 'Press Escape twice to return to the terminal.'

say '3. Open Ctrl+Shift+S, type no-such-session, then Enter on New session.'
ask 'A new session opens in this window, with a shell ready for typing?'
check 'New session added exactly one session' test "$(count)" = 5
check 'the new session is named 2 and on screen' visible 2
say 'Press Ctrl+Alt+PageDown (wrap to main), then Ctrl+Alt+PageUp (back to 2).'
ask 'Next/previous use creation order and wrap at the ends?'
check 'previous returned to 2' visible 2

say '4. Press Ctrl+Shift+N. It should reopen main, which the cycling check most recently left off screen.'
ask 'A second window opens on main, with the original window still on 2?'
check 'smart open reused the most recent off-screen session' visible main
check 'smart open did not add another session' test "$(count)" = 5
say 'In either window open the switcher.'
ask 'The session in the other window says On screen, and Current identifies this window?'
say 'Dismiss it. Close the SECOND window with the compositor close shortcut. Return to the original window on 2.'
ask 'Closing that window left main available in the session list?'

say '5. In the original window, open the switcher and type logs. Press Ctrl+Backspace.'
ask 'It asks Close session logs? Enter / Esc?'
say 'Press Escape.'
ask 'Escape returned to the list without closing logs?'
check 'Escape kept logs' name_exists logs
say 'Press Ctrl+Backspace again, then Enter.'
ask 'logs disappears while the current window stays open?'
check 'confirmed close removed logs' test "$(count)" = 4
check 'logs is gone' test "$(json | jq '[.workspace.sessions[] | select(.name == "logs")] | length')" = 0
say 'Press Escape to dismiss. Open again; press Ctrl+Backspace on Current (2), then Enter.'
ask 'Closing the shown session closes its window, while the other three sessions survive?'
check 'three other sessions survived' test "$(count)" = 3
check 'no window remains registered' test "$(json | jq '[.workspace.sessions[].windows] | add')" = 0
record "Results: $results"
say "Finished. Evidence stays in $sandbox. Only this test's processes are stopped."
! rg -q '^FAIL ' "$results"
