#!/usr/bin/env bash
# Regression: failed sends, a missing tab and a failed second window must
# be recorded without ending the guided check. No daemon or desktop needed.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture"/{scripts/manual,target/logs,target/release}
cp "${1:-$repo/scripts/manual/t43.sh}" "$fixture/scripts/manual/t43.sh"
touch "$fixture/target/release/shoal-gui"
cat > "$fixture/scripts/shoal.sh" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
case $1 in
    stop) ;;
    start)
        [[ $# == 1 ]] || exit 1
        printf 'connected session=s1\nasking for a size pane=p3 cols=80 rows=24\n' > "$repo/target/logs/window.log"
        ln -sfn window.log "$repo/target/logs/shoal-gui.log"
        ;;
    ls)
        printf '%s\n' '{"workspace":{"sessions":[{"id":"s1","active_tab":"t2","tabs":[{"id":"t2","focused":"p3","zoomed":null,"name":null,"layout":{"pane":"p3"}}]}],"panes":{"p3":{"size":{"rows":24,"cols":80}}}}}'
        ;;
    capture) printf '24 80\n' ;;
    send) exit 1 ;;
    *) exit 2 ;;
esac
FAKE
chmod +x "$fixture/scripts/shoal.sh"
"$fixture/scripts/shoal.sh" ls --json | jq -e '.workspace.sessions[0].tabs | length == 1' >/dev/null
# Pauses ignore the answer; visual questions get n and a short failure note.
printf 'n\n%.0s' {1..150} > "$fixture/answers"
if timeout 10 bash "$fixture/scripts/manual/t43.sh" < "$fixture/answers" > "$fixture/output" 2>&1; then
    echo 'FAIL: the guided check hid its failures' >&2
    exit 1
else
    status=$?
    [[ $status == 1 ]] || { cat "$fixture/output"; exit 1; }
fi
results="$fixture/target/manual/t43.txt"
grep -q '12. Done' "$fixture/output" || { tail -12 "$fixture/output" >&2; exit 1; }
grep -q '^FAIL  (auto) fullscreen:' "$results"
grep -q '^FAIL  (auto) an empty name' "$results"
grep -q '^FAIL  Bell check: second tab missing' "$results"
grep -q '^FAIL  Could not open the second window' "$results"
grep -q '^FAIL  No window for the idle check' "$results"
echo 'PASS: guided check records failed steps and reaches the end'
