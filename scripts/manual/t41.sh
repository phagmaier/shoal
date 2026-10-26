#!/usr/bin/env bash
# T41 manual checks (terminal rendering), guided. Opens a window, drives it
# with `shoal send`, asks what you see, measures the 200 × 60 full redraw from
# the window's log, and saves everything to target/manual/t41.txt.
#   scripts/manual/t41.sh
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
shoal="$repo/scripts/shoal.sh"
results="$repo/target/manual/t41.txt"
mkdir -p "$(dirname "$results")"
: >"$results"

say() { printf '\n\e[1m%s\e[0m\n' "$*"; }
record() { printf '%s\n' "$*" | tee -a "$results"; }
ask() {
    local answer note=""
    while true; do
        read -r -p "$1 [y/n] " answer
        case $answer in
        y | Y) record "PASS  $1"; return 0 ;;
        n | N)
            read -r -p "  What looked wrong? " note
            record "FAIL  $1 — $note"
            return 0
            ;;
        esac
    done
}
pause() { read -r -p "${1:-Press Enter to continue.} " _; }

say "1. Opening a window"
"$shoal" start
log="$repo/target/logs/shoal-gui.log"
# The window's session: the one a window shows, and its focused pane.
for _ in $(seq 50); do
    state=$("$shoal" ls --json 2>/dev/null || true)
    session=$(jq -r '[.workspace.sessions[] | select(.windows > 0)][0].name // empty' <<<"$state")
    [[ -n $session ]] && break
    sleep 0.2
done
[[ -n ${session:-} ]] || { echo "No window showed up; see $log"; exit 1; }
pane=$(jq -r --arg s "$session" '.workspace.sessions[] | select(.name == $s) | . as $x
    | .tabs[] | select(.id == $x.active_tab) | .focused' <<<"$state")
record "session $session, pane $pane"
send() { "$shoal" send "$session" "$1" >/dev/null; }

say "2. Make the window fullscreen: click it, then press Super+F."
pause "Press Enter when it is fullscreen."
"$repo/target/release/examples/set-size" "$pane" 200 60
sleep 1

say "3. btop"
send 'clear; btop\n'
sleep 3
ask "btop: rounded box corners join, graphs (dot patterns) and colors look right?"
send 'q'
sleep 1

say "4. nvim"
send "clear; nvim --clean $repo/README.md\\n"
sleep 2
ask "nvim: text, colors and the status line at the bottom look right?"
send 'i'
sleep 1
ask "nvim insert mode: the cursor is a thin bar?"
send $'\e:q!\n'
sleep 1

if command -v htop >/dev/null; then
    say "5. htop"
    send 'clear; htop\n'
    sleep 3
    ask "htop: meters, colors and the process list look right?"
    send 'q'
    sleep 1
else
    record "SKIP  htop isn't installed (btop covers full-screen programs)"
fi

say "6. Full redraw timing at 200 × 60"
start=$(wc -l <"$log")
# Each step makes frames that rebuild every row: btop's first screen, then
# a fast scroll.
send 'clear; btop\n'
sleep 3
send 'q'
sleep 1
send 'clear; ls -R /usr | head -5000; clear\n'
sleep 4
times=$(tail -n "+$((start + 1))" "$log" | grep 'full redraw cols=200 rows=60' |
    sed 's/.*ms=//' | sort -n || true)
count=$(grep -c . <<<"$times" || true)
if [[ $count -gt 0 ]]; then
    # Nearest-rank percentiles over the sorted times.
    stats=$(awk '
        function rank(q,  i) { i = int(q * NR + 0.999); return a[i < 1 ? 1 : i] }
        { a[NR] = $1; if ($1 >= 4) over++ }
        END { printf "frames=%d p50=%.2f p95=%.2f max=%.2f ms, %d over 4 ms", NR, rank(0.5), rank(0.95), a[NR], over }' <<<"$times")
    record "TIME  $stats"
else
    record "TIME  no full redraws of 200 × 60 in the log; was the pane resized?"
fi

say "7. Done"
read -r -p "Anything else you noticed? (Enter to skip) " note
[[ -n $note ]] && record "NOTE  $note"
read -r -p "Stop Shoal now? [y/n] " answer
[[ $answer == [yY] ]] && "$shoal" stop
echo
echo "Saved to $results. Tell the agent it's done."
