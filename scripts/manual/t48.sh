#!/usr/bin/env bash
# T48 live appearance. Uses the real Omarchy switcher in headless mode, isolated.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo"
results="$repo/target/manual/t48.txt"
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
sandbox=$(mktemp -d "$repo/target/manual/t48-env.XXXXXX")
umask 077
mkdir -p "$sandbox"/{home/.config/shoal,home/.local/state,home/.cache,home/.local/share,runtime,bin}
wayland=${WAYLAND_DISPLAY:-wayland-1}
[[ $wayland == /* ]] || wayland="${XDG_RUNTIME_DIR:?}/$wayland"
test_env=(env HOME="$sandbox/home" XDG_RUNTIME_DIR="$sandbox/runtime"
    XDG_CONFIG_HOME="$sandbox/home/.config" XDG_STATE_HOME="$sandbox/home/.local/state"
    XDG_CACHE_HOME="$sandbox/home/.cache" XDG_DATA_HOME="$sandbox/home/.local/share"
    WAYLAND_DISPLAY="$wayland" PATH="$sandbox/bin:$repo/target/release:$PATH" SHOAL_LOG=debug)
run() { "${test_env[@]}" "$@"; }
launch() { "${test_env[@]}" "$@" & launched_pid=$!; }
printf '#!/bin/sh\nexit 1\n' > "$sandbox/bin/systemctl"
chmod +x "$sandbox/bin/systemctl"
printf "PS1='T48> '\n" > "$sandbox/home/.bashrc"
config="$sandbox/home/.config/shoal/config.toml"
configure() {
    cat > "$config" <<EOF
[shell]
program = "/bin/bash"
[prompt]
mode = "shell"
[font]
family = "$1"
size = $2
line_height = $3
[window]
opacity = $4
padding = $5
gap = $6
radius = $7
[cursor]
style = "$8"
color = "$9"
EOF
}
configure 'JetBrains Mono' 11 1.0 0.88 8 6 10 block accent
for theme in t48-dark t48-light; do
    mkdir -p "$sandbox/home/.config/omarchy/themes/$theme"
done
cat > "$sandbox/home/.config/omarchy/themes/t48-dark/colors.toml" <<'EOF'
background = "#102030"
foreground = "#d0e0f0"
accent = "#30c0d0"
red = "#ff5555"
blue = "#5588ff"
EOF
cat > "$sandbox/home/.config/omarchy/themes/t48-light/colors.toml" <<'EOF'
background = "#e8e0d0"
foreground = "#302010"
accent = "#b04060"
red = "#aa2222"
blue = "#2244aa"
EOF
theme_set() {
    run env OMARCHY_PATH=/usr/share/omarchy OMARCHY_THEME_HEADLESS=1 \
        OMARCHY_THEME_SKIP_BACKGROUND=1 omarchy-theme-set "$1"
}
theme_set t48-dark
gui_pid='' other_pid='' daemon_pid=''
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
    for child in "$gui_pid" "$other_pid" "$daemon_pid"; do
        if [[ -n $child ]]; then kill "$child" 2>/dev/null || true; wait_child "$child" || true; fi
    done
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
run target/release/shoal new -n t48 >/dev/null
session=$(json | jq -er '.workspace.sessions[] | select(.name == "t48") | .id')
pane=$(json | jq -er --arg s "$session" '.workspace.sessions[] | select(.id == $s) | .tabs[0].focused')
send() { run target/release/shoal send "$pane" "$1"; }
sample() {
    local tag=$1 deadline=$((SECONDS + 5)) actual model
    send "stty size > \"\$HOME/stty-$tag\"\\n"
    until [[ -s $sandbox/home/stty-$tag ]]; do
        if ((SECONDS >= deadline)); then record 'FAIL  stty probe timed out'; return 1; fi
    done
    read -r actual < "$sandbox/home/stty-$tag"
    model=$(json | jq -r --arg p "$pane" '.workspace.panes[$p].size | "\(.rows) \(.cols)"')
    record "SIZE  $tag stty=$actual daemon=$model"
    check "$tag PTY size matches daemon" test "$actual" = "$model"
}
launch target/release/shoal-gui --session "$session" > "$sandbox/gui.log" 2>&1
gui_pid=$launched_pid
record "Sandbox: $sandbox; session: $session; pane: $pane; GUI: $gui_pid"
say '1. Focus the T48 window. Keep its window size fixed through steps 1–5.'
ask 'The window shows the T48> prompt in the dark blue theme?'
sample baseline
send 'printf "\033[31mANSI red\033[0m default text \033[38;2;1;150;80mfixed RGB\033[0m\\n"\n'
say '2. Press Ctrl+= three times in the T48 window.'
ask 'The text grew and remains crisp, including already printed text?'
sample zoomed
read -r before_rows before_cols < "$sandbox/home/stty-baseline"
read -r after_rows after_cols < "$sandbox/home/stty-zoomed"
check 'Zoom reduces PTY rows and columns' test "$after_rows" -lt "$before_rows" -a "$after_cols" -lt "$before_cols"
say '3. Press Ctrl+- once, then Ctrl+0.'
ask 'Decrease reduced the text; reset returned to its original size?'
sample reset
check 'Reset restores original grid' cmp -s "$sandbox/home/stty-baseline" "$sandbox/home/stty-reset"
say '4. Font family and size now reload to monospace, 14 pt. Leave the window size fixed.'
configure monospace 14 1.0 0.88 8 6 10 block accent
ask 'Text changed size/family without restarting or losing output?'
sample font
say '5. Only line_height now changes from 1.0 to 1.5.'
configure monospace 14 1.5 0.88 8 6 10 block accent
ask 'Rows became taller; existing text, cursor and selection positions agree?'
sample line-height
read -r before_rows before_cols < "$sandbox/home/stty-font"
read -r after_rows after_cols < "$sandbox/home/stty-line-height"
check 'Line height reduces rows and keeps columns' test "$after_rows" -lt "$before_rows" -a "$after_cols" -eq "$before_cols"
say '6. Window settings now reload: opaque, 24 px padding, 20 px gap, square corners.'
say 'Focus each pane in turn to inspect its cursor; an unfocused pane uses a hollow block.'
configure monospace 14 1.0 1.0 24 20 0 bar '#ff00ff'
call pane.split "{\"pane\":\"$pane\",\"direction\":\"right\"}" >/dev/null
ask 'The two panes show the new padding/gap, opaque fill, square corners and magenta bar cursors?'
say '7. The original pane requests an underline cursor; the configured default changes to block.'
say 'Focus each pane in turn, then return focus to the original pane.'
send 'printf "\033[4 q"\n'
configure monospace 14 1.0 1.0 24 20 0 block '#00aa55'
ask 'The original pane kept its underline; the other pane changed to a green block?'
send 'printf "\033[0 q"\n'
ask 'The original pane returned to the configured block cursor?'
say '8. Theme switch through the installed omarchy-theme-set (headless, isolated HOME). Watch now.'
theme_set t48-light
ask 'Within one second the chrome, existing default/ANSI text and pane fills recolored, while fixed RGB stayed green?'
say '9. Create another tab, then return to the first. Scroll up or select some text in it.'
ask 'Ready to test a theme switch with a retained hidden tab and local scroll/selection?'
theme_set t48-dark
ask 'Recolor preserved local state, and switching tabs shows the new theme without missing output?'
say '10. Press Ctrl+= twice in the first window. A second window opens on a separate session.'
ask 'The first window is zoomed?'
run target/release/shoal new -n t48-other >/dev/null
other=$(json | jq -er '.workspace.sessions[] | select(.name == "t48-other") | .id')
launch target/release/shoal-gui --session "$other" > "$sandbox/other.log" 2>&1
other_pid=$launched_pid
ask 'The second window uses the configured 14 pt size, independently of the first?'
configure monospace 16 1.0 1.0 24 20 0 block '#00aa55'
ask 'Both windows grew; the first kept its two-point zoom offset?'
say '11. In the first window press Ctrl+0, then Ctrl+Shift+R to rename its tab.'
say 'Press Ctrl+= while the rename field is focused, then Escape.'
ask 'Reset used the current 16 pt config, and Ctrl+= in the rename field did not zoom the terminal?'
say '12. An invalid font edit now tests last-good config retention.'
printf '[font]\nsize = -1\n' > "$config"
ask 'An error banner appeared and the last good font/theme/window settings stayed in effect?'
record "Logs: $sandbox/gui.log and $sandbox/daemon.log"
if rg -q '^FAIL' "$results"; then say "Failures recorded in $results"; exit 1; fi
say "All checks passed. Results: $results"
