#!/usr/bin/env bash
# T42 guided input checks. Own daemon, window, HOME and every XDG path.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo"
results="$repo/target/manual/t42.txt"
mkdir -p "$(dirname "$results")"
: > "$results"
record() { printf '%s\n' "$*" | tee -a "$results"; }
say() { printf '\n\e[1m%s\e[0m\n' "$*"; }
ask() {
    local answer note
    while true; do
        if ! read -r -p "$1 [y/n/s=skip] " answer; then record "FAIL  Input ended: $1"; exit 1; fi
        case $answer in
            y|Y) record "PASS  $1"; return ;;
            n|N) read -r -p 'What went wrong? ' note || note='Input ended'; record "FAIL  $1 — $note"; return ;;
            s|S) read -r -p 'Why skipped? ' note || note='Input ended'; record "SKIP  $1 — $note"; return ;;
        esac
    done
}
# Select pinned build tools before isolating HOME.
mise exec -- cargo build --locked --release -p shoal-gui -p shoal-cli -p shoal-daemon --bins --example call
record "GUI built: $(stat -c '%y' target/release/shoal-gui)"
ln -sfn ../share "$repo/target/share"
sandbox=$(mktemp -d "$repo/target/manual/t42-env.XXXXXX")
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
printf "PS1='T42> '\n" > "$sandbox/home/.bashrc"
gui_pid='' daemon_pid=''
cleanup() {
    [[ -z $gui_pid ]] || kill "$gui_pid" 2>/dev/null || true
    [[ -z $daemon_pid ]] || kill "$daemon_pid" 2>/dev/null || true
    wait 2>/dev/null || true
}
trap cleanup EXIT
ulimit -c 0
target/release/shoald > "$sandbox/daemon.log" 2>&1 & daemon_pid=$!
# Status never auto-starts a second daemon. Wait on readiness with a deadline.
deadline=$((SECONDS + 5))
until target/release/shoal daemon status >/dev/null 2>&1; do
    if ((SECONDS >= deadline)); then record 'FAIL  Daemon startup timed out'; exit 1; fi
done
target/release/shoal new -n input-check > "$sandbox/new.txt"
# Resolve the session by name rather than relying on CLI presentation.
session=$(target/release/shoal ls --json | jq -er '.workspace.sessions[] | select(.name == "input-check") | .id')
target/release/shoal-gui --session "$session" > "$sandbox/gui.log" 2>&1 & gui_pid=$!
record "Sandbox: $sandbox; session: $session"
send() { target/release/shoal send "$session" "$1" || record 'FAIL  Sending fixture input'; }
probe() {
    send "python3 '$repo/scripts/manual/t42-input.py' '$sandbox/$1.bin' '$1'\\n"
}
say '1. Click the T42 window. Type: printf "typing: é界😀\\n" and Enter.'
ask 'Typing, Backspace, arrows, Ctrl+A/C/E, Alt+B/F and Unicode reach bash correctly?'
say '2. Try the tab and pane bindings from docs/spec/m1-spec.md §10.'
say 'Ctrl+Shift+O/E split; Ctrl+Alt+Arrow focuses; add Shift to resize; Ctrl+Shift+Enter zooms.'
say 'Ctrl+Shift+T creates tabs; Ctrl+Tab / Ctrl+Shift+Tab and Ctrl+PageDown/Up cycle.'
say 'Alt+1…8 select a tab, Alt+9 selects the last; Ctrl+Shift+PageUp/Down moves tabs.'
say 'Ctrl+Shift+R renames (try Unicode, Enter and Escape); Ctrl+Shift+W closes a pane; Ctrl+Alt+W closes a tab.'
say 'Leave one tab and pane open for the remaining checks.'
ask 'All existing tab/pane bindings work and focus follows the visible layout?'
say '3. Rebind in this disposable config, keeping the window open:'
printf '  %s\n' "$sandbox/config/shoal/config.toml"
say 'Add [keybinds] then "ctrl+shift+y" = "tab.new" and try the new binding.'
ask 'The new binding takes effect without restarting?'
say '4. Return to a single pane. Run the mouse/focus probe below.'
probe mouse
say 'Move, click, drag and wheel: the program captures these. Hold Shift and double-click alpha.'
say 'Ctrl+Insert copies; Shift+Insert pastes. Middle-click pastes primary selection.'
say 'Switch tabs or windows and back; then Ctrl+C ends the probe.'
ask 'Shift overrides capture, selection highlights, copy/paste and primary paste work?'
if [[ -f $sandbox/mouse.bin ]]; then
    python3 - "$sandbox/mouse.bin" <<'PY' | tee -a "$results"
import pathlib, sys
raw = pathlib.Path(sys.argv[1]).read_bytes()
for name, needle in [('mouse reports', b'\x1b[<'), ('focus lost', b'\x1b[O'), ('focus gained', b'\x1b[I')]:
    print(('PASS' if needle in raw else 'FAIL') + '  ' + name + ' in probe bytes')
PY
else record 'FAIL  Mouse probe did not create its result'; fi
say '5. Bracketed paste probe: copy two lines, paste with Ctrl+Shift+V or Shift+Insert, then Ctrl+C.'
probe bracket
ask 'Multiline paste arrives as one paste?'
python3 - "$sandbox/bracket.bin" <<'PY' | tee -a "$results"
import pathlib, sys
p = pathlib.Path(sys.argv[1]); raw = p.read_bytes() if p.exists() else b''
print(('PASS' if b'\x1b[200~' in raw and b'\x1b[201~' in raw else 'FAIL') + '  Bracketed paste markers')
PY
say '6. Legacy paste probe: paste two lines again, then Ctrl+C.'
probe legacy
ask 'Paste without bracketed mode works?'
python3 - "$sandbox/legacy.bin" <<'PY' | tee -a "$results"
import pathlib, sys
p = pathlib.Path(sys.argv[1]); raw = p.read_bytes().removeprefix(b'READY\n') if p.exists() else b''
print(('PASS' if b'\r' in raw and b'\x1b[200~' not in raw and b'\n' not in raw else 'FAIL') + '  Legacy paste newline conversion')
PY
say '7. Try nvim, less, fzf or btop: navigation, Ctrl keys and mouse capture. Quit back to bash.'
ask 'Installed terminal applications take keyboard and mouse input correctly?'
say '8. Ctrl+Shift+, opens this sandbox config in $EDITOR (vi if unset); quit the editor.'
ask 'The config opens in a new tab?'
say '9. If you use an input method, compose text in bash and in tab rename; otherwise skip.'
ask 'Input-method composition commits text once and leaves preedit out of the shell?'
say '10. Try your Omarchy universal copy/paste keys. Its terminal tag must include dev.shoal.Shoal.'
say 'A custom Super+C binding or missing terminal tag is a desktop integration gap (T50); record skip.'
ask 'The configured universal copy/paste keys reach the Insert bindings?'
record 'T44 session/window actions, T45 history/links, T47 confirmations and T48 font/theme updates remain deferred.'
record "Results: $results"
say "Finished. Evidence remains in $sandbox; only this test's processes are stopped."
