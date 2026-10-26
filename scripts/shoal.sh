#!/usr/bin/env bash
# Run Shoal from this checkout's release build, against the real daemon
# socket and state, as if it were installed. `just run` and `just stop` call it.
#
#   scripts/shoal.sh start      build if needed, start the daemon if needed, open a window
#   scripts/shoal.sh stop       close this checkout's windows and stop the daemon
#   scripts/shoal.sh log        follow the newest window log
#   scripts/shoal.sh <command>  any `shoal` command: ls, send, kill, capture, daemon ...
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
bin="$repo/target/release"
logs="$repo/target/logs"

build() {
    echo "Building the release binaries (fast when nothing changed)..."
    (cd "$repo" && mise exec -- cargo build --locked --release --quiet \
        --package shoal-daemon --package shoal-cli --package shoal-gui \
        --example set-size)
    # Release builds find their shell integration in <prefix>/share, beside
    # bin (M1 spec §8.2); make target/ that prefix, as install-unit does.
    ln -sfn ../share "$repo/target/share"
}

case ${1:-start} in
start)
    build
    mkdir -p "$logs"
    log="$logs/shoal-gui-$(date +%Y%m%d-%H%M%S).log"
    ln -sfn "$(basename "$log")" "$logs/shoal-gui.log"
    # `shoal` would start the window with its output discarded; start it
    # directly so its log (debug level, with redraw timings) is kept.
    SHOAL_LOG=${SHOAL_LOG:-debug} setsid "$bin/shoal-gui" "${@:2}" >"$log" 2>&1 </dev/null &
    echo "Opened a window. Its log: $log"
    echo "It can't take typing until T42; type into it with: scripts/shoal.sh send <session> 'text\\n'"
    ;;
stop)
    pkill -f "^$bin/shoal-gui" && echo "Closed the windows." || true
    if [[ -x $bin/shoal ]]; then
        "$bin/shoal" daemon stop || true
    fi
    ;;
log)
    exec tail -n 50 -f "$logs/shoal-gui.log"
    ;;
*)
    [[ -x $bin/shoal ]] || build
    exec "$bin/shoal" "$@"
    ;;
esac
