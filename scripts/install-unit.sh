#!/usr/bin/env bash
# Install or remove a development shoald.service for this checkout's release
# build (T22 manual check). T50's `just install` installs the real one.
#   scripts/install-unit.sh install | uninstall
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
config=${XDG_CONFIG_HOME:-}
[[ $config == /* ]] || config="$HOME/.config"
unit="$config/systemd/user/shoald.service"

case ${1:-} in
install)
    cargo build --locked --release --quiet --manifest-path "$repo/Cargo.toml" \
        --package shoal-daemon --package shoal-cli
    mkdir -p "$(dirname "$unit")"
    sed "s|@BINDIR@|$repo/target/release|" "$repo/share/systemd/user/shoald.service.in" >"$unit"
    systemctl --user daemon-reload
    echo "Installed $unit (shoald from $repo/target/release)."
    ;;
uninstall)
    systemctl --user stop shoald.service 2>/dev/null || true
    rm -f "$unit"
    systemctl --user daemon-reload
    echo "Removed $unit."
    ;;
*)
    echo "usage: scripts/install-unit.sh install|uninstall" >&2
    exit 2
    ;;
esac
