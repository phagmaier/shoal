#!/usr/bin/env bash
# Read-only reference sources, not Cargo dependencies. Never run setup install as an agent.
set -euo pipefail

zed_rev=8dd255c3eafb5231056fe4e6f601cedd011f1c93
ghostty_rev=521cf2c5d30bd48028ca61546c64e7913764073d
root=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
cache="$root/.sources-$zed_rev-$ghostty_rev"
check_only=false
case "$*" in
    '') ;;
    --check) check_only=true ;;
    -h|--help) printf 'Usage: %s [--check]\n' "$0"; exit 0 ;;
    *) printf 'Usage: %s [--check]\n' "$0" >&2; exit 2 ;;
esac
fail() { printf 'fetch: %s\n' "$*" >&2; exit 1; }
command -v git >/dev/null || fail 'git is required'

check_repository() {
    local dir=$1 revision=$2 status
    [[ -d $dir/.git ]] || return 1
    [[ $(git --no-optional-locks -C "$dir" rev-parse --verify HEAD) == "$revision" ]] || return 1
    status=$(GIT_NO_LAZY_FETCH=1 git --no-optional-locks -C "$dir" status --porcelain --untracked-files=all) || return 1
    [[ -z $status ]] || return 1
    # Sources and metadata are immutable; status must never refresh the index.
    [[ -z $(find "$dir" ! -type l -perm /222 -print -quit) ]] || return 1
}
check_cache() {
    check_repository "$cache/zed" "$zed_rev" &&
        check_repository "$cache/ghostty" "$ghostty_rev" &&
        [[ -f $cache/zed/crates/gpui/Cargo.toml &&
           -f $cache/zed/crates/gpui/src/gpui.rs &&
           -d $cache/zed/crates/gpui/examples &&
           -f $cache/zed/crates/gpui_platform/Cargo.toml &&
           -f $cache/ghostty/include/ghostty/vt.h ]]
}
check_links() {
    [[ -L $root/gpui && $(readlink "$root/gpui") == "$(basename "$cache")/zed/crates/gpui" &&
       -L $root/gpui_platform && $(readlink "$root/gpui_platform") == "$(basename "$cache")/zed/crates/gpui_platform" &&
       -L $root/ghostty && $(readlink "$root/ghostty") == "$(basename "$cache")/ghostty" ]]
}

if $check_only; then
    check_cache && check_links || fail 'pinned sources are missing, modified, writable or incomplete; run third_party/fetch.sh'
    printf 'Pinned reference sources are intact.\n'
    exit 0
fi

# Refuse to overwrite user directories or unrecognized links, even on failure.
for name in gpui gpui_platform ghostty; do
    if [[ -e $root/$name || -L $root/$name ]]; then
        [[ -L $root/$name ]] || fail "$root/$name exists and is not a managed symlink; move it aside first"
        case "$(readlink "$root/$name")" in
            .sources-*/zed/crates/"$name"|.sources-*/ghostty) ;;
            *) fail "$root/$name is not a managed symlink; move it aside first" ;;
        esac
    fi
done

if [[ -e $cache || -L $cache ]]; then
    check_cache || fail "$cache is incomplete or modified; move it aside before fetching again"
else
    staging=$(mktemp -d "$root/.fetch-XXXXXXXX")
    cleanup() {
        if [[ -n ${staging:-} && -d $staging ]]; then
            chmod -R u+w "$staging"
            rm -rf -- "$staging"
        fi
    }
    trap cleanup EXIT
    fetch_repository() {
        local url=$1 revision=$2 destination=$3
        shift 3
        git init -q "$destination"
        git -C "$destination" remote add origin "$url"
        git -C "$destination" sparse-checkout init --cone
        git -C "$destination" sparse-checkout set "$@"
        git -C "$destination" -c protocol.version=2 fetch --depth=1 --filter=blob:none origin "$revision"
        git -C "$destination" checkout --detach FETCH_HEAD
    }
    fetch_repository https://github.com/zed-industries/zed.git "$zed_rev" "$staging/zed" crates/gpui crates/gpui_platform
    fetch_repository https://github.com/ghostty-org/ghostty.git "$ghostty_rev" "$staging/ghostty" include/ghostty
    chmod -R a-w "$staging"
    mv -- "$staging" "$cache"
    staging=''
    check_cache || fail "fetched source is incomplete; move $cache aside before retrying"
fi

for name in gpui gpui_platform ghostty; do
    target="$(basename "$cache")/zed/crates/$name"
    [[ $name != ghostty ]] || target="$(basename "$cache")/ghostty"
    # Only previously recognized symlinks can be replaced; never recurse or delete a directory.
    ln -sfnT -- "$target" "$root/$name"
done
check_links || fail 'could not publish source links'
printf 'Pinned reference sources are ready.\n'
