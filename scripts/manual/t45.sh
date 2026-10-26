#!/usr/bin/env bash
# T45 guided scrollback, selection and link checks in a disposable environment.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo"
results="$repo/target/manual/t45.txt"
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
sandbox=$(mktemp -d "$repo/target/manual/t45-env.XXXXXX")
umask 077
mkdir -p "$sandbox"/{home,runtime,config/shoal,state,cache,data,bin}
wayland=${WAYLAND_DISPLAY:-wayland-1}
[[ $wayland == /* ]] || wayland="${XDG_RUNTIME_DIR:?}/$wayland"
run() {
    env HOME="$sandbox/home" XDG_RUNTIME_DIR="$sandbox/runtime" \
        XDG_CONFIG_HOME="$sandbox/config" XDG_STATE_HOME="$sandbox/state" \
        XDG_CACHE_HOME="$sandbox/cache" XDG_DATA_HOME="$sandbox/data" \
        WAYLAND_DISPLAY="$wayland" PATH="$sandbox/bin:$repo/target/release:$PATH" SHOAL_LOG=debug "$@"
}
printf '[shell]\nprogram = "/bin/bash"\n[prompt]\nmode = "shell"\n' > "$sandbox/config/shoal/config.toml"
printf "PS1='T45> '\n" > "$sandbox/home/.bashrc"
# Record the exact opener argument without opening a browser during this check.
cat > "$sandbox/bin/xdg-open" <<OPEN
#!/usr/bin/env bash
printf '%s\\n' "\$1" >> '$sandbox/opened.txt'
OPEN
chmod +x "$sandbox/bin/xdg-open"
cat > "$sandbox/output.py" <<'PYTHON'
import pathlib, sys
fifo = pathlib.Path(sys.argv[1])
with fifo.open() as commands:
    for line in commands:
        if line.strip() == 'history':
            for i in range(300): print(f'row-{i:03d} alpha café 界', flush=True)
        else:
            print(line.rstrip(), flush=True)
PYTHON
cat > "$sandbox/links.py" <<'PYTHON'
print('\x1b]8;;https://example.org/osc8\x1b\\OSC-link\x1b]8;;\x1b\\')
print('plain: (https://example.org/a_(b)),')
PYTHON
mkfifo "$sandbox/output.fifo"
daemon_pid='' gui_pid='' writer_pid=''
cleanup() {
    [[ -z $writer_pid ]] || kill "$writer_pid" 2>/dev/null || true
    [[ -z $gui_pid ]] || kill "$gui_pid" 2>/dev/null || true
    [[ -z $daemon_pid ]] || kill "$daemon_pid" 2>/dev/null || true
    wait 2>/dev/null || true
}
trap cleanup EXIT
ulimit -c 0
run target/release/shoald > "$sandbox/daemon.log" 2>&1 & daemon_pid=$!
deadline=$((SECONDS + 5))
until run target/release/shoal daemon status >/dev/null 2>&1; do
    if ((SECONDS >= deadline)); then record 'FAIL  Daemon startup timed out'; exit 1; fi
done
run target/release/shoal new -n scroll-check >/dev/null
session=$(run target/release/shoal ls --json | jq -er '.workspace.sessions[] | select(.name == "scroll-check") | .id')
run target/release/shoal-gui --session "$session" > "$sandbox/gui.log" 2>&1 & gui_pid=$!
record "Sandbox: $sandbox; session: $session"
send() { run target/release/shoal send "$session" "$1"; }
send "python3 '$sandbox/output.py' '$sandbox/output.fifo'\\n"
# Opening the FIFO is a bounded readiness handshake; retain the writer.
coproc OUTPUT_WRITER { bash -c 'exec 3>"$1"; printf "ready\n"; cat >&3' _ "$sandbox/output.fifo"; }
writer_pid=$OUTPUT_WRITER_PID
writer_fd=${OUTPUT_WRITER[1]}
if ! read -r -t 5 ready <&"${OUTPUT_WRITER[0]}"; then
    record 'FAIL  Output fixture did not start'; exit 1
fi
printf 'history\n' >&"$writer_fd"
say '1. Click the test window. Wheel scrolls three rows per notch; use a touchpad for fractional motion.'
say 'Shift+PageUp/PageDown pages; Shift+Home/End goes to either end.'
ask 'Scrolling is smooth, the extra partial row draws correctly, and all four shortcuts work?'
say '2. Stay scrolled up and double-click alpha or café to select it.'
ask 'The word is highlighted?'
printf 'new-output-marker\n' >&"$writer_fd"
ask 'The view and selection stayed on the same text, and a new output pill appeared at bottom right?'
say 'Ctrl+Insert copies. Middle-click pastes primary selection; Ctrl+Shift+V pastes clipboard.'
say 'Click the pill to return to the bottom. Scroll up again before each paste or typed character.'
ask 'Copy and primary selection contain the selected Unicode text, and typing/paste/pill return to the bottom?'
say 'Press Ctrl+C to end the output fixture and return to bash.'
ask 'A T45 prompt is ready?'
exec {writer_fd}>&-
send "echo prompt-one; seq 1 80\\n"
ask 'The numbered output and a fresh prompt appeared?'
send "echo prompt-two; seq 81 160\\n"
say '3. Press Ctrl+Shift+Up twice, then Ctrl+Shift+Down twice.'
ask 'Prompt jumps follow the shell prompts in both directions?'
say '4. Shift+End, then wait for the two link labels below.'
send "python3 '$sandbox/links.py'\\n"
say 'Hold Ctrl over each link row, then Ctrl+click OSC-link and the plain URL.'
ask 'Links underline only while Ctrl is held, and both Ctrl+clicks reach the opener?'
python3 - "$sandbox/opened.txt" <<'PYTHON' | tee -a "$results"
import pathlib, sys
p = pathlib.Path(sys.argv[1])
opened = p.read_text().splitlines() if p.exists() else []
for uri in ['https://example.org/osc8', 'https://example.org/a_(b)']:
    print(('PASS' if uri in opened else 'FAIL') + '  Opener received ' + uri)
PYTHON
record "Results: $results"
say "Finished. Evidence stays in $sandbox. Only this test's processes are stopped."
! rg -q '^FAIL ' "$results"
