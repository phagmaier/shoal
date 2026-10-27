set shell := ["bash", "-euo", "pipefail", "-c"]

# List available commands.
default:
    @just --list

# Required pre-commit checks, including the M1 dependency boundaries.
check:
    mise exec -- cargo fmt --all -- --check
    mise exec -- cargo fmt --manifest-path fuzz/Cargo.toml --all -- --check
    mise exec -- cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
    mise exec -- cargo clippy --manifest-path fuzz/Cargo.toml --locked --all-targets -- -D warnings
    mise exec -- cargo test --locked --workspace --all-features
    mise exec -- cargo deny --locked --all-features check bans
    mise exec -- scripts/check-boundaries.sh
    bash scripts/tests/manual-t43.sh
    bash scripts/tests/manual-t49.sh

# Run the workspace tests.
test:
    mise exec -- cargo test --locked --workspace --all-features

# The two development commands share a checkout-local, disposable XDG sandbox.
dev-daemon:
    mise exec -- scripts/dev-run.sh shoal-daemon shoald

dev-gui:
    mise exec -- scripts/dev-run.sh shoal-gui shoal-gui

# T04 supplies fuzz/Cargo.toml and the frame_decoder target; only fuzz uses nightly.
fuzz-proto *args:
    mise exec -- cargo +nightly fuzz run frame_decoder {{args}}

# Open a Shoal window from this checkout's release build (docs/using-shoal.md).
run:
    scripts/shoal.sh start

# Close this checkout's windows and stop shoald.
stop:
    scripts/shoal.sh stop

# A task's guided manual checks, e.g. `just manual t41`.
manual task:
    scripts/manual/{{task}}.sh

# A development shoald.service running this checkout's release build (T22).
install-unit:
    mise exec -- scripts/install-unit.sh install

uninstall-unit:
    mise exec -- scripts/install-unit.sh uninstall

# The native prompt's hyperfine budgets (M1 spec §8.3) on a release build.
bench-prompt:
    mise exec -- scripts/bench-prompt.sh
