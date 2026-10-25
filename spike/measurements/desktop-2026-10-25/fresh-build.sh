#!/usr/bin/env bash
# Fresh-clone, empty-cache build of spike/t00, following docs/spike-00.md.
set -uo pipefail
F=$1
log() { printf '[%s] %s\n' "$(date -u +%FT%TZ)" "$*"; }
export CARGO_HOME=$F/cargo-home ZIG_GLOBAL_CACHE_DIR=$F/zig-cache
unset GHOSTTY_SOURCE_DIR
log "clone"
git clone --branch spike/t00 https://github.com/phagmaier/shoal.git $F/shoal 2>&1 | tail -1
cd $F/shoal || exit 1
log "HEAD $(git rev-parse HEAD)"
log "cache state: CARGO_HOME entries=$(ls -A $CARGO_HOME | wc -l) ZIG cache entries=$(ls -A $ZIG_GLOBAL_CACHE_DIR | wc -l) target exists=$(test -e spike/target && echo yes || echo no) ~/.cache/zig exists=$(test -e ~/.cache/zig && echo yes || echo no)"
mise trust mise.toml && mise trust spike/mise.toml
scripts/setup-machine.sh --check | tail -1
mise install 2>&1 | tail -1
cd spike
mise exec -- sh -c 'echo "toolchain: $(rustc --version); $(cargo --version); zig $(zig version); RUSTUP_TOOLCHAIN=$RUSTUP_TOOLCHAIN CARGO_HOME=$CARGO_HOME"'
log "build start"
start=$(date +%s)
mise exec -- cargo build --release --locked
rc=$?
log "build exit=$rc wall=$(( $(date +%s) - start ))s"
ls -la target/release/spike-gui target/release/snapcheck target/release/rowbench 2>&1
sha256sum target/release/spike-gui target/release/snapcheck target/release/rowbench 2>&1
exit $rc
