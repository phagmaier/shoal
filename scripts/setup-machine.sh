#!/usr/bin/env bash
# Set up an Arch / Omarchy machine to build, test and run Shoal.
#
#   scripts/setup-machine.sh           install what's missing (asks for sudo for pacman)
#   scripts/setup-machine.sh --check   only report; installs nothing, never uses sudo
#
# Safe to re-run. Agents run --check only; installing is for the user.
# When the build needs a new package or tool, add it here in the same commit.
set -euo pipefail

usage() {
    sed -n '4,5p' "${BASH_SOURCE[0]}" | sed 's/^#   //'
}

check_only=false
case "${1:-}" in
    "") ;;
    --check) check_only=true ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

# System packages. Build: compilers, GPUI's Wayland/Vulkan/font stack.
# Tests and measuring: zsh for shell tests, sysstat (pidstat) and hyperfine for budgets.
packages=(
    base-devel clang cmake git rustup mise just bash zsh
    wayland libxkbcommon libxkbcommon-x11 libxcb fontconfig freetype2
    vulkan-icd-loader vulkan-headers alsa-lib openssl zstd
    ttf-jetbrains-mono noto-fonts-emoji
    sysstat hyperfine socat
)
cargo_tools=(cargo-deny cargo-insta cargo-fuzz)

problems=()
ok() { printf '  ok    %s\n' "$1"; }
warn() { printf '  note  %s\n' "$1"; }
missing() { printf '  miss  %s\n' "$1"; problems+=("$1"); }
step() { printf '\n%s\n' "$1"; }

if ! command -v pacman >/dev/null; then
    echo "This script expects Arch Linux (pacman)." >&2
    exit 1
fi

step "System packages"
mapfile -t absent < <(pacman -T "${packages[@]}" || true)
if ((${#absent[@]} == 0)); then
    ok "all ${#packages[@]} installed"
elif $check_only; then
    missing "packages: ${absent[*]}"
    echo "        fix: sudo pacman -S --needed ${absent[*]}"
else
    if pacman -Qq rust >/dev/null 2>&1; then
        warn "the 'rust' package conflicts with rustup; let pacman replace it"
    fi
    sudo pacman -S --needed "${absent[@]}"
    ok "installed ${absent[*]}"
fi

step "Vulkan driver"
drivers=$(find /usr/share/vulkan/icd.d -name '*.json' ! -name 'lvp_icd*' 2>/dev/null || true)
if [[ -n $drivers ]]; then
    ok "$(basename -a $drivers | tr '\n' ' ')"
else
    missing "no hardware Vulkan driver (vulkan-radeon, vulkan-intel or nvidia-utils)"
fi

step "Rust toolchains"
if ! command -v rustup >/dev/null; then
    missing "rustup (install the packages above first)"
else
    # Present toolchains are left alone, so re-running never upgrades Rust.
    # nightly is only for cargo-fuzz.
    for tc in stable nightly; do
        if rustup toolchain list | grep -q "^$tc-"; then
            ok "$tc"
        elif $check_only; then
            missing "rust $tc toolchain (run: rustup toolchain install $tc)"
        else
            rustup toolchain install "$tc" --profile minimal
            ok "installed $tc"
        fi
    done
    if ! $check_only; then
        rustup component add --toolchain stable rustfmt clippy
        rustup default >/dev/null 2>&1 || rustup default stable
    fi
    if [[ -f $repo/rust-toolchain.toml ]]; then
        if $check_only; then
            (cd "$repo" && rustup show active-toolchain >/dev/null 2>&1) \
                && ok "rust-toolchain.toml toolchain" \
                || missing "toolchain from rust-toolchain.toml (run: rustup toolchain install, in the repo)"
        else
            (cd "$repo" && rustup toolchain install)
            ok "rust-toolchain.toml toolchain"
        fi
    fi
fi

step "Pinned tools (mise.toml)"
if [[ ! -f $repo/mise.toml ]]; then
    warn "no mise.toml yet (task T00 adds it)"
elif ! command -v mise >/dev/null; then
    missing "mise (install the packages above first)"
elif $check_only; then
    if (cd "$repo" && mise ls --current 2>/dev/null | grep -q missing); then
        missing "mise tools (run: mise install, in the repo)"
    else
        ok "mise tools installed"
    fi
else
    mise trust "$repo/mise.toml"
    (cd "$repo" && mise install)
    ok "mise tools installed"
fi

step "Cargo tools"
for tool in "${cargo_tools[@]}"; do
    if command -v "$tool" >/dev/null || (cd "$repo" && mise which "$tool" >/dev/null 2>&1); then
        ok "$tool"
    elif $check_only; then
        missing "$tool (run: cargo install --locked $tool)"
    else
        cargo install --locked "$tool"
        ok "$tool"
    fi
done

step "Vendored sources (third_party/)"
if [[ -x $repo/third_party/fetch.sh ]]; then
    if $check_only; then
        warn "run third_party/fetch.sh to refresh the read-only copies"
    else
        "$repo/third_party/fetch.sh"
        ok "fetched"
    fi
else
    warn "no third_party/fetch.sh yet (task T01 adds it)"
fi

step "Session (checked, not changed)"
[[ -n ${WAYLAND_DISPLAY:-} ]] && ok "Wayland ($WAYLAND_DISPLAY)" || missing "not a Wayland session"
if pgrep -x Hyprland >/dev/null || [[ -d ${XDG_RUNTIME_DIR:-/nonexistent}/hypr ]]; then
    ok "Hyprland"
else
    missing "Hyprland is not running"
fi
if [[ -f $HOME/.local/state/omarchy/current/theme/colors.toml ]]; then
    ok "Omarchy 4 theme (~/.local/state/omarchy/current/theme/colors.toml)"
elif [[ -f $HOME/.config/omarchy/current/theme/colors.toml ]]; then
    ok "Omarchy 3 theme (~/.config/omarchy/current/theme/colors.toml)"
else
    warn "no Omarchy colors.toml; Shoal will use its built-in palette"
fi
if [[ -n $(git config --global user.name || true) && -n $(git config --global user.email || true) ]]; then
    ok "git identity"
else
    missing "git identity (git config --global user.name / user.email)"
fi
command -v fcitx5 >/dev/null && ok "input method (fcitx5)" || warn "no input method found; the spike's IME check is skipped"

echo
if ((${#problems[@]} == 0)); then
    echo "Ready."
else
    echo "Missing ${#problems[@]} item(s):"
    printf '  - %s\n' "${problems[@]}"
    exit 1
fi
