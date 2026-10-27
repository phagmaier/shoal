#!/usr/bin/env bash
# Disposable T49 playground. Closing its window stops only its own daemon.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo"
umask 077
mkdir -p target/manual
sandbox=$(mktemp -d "$repo/target/manual/t49-play.XXXXXX")
log="$sandbox/build.log"
printf 'Building the playground; log: %s\n' "$log"
if ! CARGO_BUILD_JOBS=8 mise exec -- cargo build --locked --release -p shoal-gui -p shoal-cli -p shoal-daemon --bins --example call >"$log" 2>&1; then
    printf 'Build failed; see %s\n' "$log" >&2
    exit 1
fi
ln -sfn ../share "$repo/target/share"
mkdir -p "$sandbox"/{home/.config/shoal,home/.local/state/omarchy/current/theme,home/.cache,home/.local/share,runtime,bin,home/repo}
printf '#!/bin/sh\nexit 1\n' > "$sandbox/bin/systemctl"
chmod +x "$sandbox/bin/systemctl"
for current in "${XDG_STATE_HOME:-$HOME/.local/state}/omarchy/current/theme" "${XDG_CONFIG_HOME:-$HOME/.config}/omarchy/current/theme"; do
    if [[ -f $current/colors.toml ]]; then cp "$current/colors.toml" "$sandbox/home/.local/state/omarchy/current/theme/"; break; fi
done
cat > "$sandbox/home/.config/shoal/config.toml" <<'EOF'
[shell]
program = "/bin/bash"
[behavior]
close_pane_on_exit = false
EOF
printf 'cd "$HOME/repo"\n' > "$sandbox/home/.bashrc"
mkdir -p "$sandbox/home/repo"/{crates,docs,scripts}
touch "$sandbox/home/repo"/{Cargo.toml,README.md,justfile}
git -C "$sandbox/home/repo" init -q -b main
git -C "$sandbox/home/repo" -c user.name=T49 -c user.email=t49@invalid add -A
git -C "$sandbox/home/repo" -c user.name=T49 -c user.email=t49@invalid commit -qm fixture
wayland=${WAYLAND_DISPLAY:-wayland-1}
[[ $wayland == /* ]] || wayland="${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is needed for Wayland}/$wayland"
test_env=(env HOME="$sandbox/home" XDG_RUNTIME_DIR="$sandbox/runtime"
    XDG_CONFIG_HOME="$sandbox/home/.config" XDG_STATE_HOME="$sandbox/home/.local/state"
    XDG_CACHE_HOME="$sandbox/home/.cache" XDG_DATA_HOME="$sandbox/home/.local/share"
    WAYLAND_DISPLAY="$wayland" PATH="$sandbox/bin:$repo/target/release:$PATH")
run() { "${test_env[@]}" "$@"; }
call() { run target/release/examples/call "$@"; }
daemon_pid='' gui_pid=''
cleanup() {
    for child in "$gui_pid" "$daemon_pid"; do
        if [[ -n $child ]]; then
            kill "$child" 2>/dev/null || true
            if ! python3 - "$child" <<'PY'
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
            then
                printf 'Child %s did not stop; forcing cleanup.\n' "$child" >&2
                kill -KILL "$child" 2>/dev/null || true
            fi
            wait "$child" 2>/dev/null || true
        fi
    done
}
trap cleanup EXIT
trap 'exit 130' INT TERM
ulimit -c 0
"${test_env[@]}" target/release/shoald >"$sandbox/daemon.log" 2>&1 & daemon_pid=$!
deadline=$((SECONDS + 5))
until run target/release/shoal daemon status >/dev/null 2>&1; do
    if ! kill -0 "$daemon_pid" 2>/dev/null || ((SECONDS >= deadline)); then
        printf 'Daemon did not start; see %s\n' "$sandbox/daemon.log" >&2
        exit 1
    fi
done
run target/release/shoal new -n main --cwd "$sandbox/home/repo" >/dev/null
run target/release/shoal new -n work --cwd "$sandbox/home/repo" >/dev/null
workspace=$(run target/release/shoal ls --json)
session=$(jq -er '.workspace.sessions[] | select(.name == "main") | .id' <<< "$workspace")
left=$(jq -er --arg s "$session" '.workspace.sessions[] | select(.id == $s) | .tabs[0].focused' <<< "$workspace")
right=$(call pane.split "{\"pane\":\"$left\",\"direction\":\"right\"}" | jq -er .pane)
call pane.split "{\"pane\":\"$right\",\"direction\":\"down\"}" >/dev/null
call tab.create "{\"session\":\"$session\",\"cwd\":\"$sandbox/home/repo\"}" >/dev/null
tab=$(jq -er --arg s "$session" '.workspace.sessions[] | select(.id == $s) | .tabs[0].id' <<< "$workspace")
call tab.activate "{\"tab\":\"$tab\"}" >/dev/null
call pane.focus "{\"pane\":\"$left\"}" >/dev/null
run target/release/shoal send "$left" 'ls\n' >/dev/null
"${test_env[@]}" target/release/shoal-gui --session "$session" >"$sandbox/gui.log" 2>&1 & gui_pid=$!
printf '\nExplore the Shoal window. This is a disposable session.\n\n'
printf '  Ctrl+Shift+S      Sessions: filter, Enter to switch, F2 to rename\n'
printf '  Ctrl+Shift+Enter  Zoom or restore the focused pane\n'
printf '  Ctrl+Shift+W      Close the focused pane\n'
printf '  Ctrl+Shift+O / E  Split right / down (or use the top-right buttons)\n'
printf '  Shift+PageUp      Scroll back\n\n'
printf 'Close the window or press Ctrl+C here to stop the playground.\n'
printf 'Logs and disposable files: %s\n' "$sandbox"
printf 'Share any observations in your own words; partial or mixed feedback is welcome.\n'
if wait "$gui_pid"; then
    gui_pid=''
    printf 'Playground stopped.\n'
else
    gui_pid=''
    printf 'Window exited with an error; see %s/gui.log\n' "$sandbox" >&2
    exit 1
fi
