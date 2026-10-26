#!/usr/bin/env bash
# T41 manual checks (terminal rendering), guided. Opens its own window, drives
# it with `shoal send`, asks what you see, measures the 200 × 60 full redraw
# from that window's log, and saves everything to target/manual/t41.txt.
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
# Wait up to 10 s for a line matching $2 in file $1; print the first match.
wait_for() {
    local line
    for _ in $(seq 50); do
        line=$(grep -m1 -o "$2" "$1" 2>/dev/null || true)
        [[ -n $line ]] && { echo "$line"; return 0; }
        sleep 0.2
    done
    return 1
}

say "1. Opening a window"
"$shoal" start
# This window's own log and session, even if other windows are open.
log="$repo/target/logs/$(readlink "$repo/target/logs/shoal-gui.log")"
session=$(wait_for "$log" 'connected session=s[0-9]*' | sed 's/.*=//') ||
    { echo "The window didn't connect; see $log"; exit 1; }
pane=$("$shoal" ls --json | jq -r --arg s "$session" '.workspace.sessions[]
    | select(.id == $s) | . as $x | .tabs[] | select(.id == $x.active_tab) | .focused')
record "session $session, pane $pane"
send() { "$shoal" send "$session" "$1" >/dev/null; }

say "2. Make the new window fullscreen: click it, then press Super+F."
pause "Press Enter when it is fullscreen."
# Until T43 the window doesn't size its panes, so size this one to fit: the
# pane area is the window less the 34 px top bar, 8 px padding and 6 px inset
# on each side, divided into cells of the size the window logged.
cell=$(wait_for "$log" 'width: [0-9.]*, height: [0-9.]*') || cell=""
cell_w=$(sed -n 's/width: \([0-9.]*\),.*/\1/p' <<<"$cell")
cell_h=$(sed -n 's/.*height: \([0-9.]*\)/\1/p' <<<"$cell")
export HYPRLAND_INSTANCE_SIGNATURE=${HYPRLAND_INSTANCE_SIGNATURE:-$(ls "$XDG_RUNTIME_DIR/hypr/" | head -1)}
gui=$(pgrep -n -f "^$repo/target/release/shoal-gui" || true)
read -r win_w win_h < <(hyprctl clients -j |
    jq -r --argjson pid "${gui:-0}" '.[] | select(.pid == $pid) | "\(.size[0]) \(.size[1])"') || true
if [[ -n ${win_w:-} && -n $cell_w ]]; then
    cols=$(awk -v w="$win_w" -v c="$cell_w" 'BEGIN { print int((w - 28) / c) }')
    rows=$(awk -v h="$win_h" -v c="$cell_h" 'BEGIN { print int((h - 34 - 28) / c) }')
else
    cols=120 rows=40
fi
"$repo/target/release/examples/set-size" "$pane" "$cols" "$rows" >/dev/null
record "visual checks at ${cols} × ${rows} (window ${win_w:-?} × ${win_h:-?})"
sleep 1

say "3. btop"
send 'clear; btop\n'
sleep 3
ask "btop: whole screen visible, box corners join, graphs (dot patterns) and colors look right?"
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
echo "The pane grows to 200 × 60. On a 1080p screen its bottom is cut off; that's"
echo "expected until windows size their panes (T43). Nothing to judge here."
"$repo/target/release/examples/set-size" "$pane" 200 60 >/dev/null
sleep 1
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
if [[ -n $times ]]; then
    # Nearest-rank percentiles over the sorted times.
    stats=$(awk '
        function rank(q,  i) { i = int(q * NR + 0.999); return a[i < 1 ? 1 : i] }
        { a[NR] = $1; if ($1 >= 4) over++ }
        END { printf "frames=%d p50=%.2f p95=%.2f max=%.2f ms, %d over 4 ms", NR, rank(0.5), rank(0.95), a[NR], over }' <<<"$times")
    record "TIME  $stats"
else
    record "TIME  no full redraws of 200 × 60 in $log"
fi
"$repo/target/release/examples/set-size" "$pane" "$cols" "$rows" >/dev/null

say "7. Done"
read -r -p "Anything else you noticed? (Enter to skip) " note
[[ -n $note ]] && record "NOTE  $note"
read -r -p "Stop Shoal now? [y/n] " answer
[[ $answer == [yY] ]] && "$shoal" stop
echo
echo "Saved to $results. Tell the agent it's done."
