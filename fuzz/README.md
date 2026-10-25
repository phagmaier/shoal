# Protocol decoder fuzzing

Run from the repository root, with the pinned cargo-fuzz and an installed nightly:

```sh
mise exec -- just fuzz-proto -- -max_total_time=600 -timeout=5 -rss_limit_mb=2048 -max_len=65536
```

The target checks arbitrary/truncated inputs, concatenated frames, frame encode/decode
round trips, JSON-RPC envelopes and typed request/notification parsing. Named seeds
come from the protocol fixtures. New corpus entries, crashes and build output are
ignored; preserve any regression as a focused test before fixing it. The deterministic
frame tests cover the exact 64 MiB limit without making fuzz inputs that large.

`fuzz/Cargo.lock` is committed because this is a separate cargo-fuzz workspace.
For a locked verification build:

```sh
mise exec -- cargo check --manifest-path fuzz/Cargo.toml --locked --all-targets
```
