#!/usr/bin/env bash
# Exercise the playground's startup, failure reporting, and cleanup without a desktop.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture"/{scripts/manual,target/release/examples,target/manual,bin}
cp "$repo/scripts/manual/t49.sh" "$fixture/scripts/manual/t49.sh"
printf 'Previous feedback\n' > "$fixture/target/manual/t49.txt"
cat > "$fixture/bin/mise" <<'EOF'
#!/bin/sh
test "${FAIL_BUILD:-0}" != 1
EOF
cat > "$fixture/target/release/shoald" <<'EOF'
#!/bin/sh
echo $$ > "$HOME/daemon.pid"
exec python3 -c 'import signal; signal.pause()'
EOF
cat > "$fixture/target/release/shoal-gui" <<'EOF'
#!/bin/sh
test "${FAIL_GUI:-0}" != 1
EOF
cat > "$fixture/target/release/examples/call" <<'EOF'
#!/bin/sh
echo '{"pane":"p8"}'
EOF
cat > "$fixture/target/release/shoal" <<'EOF'
#!/bin/sh
case "$1" in
  daemon) test "${FAIL_DAEMON:-0}" != 1 ;;
  new|send) exit 0 ;;
  ls) echo '{"workspace":{"sessions":[{"id":"s1","name":"main","tabs":[{"id":"t2","focused":"p3"}]}]}}' ;;
  *) exit 1 ;;
esac
EOF
chmod +x "$fixture/bin/mise" "$fixture/target/release/"{shoald,shoal-gui,shoal} "$fixture/target/release/examples/call"
run_test() {
    env PATH="$fixture/bin:$PATH" XDG_RUNTIME_DIR="$fixture" "$@" timeout 15 bash "$fixture/scripts/manual/t49.sh"
}
run_test > "$fixture/success" 2>&1
rg -q 'Playground stopped' "$fixture/success"
rg -q 'Ctrl\+Shift\+S' "$fixture/success"
cmp -s "$fixture/target/manual/t49.txt" <(printf 'Previous feedback\n')
for pidfile in "$fixture"/target/manual/t49-play.*/home/daemon.pid; do
    pid=$(< "$pidfile")
    if kill -0 "$pid" 2>/dev/null; then echo 'FAIL: daemon survived window exit' >&2; exit 1; fi
done
if run_test FAIL_BUILD=1 > "$fixture/build-fail" 2>&1; then echo 'FAIL: build failure hidden' >&2; exit 1; fi
rg -q 'Build failed; see .*build.log' "$fixture/build-fail"
if run_test FAIL_DAEMON=1 > "$fixture/daemon-fail" 2>&1; then echo 'FAIL: daemon failure hidden' >&2; exit 1; fi
rg -q 'Daemon did not start; see .*daemon.log' "$fixture/daemon-fail"
if run_test FAIL_GUI=1 > "$fixture/gui-fail" 2>&1; then echo 'FAIL: window failure hidden' >&2; exit 1; fi
rg -q 'Window exited with an error; see .*gui.log' "$fixture/gui-fail"
for pidfile in "$fixture"/target/manual/t49-play.*/home/daemon.pid; do
    pid=$(< "$pidfile")
    if kill -0 "$pid" 2>/dev/null; then echo 'FAIL: daemon survived window failure' >&2; exit 1; fi
done
echo 'PASS: T49 playground preserves feedback, reports failures and cleans up'
