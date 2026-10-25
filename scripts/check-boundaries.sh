#!/usr/bin/env bash
# cargo-deny wrappers enforce crate edges; these rules also need dependency kind.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cargo metadata --manifest-path "$repo/Cargo.toml" --locked --no-deps --all-features --format-version 1 |
    jq -e '
        [.packages[] | select(.name | startswith("shoal-")) |
            .name as $from | .dependencies[] |
            select(
                ($from == "shoal-proto" and .name == "tokio") or
                (.name == "shoal-testkit" and .kind != "dev")
            ) |
            "\($from): forbidden \(.kind // "normal") dependency on \(.name)"
        ] as $errors |
        if ($errors | length) == 0 then true
        else error($errors | join("\n")) end
    ' >/dev/null
