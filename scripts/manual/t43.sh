#!/usr/bin/env bash
# T43 manual checks (layout interactions and tab bar), guided. Opens its own
# window, asks you to click, drag and type in it, checks after each step that
# every shown pane's `stty size` matches the size the daemon and the window
# agree on, and saves everything to target/manual/t43.txt.
#   scripts/manual/t43.sh
# Individual checks can fail; keep collecting evidence for the later steps.
set -uo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
shoal="$repo/scripts/shoal.sh"
results="$repo/target/manual/t43.txt"
mkdir -p "$(dirname "$results")"
: >"$results"

say() { printf '\n\e[1m%s\e[0m\n' "$*"; }
record() { printf '%s\n' "$*" | tee -a "$results"; }
ask() {
    local answer note=""
    while true; do
        read -r -p "$1 [y/n] " answer || { record "FAIL  Input ended during: $1"; exit 1; }
        case $answer in
        y | Y) record "PASS  $1"; return 0 ;;
        n | N)
            read -r -p "  What looked wrong? " note || note="Input ended"
            record "FAIL  $1 — $note"
            return 0
            ;;
        esac
    done
}
pause() { read -r -p "${1:-Press Enter when done.} " _ || { record "FAIL  Input ended before the next check"; exit 1; }; }
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
# Record PASS or FAIL for an automatic check: a label, then a command.
check() {
    local label=$1
    shift
    if "$@"; then record "PASS  (auto) $label"; else record "FAIL  (auto) $label"; fi
}

say "1. Opening a window"
echo "The terminal can't take typing yet. Only the tab rename box, Ctrl+Shift+R"
echo "(rename) and Ctrl+Shift+Enter (zoom) take keys. This script sends shell commands."
echo "Starting clean: closing this checkout's Shoal windows and stopping its daemon."
"$shoal" stop || record "FAIL  Could not stop the previous run"
if ! "$shoal" start; then
    record "FAIL  Could not build or open the first window"
    exit 1
fi
record "GUI built: $(stat -c '%y' "$repo/target/release/shoal-gui")"
log="$repo/target/logs/$(readlink "$repo/target/logs/shoal-gui.log")"
session=$(wait_for "$log" 'connected session=s[0-9]*' | sed 's/.*=//') ||
    { record "FAIL  The window didn't connect; see $log"; exit 1; }
record "session $session, window log $log"

json() {
    "$shoal" ls --json || { record "FAIL  Could not read the workspace" >&2; return 1; }
}
# Require the tab to exist, including for tests of a null (automatic) name.
tab_at() {
    json | jq -ce --arg s "$session" --argjson i "$1" '
        [.workspace.sessions[] | select(.id == $s)][0].tabs[$i] // error("tab missing")'
}
# The session's active tab, as JSON.
active_tab() {
    json | jq -c --arg s "$session" '.workspace.sessions[] | select(.id == $s)
        | . as $x | .tabs[] | select(.id == $x.active_tab)'
}
tab_count() { json | jq --arg s "$session" '.workspace.sessions[] | select(.id == $s) | .tabs | length'; }
# The panes on screen: the zoomed one, or every pane in the active tab.
shown_panes() {
    active_tab | jq -r 'if .zoomed then .zoomed else (.layout | .. | objects | select(has("pane")) | .pane) end'
}
focused_pane() { active_tab | jq -r .focused; }
# Each shown pane's `stty size` must equal its size in the daemon and the
# size the window last asked for.
sizes_match() {
    local pane panes ok=0 stty daemon asked
    panes=$(shown_panes) || return 1
    [[ -n $panes ]] || { record "      No shown panes to measure"; return 1; }
    for pane in $panes; do
        "$shoal" send "$pane" 'clear; stty size\n' >/dev/null || ok=1
    done
    # Poll the actual output, not a fixed delay; a busy shell can take time.
    for pane in $panes; do
        daemon=$(json | jq -r --arg p "$pane" '.workspace.panes[$p].size | "\(.rows) \(.cols)"')
        for _ in $(seq 25); do
            stty=$("$shoal" capture "$pane" | sed -n '1p')
            [[ $stty == "$daemon" ]] && break
            sleep 0.2
        done
        asked=$(grep "asking for a size pane=$pane " "$log" | tail -1 |
            sed 's/.*cols=\([0-9]*\) rows=\([0-9]*\).*/\2 \1/')
        record "      $pane: stty $stty, daemon $daemon, window asked ${asked:-nothing}"
        if [[ ! $daemon =~ ^[0-9]+\ [0-9]+$ || $stty != "$daemon" || ( -n $asked && $asked != "$daemon" ) ]]; then ok=1; fi
    done
    return $ok
}

say "2. Fullscreen"
echo "Click the new window, then press Super+F to make it fullscreen."
pause
check "fullscreen: every pane's grid fits the window" sizes_match
ask "The prompt and the line with the two numbers sit at the top left, and the pane box fills the screen?"
echo "Press Super+F again to leave fullscreen."
pause
check "back to tiled: the grid follows" sizes_match

say "3. Split buttons and tooltips"
echo "Hover over the two buttons at the top right for a moment each."
ask "Their tooltips read \"Split right\" with key caps Ctrl Shift O, and \"Split down\" with Ctrl Shift E?"
echo "Click \"Split right\" (the left one of the two)."
pause
check "split right made a second pane" test "$(shown_panes | wc -l)" -eq 2
echo "Click \"Split down\"."
pause
check "split down made a third pane" test "$(shown_panes | wc -l)" -eq 3
check "three panes: every grid fits its box" sizes_match
ask "Three panes, the focused one with the accent border, each prompt inside its box?"

say "4. Click to focus"
before=$(focused_pane)
echo "Click inside a pane that does NOT have the accent border."
pause
focus_changed() { local after; after=$(focused_pane); [[ $before == p[0-9]* && $after == p[0-9]* && $after != "$before" ]]; }
check "the click moved focus" focus_changed
ask "The accent border moved to the pane you clicked?"

say "5. Dragging dividers"
mark=$(wc -l <"$log")
echo "Drag the gap between the left pane and the right ones slowly to the right"
echo "and back a bit, then let go. Then drag the gap between the two right panes"
echo "up or down. The pointer should turn into a resize arrow over each gap."
pause
check "after the drags: every grid fits its box" sizes_match
requests=$(tail -n "+$((mark + 1))" "$log" | grep -c 'asking for a size' || true)
record "INFO  $requests size requests during the drags (they wait until the grid holds still for 50 ms)"
ask "The resize pointer showed over the gaps, and the dividers followed the pointer smoothly?"
ask "After letting go, each pane's text was redrawn to fit its box (nothing cut off, no wide empty band)?"

say "6. Zoom"
echo "With the window focused, press Ctrl+Shift+Enter."
pause
check "the focused pane is zoomed" test "$(active_tab | jq -r '.zoomed != null')" = true
check "zoomed: the grid fills the area" sizes_match
ask "One pane fills the whole area, and the tab's icon changed to the zoom icon?"
echo "Press Ctrl+Shift+Enter again."
pause
check "unzoomed" test "$(active_tab | jq -r '.zoomed == null')" = true
check "unzoomed: every grid fits its box again" sizes_match

say "7. Tabs"
tabs=$(tab_count)
echo "Hover over the + after the tab, then click it."
pause
check "+ made a tab" test "$(tab_count)" -eq $((tabs + 1))
ask "The + tooltip read \"New tab\" with Ctrl Shift T, and the new tab became active?"
first=$(json | jq -r --arg s "$session" '.workspace.sessions[] | select(.id == $s) | .tabs[0].id')
echo "Click the first tab."
pause
check "clicking selected the first tab" test "$(active_tab | jq -r .id)" = "$first"
echo "Double-click the second tab, type: logs, and press Enter."
pause
name=$(tab_at 1 | jq -r .name)
check "double-click renamed the tab to logs" test "$name" = logs
echo "Click the first tab, press Ctrl+Shift+R, type: build, then press Escape."
pause
name=$(tab_at 0 | jq -r .name)
check "Escape cancelled the rename" test "$name" = null
echo "Double-click the \"logs\" tab, press Backspace, then Enter."
pause
name=$(tab_at 1 | jq -r .name)
check "an empty name gave the tab its automatic title back" test "$name" = null
ask "While renaming, the whole label was highlighted at first, and a caret showed where you typed?"

say "8. Bell dot"
echo "Click the first tab, so the second one is in the background."
pause
ringing=$(tab_at 1 | jq -r .focused)
if [[ $ringing == p[0-9]* ]] && "$shoal" send "$ringing" 'printf "\\a"\n' >/dev/null; then
    ask "The second tab shows a small accent-colored dot?"
    echo "Click the second tab."
    pause
    check "selecting the tab cleared its bell" test "$(json | jq -r --arg p "$ringing" '.workspace.panes[$p].bell')" = false
else
    record "FAIL  Bell check: second tab missing or could not send the bell"
fi

say "9. Middle-click closes a tab"
tabs=$(tab_count)
echo "Middle-click the second tab."
pause
check "middle-click closed the tab" test "$(tab_count)" -eq $((tabs - 1))

say "10. Two windows: the focused one sizes the panes"
if "$shoal" start --session "$session"; then
    second="$repo/target/logs/$(readlink "$repo/target/logs/shoal-gui.log")"
    if wait_for "$second" "connected session=$session\\b" >/dev/null; then
        grids() { json | jq -c '[.workspace.panes[] | .size]'; }
        echo "A second window opened on the same session. Hold Super and drag with the"
        echo "right mouse button to make one of the two windows clearly bigger."
        echo "Then click inside the FIRST window."
        pause
        check "the first window's sizes count" sizes_match
        in_first=$(grids)
        log_first=$log
        log=$second
        echo "Now click inside the SECOND window."
        pause
        check "the second window's sizes count" sizes_match
        in_second=$(grids)
        log=$log_first
        record "INFO  grids with the first window focused $in_first, with the second $in_second"
        check "the panes changed size when focus moved" test "$in_first" != "$in_second"
        ask "In whichever window isn't focused, the panes are drawn padded or cut off, not garbled?"
        echo "Close the second window (Super+W with it focused)."
        pause
    else
        record "FAIL  The second window didn't connect; see $second"
    fi
else
    record "FAIL  Could not open the second window"
fi

say "11. Idle"
gui=$(pgrep -n -f "^$repo/target/release/shoal-gui" || true)
if [[ -n $gui ]]; then
    if start=$(awk '{print $14 + $15}' "/proc/$gui/stat") && sleep 10 &&
        end=$(awk '{print $14 + $15}' "/proc/$gui/stat"); then
        record "INFO  window CPU over 10 s idle: $((end - start)) ticks (100 = one CPU-second)"
    else
        record "FAIL  Window exited during the idle check"
    fi
else
    record "FAIL  No window for the idle check"
fi

say "12. Done"
read -r -p "Anything else you noticed? (Enter to skip) " note || note=""
[[ -n $note ]] && record "NOTE  $note"
read -r -p "Stop Shoal now? [y/n] " answer || answer=n
if [[ $answer == [yY] ]]; then "$shoal" stop || record "FAIL  Could not stop Shoal"; fi
echo
echo "Saved to $results. Tell the agent it's done."
! grep -q '^FAIL ' "$results"
