#!/usr/bin/env bash
# Development binaries must never use the real Shoal socket or state.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
sandbox="$repo/target/dev-env"
umask 077
# Build tools retain their stores; the program gets an isolated HOME.
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
# Wayland still connects to the real compositor when its socket name is relative.
if [[ -n ${WAYLAND_DISPLAY:-} && $WAYLAND_DISPLAY != /* ]]; then
    export WAYLAND_DISPLAY="${XDG_RUNTIME_DIR:?Wayland needs XDG_RUNTIME_DIR}/$WAYLAND_DISPLAY"
fi
mkdir -p "$sandbox"/{home,runtime,config,state,cache,data}
export HOME="$sandbox/home"
export XDG_RUNTIME_DIR="$sandbox/runtime"
export XDG_CONFIG_HOME="$sandbox/config"
export XDG_STATE_HOME="$sandbox/state"
export XDG_CACHE_HOME="$sandbox/cache"
export XDG_DATA_HOME="$sandbox/data"
export RUST_LOG="${RUST_LOG:-shoal=debug,shoald=debug}"
# mise selects the pinned tools before this script changes HOME.
exec cargo run --locked --manifest-path "$repo/Cargo.toml" --package "$1" --bin "$2"
