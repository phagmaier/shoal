#!/usr/bin/env bash
# M1 spec §8.3 prompt budgets, measured with hyperfine on a release build:
# p95 under 5 ms outside a repository and under 30 ms inside a 5,000-file one.
# Reads the real config and theme, as a prompt does; writes only a temp dir.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cargo build --locked --release --quiet --manifest-path "$repo/Cargo.toml" --package shoal-cli
shoal="$repo/target/release/shoal"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mkdir "$work/plain"
mkdir "$work/repo"
(
    cd "$work/repo"
    git init -q -b main
    for dir in $(seq -w 1 50); do
        mkdir "src$dir"
        for file in $(seq -w 1 100); do
            echo "$dir $file" >"src$dir/file$file.txt"
        done
    done
    git add .
    git -c user.name=Shoal -c user.email=shoal@example.com commit -q -m files
    echo "edit" >>src01/file001.txt
)

fail=0
# name, directory, budget in ms
for case in "outside a repository:plain:5" "inside a 5,000-file repository:repo:30"; do
    IFS=: read -r name dir budget <<<"$case"
    (cd "$work/$dir" && hyperfine --shell=none --warmup 20 --runs 300 --style none \
        --export-json "$work/$dir.json" \
        "$shoal prompt --shell bash --status 0 --duration-ms 0 --jobs 0 --cols 120" >/dev/null)
    # hyperfine 2 reports seconds per run; sort them and take the 95th percentile.
    read -r p95 mean < <(jq -r '.results[0]
        | ([.measurements[].time_wall_clock.value] | sort) as $t
        | "\($t[($t | length) * 0.95 | floor] * 1000) \(.summary.time_wall_clock.mean * 1000)"' \
        "$work/$dir.json")
    verdict=$(awk -v p="$p95" -v b="$budget" 'BEGIN { print (p < b) ? "ok" : "over budget" }')
    printf '%-32s p95 %6.2f ms  mean %6.2f ms  budget %2d ms  %s\n' "$name" "$p95" "$mean" "$budget" "$verdict"
    [[ $verdict == ok ]] || fail=1
done
exit "$fail"
