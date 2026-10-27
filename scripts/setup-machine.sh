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
    printf 'Usage: %s [--check]\n' "$0"
}

check_only=false
case "$*" in
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
    ttf-jetbrains-mono noto-fonts noto-fonts-emoji
    sysstat hyperfine socat jq python xdg-utils
)

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

step "Pinned tools (mise.toml)"
if [[ ! -f $repo/mise.toml ]]; then
    missing "mise.toml"
elif ! command -v mise >/dev/null; then
    missing "mise (install the packages above first)"
elif $check_only; then
    # An untrusted config or failed inspection is not evidence of readiness.
    if ! trust=$(cd "$repo" && MISE_OFFLINE=true MISE_COLOR=0 mise trust --show 2>&1 </dev/null); then
        missing "cannot inspect mise config trust: $trust"
    elif [[ -z $trust || $trust == *': untrusted'* ]]; then
        missing "mise config is not trusted (run scripts/setup-machine.sh): $trust"
    elif ! inspection=$(cd "$repo" && MISE_OFFLINE=true mise ls --current --missing 2>&1 </dev/null); then
        missing "cannot inspect mise tools: $inspection"
    elif [[ -n $inspection ]]; then
        missing "mise tools: $inspection (run scripts/setup-machine.sh)"
    else
        ok "mise tools installed"
    fi
else
    mise trust "$repo/mise.toml"
    (cd "$repo" && mise install)
    ok "mise tools installed"
fi

step "Rust toolchains"
if ! command -v rustup >/dev/null; then
    missing "rustup (install the packages above first)"
elif [[ ! -f $repo/rust-toolchain.toml ]]; then
    missing "rust-toolchain.toml"
else
    # This repository uses a literal channel and string-array components, not
    # arbitrary TOML. Read both so a pin/component change updates this check too.
    channel=''
    components=()
    in_components=false
    while IFS= read -r line; do
        if [[ $line =~ ^[[:space:]]*channel[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
            channel=${BASH_REMATCH[1]}
        fi
        if [[ $line =~ ^[[:space:]]*components[[:space:]]*= ]]; then
            in_components=true
        fi
        if $in_components; then
            values=$line
            while [[ $values =~ \"([^\"]+)\" ]]; do
                components+=("${BASH_REMATCH[1]}")
                values=${values#*"${BASH_REMATCH[0]}"}
            done
            [[ $line != *']'* ]] || in_components=false
        fi
    done < "$repo/rust-toolchain.toml"
    if [[ -z $channel ]]; then
        missing "cannot read channel from rust-toolchain.toml"
    elif $check_only; then
        # Disable rustup's implicit installation and mise's inherited override.
        if active=$(cd "$repo" && env -u RUSTUP_TOOLCHAIN RUSTUP_AUTO_INSTALL=0 rustup show active-toolchain 2>&1) &&
            [[ $active == "$channel-"* || $active == "$channel "* ]]; then
            if installed=$(env -u RUSTUP_TOOLCHAIN RUSTUP_AUTO_INSTALL=0 rustup component list --toolchain "$channel" --installed 2>&1); then
                ok "Rust $channel selected by rust-toolchain.toml"
                for component in "${components[@]}"; do
                    found=false
                    while IFS= read -r entry; do
                        if [[ $entry == "$component" || $entry == "$component-"* ]]; then
                            found=true
                        fi
                    done <<< "$installed"
                    $found && ok "$component" || missing "Rust $channel component $component"
                done
            else
                missing "cannot inspect Rust $channel components: $installed"
            fi
        else
            missing "Rust $channel from rust-toolchain.toml: $active"
        fi
    else
        (cd "$repo" && env -u RUSTUP_TOOLCHAIN rustup toolchain install "$channel" --profile minimal)
        if ((${#components[@]})); then
            env -u RUSTUP_TOOLCHAIN rustup component add --toolchain "$channel" "${components[@]}"
        fi
        ok "Rust $channel and requested components"
    fi
    # cargo-fuzz requires nightly, independently of the workspace compiler.
    if toolchains=$(rustup toolchain list 2>&1); then
        nightly_present=false
        while IFS= read -r entry; do
            [[ $entry != nightly-* ]] || nightly_present=true
        done <<< "$toolchains"
        if $nightly_present; then
            ok "nightly (cargo-fuzz)"
        elif $check_only; then
            missing "Rust nightly (cargo-fuzz)"
        else
            rustup toolchain install nightly --profile minimal
            ok "installed nightly (cargo-fuzz)"
        fi
    else
        missing "cannot inspect Rust toolchains: $toolchains"
    fi
fi

step "Vendored sources (third_party/)"
if [[ ! -x $repo/third_party/fetch.sh ]]; then
    missing "third_party/fetch.sh"
elif $check_only; then
    if "$repo/third_party/fetch.sh" --check; then
        ok "pinned reference sources"
    else
        missing "pinned reference sources (run third_party/fetch.sh)"
    fi
else
    "$repo/third_party/fetch.sh"
    ok "fetched pinned reference sources"
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
