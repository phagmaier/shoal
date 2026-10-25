# T00 — GPUI + libghostty spike

**T00 is complete (2026-10-25).** GPUI is kept, and the spec now records the user's decisions (section "Decisions recorded in the spec"). Evidence comes from two machines, kept apart: the original **laptop** (rounds 1 and 2) and a **desktop** that took over development the same day (section "Second machine"). Never pool the two machines' performance numbers.

- **Redraw:** the 4 ms target is unchanged. Laptop, round 2, full-damage styled 200 × 60 on battery under `balanced`: GPUI frame p50 2.64, p95 3.19, p99 3.44 ms, max 4.11 ms. **599 of 600 frames came in under 4 ms; one took 4.11 ms.** The spike's own render-to-paint time never exceeded 3.31 ms. Desktop: **all 600 frames under 4 ms**, GPUI p50 2.24, p95 2.48, p99 2.82, max 3.13 ms, at scale 1 (a quarter of the laptop's device pixels). Neither machine was measured under `power-saver`. These are measurements against the target, not a percentile replacement for it.
- **Snapshot:** the formatter fails the fidelity gate. Live attach uses libghostty's binary snapshot at an exact build: it matched 31/31 checked cases at restore and after follow-up output, on both machines.
- **Clean checkout:** passed on the desktop. A fresh clone of `spike/t00` (94f8d92) built with `cargo build --release --locked` through mise, with empty Cargo, Zig and target caches (section "Fresh-clone build").

T00 commits on `spike/t00` (never merged): d499ff2 (spike sources, measurements, this report), 94f8d92 (formatting, font attribution, Rust pin), 09c6967 (design checks, `--scale`, desktop environment and build log), plus the commit that adds the desktop measurements. `main` carries only this report, `mise.toml`, and the `docs/spec/` edits. T01 has not started; it needs a fresh session.

## Go/no-go checklist

These are the six gates in [decisions.md, Risks and the spike](spec/decisions.md#risks-and-the-spike), not revised acceptance criteria.

| Gate | Status | Evidence |
| --- | --- | --- |
| Transparent GPUI window, Hyprland blur, crisp text at the user's scale | PASS (laptop, by eye) | On the laptop at scale 2, the user confirmed translucency, crisp text and wallpaper blur. The desktop (scale 1, blur turned off in its Hyprland config) has not been checked by eye; that check is optional (see "By-eye checks"). |
| 200 × 60 fed by bash through libghostty, redraw under 4 ms | PASS with the qualification above; round 1 FAIL | Laptop round 2, cached glyph placement: GPUI p50 **2.64** / p95 **3.19** / p99 **3.44** / max **4.11** ms, one of 600 frames over 4 ms; internal p50 1.84 / p95 2.34 / max 3.31 ms. Desktop: GPUI p50 **2.24** / p95 **2.48** / p99 **2.82** / max **3.13** ms, none over 4 ms. Per-run shaping misses on both: laptop GPUI p50 4.32 ms; desktop p50 3.70 ms, 55 of 600 frames over 4 ms. Round 1 (laptop, probably `power-saver`): GPUI p50 7.42 ms. |
| `cat` 50 MB finishes; idle CPU near zero | PASS, limited to this spike | Laptop round 2: 1.079 / 1.262 / 1.249 s, 64–75 paints while `cat` writes, 0.00% idle CPU over 30 s. Desktop: **0.493 / 0.480 / 0.496 s**, 26–28 paints, **0.00%** idle CPU over 30 s, 21 threads, sleeping. All runs exit 0 and consume all 52,798,016 PTY bytes. Not the future daemon + ten-pane M1 budget. |
| Formatter restore equals original screen, scrollback and modes | Formatter FAIL; binary snapshot adopted for live attach | Laptop: formatter 25/31 at restore, 17/31 after follow-up, 19/31 with continuation; binary 31/31 and 31/31. Desktop rerun: formatter 25/31, **16/31**, **18/31** (the `less -R` row failed after follow-up here, as it did in an earlier laptop session); binary **31/31** and **31/31**. The user approved the exact-build binary snapshot. The spec now uses it, with the review's corrections. |
| Input method reaches the terminal, if used | SKIPPED (no input method) | User confirms no input method is used: “i type only in english and don't plan to type in anything else.” This optional test is skipped, not an IME functionality pass. fcitx5 is installed on the desktop but not used. |
| Pins documented and build steps work from a clean checkout | PASS (desktop) | Full pins and recipe below. A fresh clone of 94f8d92 built in 8 min 53 s with empty `CARGO_HOME`, Zig cache and target; the toolchains were pre-installed. The laptop's builds reused existing artifacts and are not clean-checkout evidence. |

## Environment and pins

This table is the **laptop** used for rounds 1 and 2; the desktop is described in "Second machine". Raw host evidence: `spike/measurements/environment.txt` and `environment-extra.txt` on `spike/t00`. CPU/GPU values are host queries, not an isolated performance lab. No governor, temperature, competing-workload, or GPU-completion measurements were taken.

| Item | Observed value |
| --- | --- |
| OS | Omarchy **4.0.6**, Arch-based; Linux **7.2.8-9-omarchy**, x86_64 |
| CPU | AMD Ryzen 7 7840U, 8 cores / 16 logical CPUs |
| Hyprland | **0.56.2**, `efb50993780079460b0cbed1363e2166a2de1d9f`; Lua configuration |
| Session | Native Wayland, `wayland-1`; `hyprctl` reports `xwayland: false` for the spike |
| Monitor | `eDP-1`, BOE **0x0BCA**, **2256 × 1504**, **59.999 Hz**, scale **2**; logical **1128 × 752**, top reserved area 26 logical px |
| Vulkan hardware device | AMD Radeon 780M Graphics, **RADV PHOENIX**, Mesa **26.2.5-arch1.1**, Vulkan device API **1.4.354**; loader **1.4.363**. llvmpipe is also enumerated. No app-specific adapter-selection trace was captured. |
| Rust | **1.98.1**, `rustc 1.98.1 (48a229cea 2026-09-01)` |
| Zig / mise | Zig **0.17.0** via `mise.toml`; mise **2026.10.17** |
| Main font | JetBrains Mono Regular, package **2.304-2**; `fc-match 'JetBrains Mono'` selects `JetBrainsMono-Regular.ttf` |
| Font fallbacks | Embedded Symbols Nerd Font Mono, then Noto Color Emoji, then system fallback |
| Shell / measurement | Bash **5.3.20**, sysstat/pidstat **12.8.0** |
| Real-program fixtures | neovim **0.12.6**, less **710**, btop **1.4.7**, fzf **0.74.5**, lazygit **0.66.2**; zsh **5.9.2** is installed but not exercised by snapcheck |
| Theme | Omarchy 4 `~/.local/state/omarchy/current/theme/colors.toml` |
| Power | Round 1: not recorded. Round 2 timed runs: battery, `balanced` profile, governor `powersave`, EPP `balance_power`, recorded per command in each folder's `power.jsonl` |

On the laptop, `scripts/setup-machine.sh --check` was run **once**: all 25 packages, Vulkan driver, Rust toolchains, pinned mise tools, cargo tools, Wayland, Hyprland, theme and git identity ready. It notes that `third_party/fetch.sh` awaits T01 and no input method was found. No sudo or package installation was used; no setup-script gap was found for the work actually performed. On the desktop it reported `Ready.` with the same results, except that it found fcitx5 and the Intel Vulkan driver. One gap the script doesn't check: a global mise `rust` entry exports `RUSTUP_TOOLCHAIN`, which overrides `rust-toolchain.toml` (see the recipe).

The agent shell lacked both `WAYLAND_DISPLAY` and `HYPRLAND_INSTANCE_SIGNATURE`; the driver discovers the unique Wayland socket and live instance under `$XDG_RUNTIME_DIR/hypr/`. A normal interactive desktop shell may already export them.

| Dependency | Full pin |
| --- | --- |
| GPUI / gpui_platform, Zed v1.25.3 source | `8dd255c3eafb5231056fe4e6f601cedd011f1c93` |
| libghostty-rs / libghostty-vt | `9ede35b8ea94a55ed3608408d67e102883129f1e` |
| Ghostty, selected by the bindings' build.rs | `521cf2c5d30bd48028ca61546c64e7913764073d` |
| Zed async-task patch | `015845a19fc31752c02367567d3b8900a406f9da` |
| Zed calloop patch | `3759371fee14c40066d64777e2a36b6ffcc22590` |

The Zed tag is not the GPUI crate version: its `crates/gpui/Cargo.toml` still says **0.2.2**. Linux Wayland uses `gpui_linux` and `gpui_wgpu`; `gpui_platform` enables `wayland,font-kit`. Transitive versions are fixed by `spike/Cargo.lock`; portable-pty is 0.9.0. The two patches repeat the upstream workspace patches that do not propagate to an external git consumer.

### Font provenance

The bundled `SymbolsNerdFontMono-Regular.ttf` (2,610,012 bytes) is the unmodified file from Nerd Fonts **v3.5.1** `NerdFontsSymbolsOnly.zip`. Font SHA-256 `fe471e538392f51910faab985fa8e192a39dd3426125edd15b71b3680df0e749`; archive SHA-256 `fdca3682534f6f65e1ccb2345b0362ccf67d9b8eca7c8025330946e93e2473bc`. An earlier version of this paragraph attached the font hash to the archive name. On 2026-10-25 the archive was downloaded again and the font compared byte for byte with the committed copy.

The archive LICENSE is MIT, and Nerd Fonts lists Symbols Only as MIT. Its [icon-source table](https://github.com/ryanoasis/nerd-fonts/blob/v3.5.1/patched-fonts/NerdFontsSymbolsOnly/README.md) shows the glyphs come from sets under MIT, CC BY 4.0 (Codicons, Font Awesome icons), OFL 1.1 (Pomicons with RFN, Weather Icons; Font Awesome's font files), Apache 2.0 (MaterialDesign), plus one "unlicensed" set (Font Logos). **The font is not uniformly MIT or OFL.**

On `spike/t00`, the font was committed in d499ff2. Attribution was added in 94f8d92: `spike/assets/README.md` records provenance, hashes and the per-set table, and `spike/assets/licenses/` holds the upstream LICENSE, README and license audit, every per-set license file nerd-fonts ships at v3.5.1, and the full Apache 2.0 text. Gaps: five MIT sets have no license file upstream, the Weather Icons OFL copy still has template placeholders, and Font Logos has no license. That is acceptable for a throwaway branch that is never shipped. Before Shoal ships the font, T50 must install the license texts and the user must decide whether to subset out the Font Logos glyphs or obtain terms (M1 spec §7.3, `tasks.md` T50).

## Build and reproduction

All commands below are from the repository root unless stated otherwise. T00 is an independent workspace; there is no root Cargo workspace, `justfile`, vendored tree, daemon, or test kit yet.

### Clean-checkout recipe

```sh
git clone --branch spike/t00 https://github.com/phagmaier/shoal.git && cd shoal
scripts/setup-machine.sh --check
mise trust mise.toml && mise trust spike/mise.toml
mise install          # Zig 0.17.0 (root), Rust 1.98.1 with rustfmt and clippy (spike/mise.toml)
cd spike
mise exec -- cargo build --release --locked
```

Run on a machine with the setup script's prerequisites already installed. If `--check` reports missing system packages, the user runs the package command it prints; an agent must not sudo. A first build needs network access to the pinned Cargo git dependencies, crates and Ghostty/Zig dependencies, or pre-populated caches. No `/tmp/shoal-research` checkout is required: the bindings fetch their exact Ghostty pin themselves. Avoid unpinned `GHOSTTY_SOURCE_DIR` overrides.

**Why `spike/mise.toml`:** on the desktop the user's global mise config pins `rust = "1.97.1"`, and mise exports that as `RUSTUP_TOOLCHAIN`, which overrides `spike/rust-toolchain.toml`. The original recipe, `rustup toolchain install 1.98.1` plus `mise exec -- cargo build`, would quietly have built with 1.97.1 there. `spike/mise.toml` (94f8d92) pins Rust 1.98.1 for the spike folder only, and the recipe checks it: inside `spike/`, `mise exec` reports `RUSTUP_TOOLCHAIN=1.98.1` and `rustc 1.98.1`. A fresh clone at a new path also needs `mise trust` for both files. T01 will hit the same override when it adds the workspace `rust-toolchain.toml`.

`spike/.cargo/config.toml` sets **`LIBGHOSTTY_VT_SYS_OPTIMIZE=ReleaseFast`**. This matters: the bindings otherwise select Debug when Cargo supplies DEBUG=true, including profiles with debug information. The release profile retains line-table debug information. The bindings build libghostty-vt for the `baseline` CPU model unless `LIBGHOSTTY_VT_SYS_CPU` says otherwise. On an OOM kill, retry with a lower `CARGO_BUILD_JOBS` (8 or fewer; the desktop has 6 cores and needed no retry); don't clean and rebuild GPUI unnecessarily.

### Fresh-clone build

Verified on the desktop, 2026-10-25, with `spike/measurements/desktop-2026-10-25/fresh-build.sh <empty folder>`. Its full output is `fresh-clone-build.log` in the same folder on `spike/t00`.

- `git clone --branch spike/t00` from GitHub at **94f8d92**, into a new folder.
- **Empty caches, checked by the script before building:** a new, empty `CARGO_HOME` (so no registry, git checkouts or `~/.cargo/config.toml`), a new, empty `ZIG_GLOBAL_CACHE_DIR`, no `target/`, and no `~/.cache/zig` on the machine. No rustc wrapper was set. Every crate (442 downloads), every git dependency (Zed, wgpu, libghostty-rs, …) and Ghostty itself were fetched over the network during the build.
- **Pre-installed, not part of the claim:** the system packages, mise, the Rust 1.98.1 toolchain under `~/.rustup`, and Zig 0.17.0 under mise.
- Recipe exactly as above, plus `CARGO_HOME` and `ZIG_GLOBAL_CACHE_DIR` pointed at the empty folders. `--check`: `Ready.`; `mise exec` reported rustc 1.98.1, cargo 1.98.1, Zig 0.17.0.
- `cargo build --release --locked`: **exit 0, 8 min 53 s**, 432 crates compiled, default job count (6), no OOM. Cargo time includes the downloads.
- SHA-256: `spike-gui` `be9e9697…a349f2`, `snapcheck` `83844a2a…a61c0`, `rowbench` `e4674df4…ce8`. Rebuilding the same commit later in that clone gave identical hashes.

The laptop's builds (1.33 s and 2.26 s Cargo time) reused existing artifacts and are not clean-checkout evidence.

### Measurements

```sh
# Native Hyprland session required; Python driver is standard-library-only.
python3 spike/measure.py frames
python3 spike/measure.py cat-idle
python3 spike/measure.py summary
# Round 2 wrote to its own folders, e.g.:
python3 spike/measure.py frames --out spike/measurements/round2
python3 spike/measure.py frames --bin spike/target/round1/spike-gui --out spike/measurements/round2-round1-binary
python3 spike/measure.py frames --verify --frames 200 --out spike/measurements/round2-verify
python3 spike/measure.py summary --out spike/measurements/round2
./spike/target/release/rowbench          # headless builder cost breakdown

./spike/target/release/snapcheck --programs --verbose
./spike/target/release/snapcheck --bench
./spike/target/release/snapcheck --design   # added after the recorded runs (09c6967)

# Desktop (scale 1), from a fresh-clone build:
python3 spike/measure.py frames   --scale 1 --bin <clone>/spike/target/release/spike-gui --out spike/measurements/desktop-2026-10-25/frames
python3 spike/measure.py cat-idle --scale 1 --bin <clone>/spike/target/release/spike-gui --out spike/measurements/desktop-2026-10-25/cat-idle
```

snapcheck always exits 0, even when comparisons fail; read its table. Pass its flags as separate words: a zsh variable holding `--programs --verbose` arrives as one argument, which matches neither flag. That silently drops the six program rows and the details. It happened once on the desktop and the run was redone.

The driver saves raw stdout/stderr, timestamps, effective benchmark environment, exact per-window `hyprctl eval` calls and window metadata under **`spike/measurements/`**, outside `/tmp`. `summary` reproduces GPUI percentiles and cat durations in `summary.json`; raw internal percentile summaries are emitted by the executable. Snapshot logs are `snapcheck-programs-20261025T075721Z.log` and `snapcheck-bench-20261025T075721Z.log`. The requested binaries were actually run; no permanent test suite was added for this investigation.

The measurement driver assumes **scale 2** unless given `--scale` (added in 09c6967; the default keeps the recorded commands' meaning) and the Hyprland **Lua dispatcher API**, and refuses to start the workload until GPUI reports the requested full-size geometry with `fits=true`. It changes only its own window: floating, no animation, size 1800×1400, position (12,38). It creates no persistent compositor rules and closes each window. It synchronizes with the Hyprland event socket, observed geometry and FIFOs with deadlines, not sleeps. The live session rejected legacy `hyprctl keyword`/dispatcher syntax; use the recorded Lua API rather than old `windowrulev2` recipes.

Equivalent frame environment, for each mode `static`, `styled`, `typing`, `scroll`:

```sh
SPIKE_GRID=200x60 SPIKE_WINDOW=1800x1400 SPIKE_FONT_PX=14 \
SPIKE_BENCH=600 SPIKE_BENCH_MODE=styled SPIKE_RENDER=glyph \
SPIKE_CMD='read -r -t 60 line' ZED_MEASUREMENTS=1 \
WAYLAND_DISPLAY=wayland-1 ./spike/target/release/spike-gui
```

This direct command still needs the window resizing/geometry handshake performed by `measure.py`; a tiling compositor may ignore the requested initial size. `SPIKE_RENDER=shape` supplies the additional shaped-run comparison. Synthetic data is written directly into libghostty before each frame; this is not a timed bash echo/input-latency experiment. Real bash → PTY → libghostty output is measured separately below.

## Round 1 frame results: corrected geometry

**Measured client viewport 1800 × 1400 logical px, scale 2, font 14.00 logical px, cell 8.400 × 21.500 logical px, grid 200 × 60.** With 6 px padding on each edge, the entire grid fits inside the client surface. Derived device-space sizes: surface 3600×2800, font 28 px, cell 16.8×43 px; these are scale products, not GPU timestamp measurements.

**Monitor caveat:** a 14 px 200×60 grid cannot fit on this monitor's 1128×752 logical desktop at scale 2. The floating surface is deliberately oversized, extending beyond the physical monitor. All grid rows fit the client surface and are submitted to the spike renderer, but Hyprland can only show part of that surface. This measures full-size client work, not a fully visible 200×60 grid at native monitor scale or compositor presentation latency. No monitor scale was changed.

The previous 900×600 fixed-grid harness silently shrank the font. That code path is removed: fixed grids now keep the requested font size and report whether they fit. Earlier small-font results are **superseded**, not evidence of a normal-scale pass. The default by-eye window still requests 900×600 at 14 px and chooses its grid from the actual viewport.

For each synthetic run: 30 warm-up renders, then **600** internal build/paint samples. GPUI percentiles use the last 600 completed `frame duration` records **before** the BENCH summary, excluding the quit frame. Raw logs include the initial scale-1/configure frames; they precede the measured warm interval. One serial run per mode; no statistical confidence or machine-load isolation is claimed.

| Workload / text path | Build p50 ms | Paint p50 ms | Internal total p50 ms | Total p95 ms | GPUI frame p50 ms | GPUI p95 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Static, cached glyph placement | 1.65 | 1.57 | 3.24 | 5.10 | 3.93 | 6.24 |
| Full-damage styled, cached glyph placement | 2.28 | 2.69 | 5.02 | 5.19 | 7.42 | 7.77 |
| Typing: one cell/frame, cached glyph placement | 1.90 | 1.71 | 3.86 | 5.34 | 4.60 | 6.57 |
| Scroll: one log line/frame, cached glyph placement | 3.13 | 1.44 | 4.61 | 4.95 | 5.40 | 5.82 |
| Full-damage styled, shape style runs | 2.30 | 3.37 | 5.71 | 5.89 | 8.16 | 8.47 |

Raw files: `frames-glyph-{static,styled,typing,scroll}.log`, `frames-shape-styled.log` and corresponding `*-window.json`; all include p99/max internally, with GPUI p99/max in `summary.json`.

Timing boundaries:

- **Build:** `Builder::build`, including render-state extraction, full-grid iteration and merged-run construction.
- **Paint:** canvas input registration, layer setup, glyph lookup/shaping and primitive submission. These are CPU wall times, not GPU execution times.
- **Internal total:** render start **after synthetic generation and VT parsing** through canvas paint end, including intervening GPUI work. Percentiles are independent; do not add percentile columns.
- **GPUI frame:** pinned `crates/gpui/src/window.rs` around `window.draw`, `window.present` and arena cleanup. Different scope; not photon latency or guaranteed GPU completion. Synthetic workload work can contribute to that framework interval. Values are not interchangeable with internal totals.

Static deliberately forces redraw; it is not an idle CPU measurement. Typing seeds a styled screen and changes one bottom-row cell per frame; scrolling replaces it over time with shorter, lower-density log lines. Those medians are not comparable full-damage workloads.

**Interpretation:** the spike rebuilds and repaints every row even for a one-cell change (`gui/src/draw.rs`, `Builder::build`). It has no dirty-row/shaped-row cache as required by M1 §7.3; the draw-list builder also repeatedly scans growing strings while merging runs. Glyph lookup and per-glyph timing add instrumentation overhead. [INFERENCE] Damage tracking and eliminating repeated work could improve incremental frames, but no optimized implementation was measured. Damage tracking alone does not prove the required **full redraw** can pass. This is a failure of this spike renderer against the gate, **not proof that GPUI cannot do it**.

## Round 1: real 50 MiB output and idle CPU

The driver regenerates `/tmp/shoal-t00-50mb.log` each time: exactly **52,428,800 bytes**, **369,216 newline-terminated lines**, **128 trailing bytes**. Each full line is the fixed ASCII prefix `2026-10-25T00:00:00Z INFO request=123456 path=/api/build status=200 elapsed_ms=2 `, sixty `x` bytes, then LF. This differs from the previous session's file, so its earlier cat times are not pooled with these runs.

Each fresh GUI runs `bash -c` with the command recorded in its log. Bash blocks on a start FIFO until native-scale geometry is confirmed, records `$EPOCHREALTIME` immediately around `cat`, reports cat's exit code through a completion FIFO, then blocks on a stop FIFO. The GUI's PTY reader and foreground feed remain active. The measured cat time is **writer completion**, not time to the last presented frame.

`SPIKE_STATS=1` and `ZED_MEASUREMENTS=1` are enabled. Internal statistics reset on the first actual PTY output, excluding empty startup/configure renders. All three report **52,798,016 bytes consumed**: file bytes plus one extra CR for each LF under PTY output processing. A final settle/idle interval precedes shell exit and statistics emission. Per-canvas `SPIKE_FRAME` records include wall-clock microseconds, consumed-byte count, paint and total duration; `summary` compares their end timestamps with Bash's start/end timestamps. These count CPU canvas paints, not GPU-completed or presented frames.

| Run | Cat s | Exit | Paints ending while cat writes | All post-output paints | GPUI records, whole launch |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 0.729216 | 0 | 14 | 15 | 18 |
| 2 | 0.771469 | 0 | 8 | 9 | 12 |
| 3 | 0.777571 | 0 | 5 | 6 | 9 |

The following internal distributions cover first PTY output through shell exit, including the final paint after cat completes. Strict during-writing samples are preserved separately in `summary.json` (`while_cat_writes`).

| Run | Build p50 ms | Paint p50 ms | Total p50 ms | Total max ms | GPUI p50 ms | GPUI max ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1.38 | 2.14 | 3.55 | 21.15 | 4.55 | 24.94 |
| 2 | 1.41 | 2.17 | 3.59 | 21.99 | 4.67 | 25.74 |
| 3 | 1.57 | 2.43 | 4.04 | 23.04 | 5.01 | 27.71 |

Raw evidence: `cat-{1,2,3}.log`, `cat-{1,2,3}-time.txt`, and window JSON. The GPUI timer covers the whole launch, including startup; **its counts/distribution are not the same sample population** as the post-output internal statistics. Only 5–14 paints ended during each roughly 0.7 s stream, and cold glyph/cache work produced 21–23 ms internal spikes. The foreground receive loop drains ready batches without an explicit scheduling yield; [INFERENCE] it can monopolize the UI during a fast stream. These runs prove completion, byte ingestion and a final paint with all bytes consumed, **not smooth stream rendering, an FPS target, or a 4 ms frame pass**. No frame counts were inflated with synthetic redraws. Earlier instrumentation-only runs produced as few as one post-output paint; they are not pooled with the phase-timestamped final runs reported here.

After run 1, with the same 200×60 grid/window and the shell blocked on the FIFO:

```sh
pidstat -u -p "$gui_pid" 1 30
```

Raw `idle-30s.txt`: 30 one-second samples, average **0.03% CPU**, **0.03% usr**, **0.00% sys**, **0.00% guest**, **0.00% wait**. Percentages are relative to one logical CPU, not divided by 16. First sample is 1.00% CPU while final work settles; the following 29 samples are 0.00%. `/proc/<pid>/status` reports **34 threads**, process state **S (sleeping)**. `idle-process-status.txt` and `idle-threads.json` preserve process/thread evidence. Only the GUI process and its threads are sampled; no daemon exists yet and the separate FIFO-waiting shell is not included.

## Round 2: bounded GPUI renderer experiment

Approved by the user on 2026-10-25 (see "User decisions"). Spike only; same GPUI, Ghostty and libghostty-rs pins, same driver geometry (1800 × 1400 logical, scale 2, 14 px, 200 × 60, `fits=true`), same 30 warm-ups + 600 reported frames. Evidence: `spike/measurements/round2/`. Round-1 sources and binary are preserved as `spike/measurements/round1-sources/` and `spike/target/round1/spike-gui` (hash matches round 1's `measured-sources-sha256.json`).

### What changed

- **Dirty-row cache** (`gui/src/draw.rs`). libghostty's render-state dirty flags (`next_dirty`, then `clean`) pick the rows to rebuild; clean rows reuse their cached draw-list entry. Each rebuilt row gets a new version number.
- **Cheaper row build.** Raw cells are read in bulk (`cells_raw`). Style and colors are queried only when the style id changes within a row. Content-tag and style-id reads are skipped when the row's grapheme and styled flags make them redundant. Run merging tracks length instead of rescanning text.
- **Per-row paint cache** (`gui/src/paint.rs`). Resolved glyphs, or shaped lines in shape mode, plus background and underline quads, are cached by row version. Glyph lookup uses an ASCII table. The per-glyph timers from round 1 are gone.
- **Feed scheduling** (`gui/src/main.rs`). Parse at most 4 ms, request a frame, then yield once. GPUI runs foreground tasks as idle callbacks after pending events, so yielding lets a due frame and input in during a flood. Round 1 never yielded while bytes were ready.
- **Scroll accumulator** (`gui/src/view.rs`). Fractional wheel and touchpad deltas carry over. Round 1 rounded each event, so small touchpad deltas did nothing. This is a candidate cause of the "scrolling isn't ideal" observation, not a confirmed one.

### Method changes and verification

- **Benchmark output moved outside every timer.** It's now written in a GPUI next-frame callback, which runs before GPUI's `frame duration` timer. Round 2's GPUI numbers cover only draw and present. Round 1's GPUI numbers included about 1.2 ms (`balanced`) of output generation and VT parsing per styled frame, reported separately as "bench output + VT parse (untimed)".
- **Correctness oracle.** `SPIKE_VERIFY_ROWS=1` rebuilds every row from the same render-state snapshot with round 1's unchanged row loop (`gui/src/draw_ref.rs`) and compares it with the cache every frame. Results:
  - All five benchmark modes, 231 frames each: **0** mismatched rows of 13,860 per mode.
  - Three verified `cat` runs, 168–172 frames each: **0** of 30,720 rows.
  - A scripted VT run with background-only cells, inverse, wide characters, graphemes, alternate screen, scroll regions, palette and default-color changes, insert/delete line, erase and DECALN: **0** of 1,056 rows.

  Evidence: `spike/measurements/round2-verify/`, including the script `vt-exercise.sh`. Run it with `SPIKE_GRID=80x24 SPIKE_VERIFY_ROWS=1 SPIKE_STATS=1 SPIKE_CMD='bash spike/measurements/round2-verify/vt-exercise.sh' ./spike/target/release/spike-gui` from the repository root. The VT script paces itself with `sleep`, which only creates frames between steps; it isn't used for synchronization. Not covered: viewport scrolling through scrollback, resize and selection.
- **Oracle bug found on the way.** The first oracle used a second `RenderState` on the same terminal and reported false mismatches. Each `RenderState::update` consumes the terminal's own dirty rows, so a second render state goes stale. **For M1: one render state per terminal.**
- **Power state.** The laptop was on battery, and the user chose the `balanced` profile (EPP `balance_power`) for the final runs. `power.jsonl` in each folder records AC status, profile, governor and EPP before and after every command. Round 1 recorded none of this.

### Results: battery, `balanced`, same session

Round 2 against the round-1 binary, both run in this state (`round2/`, `round2-round1-binary/`). GPUI frame = draw + present CPU time, not GPU completion or photon latency.

| Workload | R2 internal p50 / p95 ms | R2 GPUI p50 / p95 / p99 ms | R1 binary internal p50 / p95 ms | R1 binary GPUI p50 / p95 ms |
| --- | ---: | ---: | ---: | ---: |
| **Full-damage styled, cached glyphs (gate)** | **1.84 / 2.34** | **2.64 / 3.19 / 3.44** | 3.01 / 3.56 | 5.14 / 5.82 (incl. ~1.2 ms output) |
| Static (clean frames) | 1.24 / 1.54 | 2.09 / 2.54 / 2.73 | 3.58 / 4.48 | 4.51 / 5.73 |
| Typing, one cell per frame | 1.33 / 1.65 | 2.21 / 2.70 / 2.91 | 3.62 / 4.08 | 4.55 / 5.07 |
| Scroll, one log line per frame | 1.48 / 1.83 | 2.12 / 2.56 / 2.73 | 3.18 / 3.63 | 3.89 / 4.40 |
| Full-damage styled, shaped runs | 3.50 / 4.07 | **4.32 / 4.95 / 5.21** | 3.44 / 3.93 | 5.66 / 6.19 |

The gate case fully redraws every frame: all 60 rows rebuilt and re-resolved, 3,990 glyphs. Round-2 p50 split: build **0.86 ms** (render-state update 0.11), resolve **0.15 ms**, paint **0.75 ms**. GPUI's own max was 4.11 ms in one of 600 frames. Typing rebuilds one row per frame; static rebuilds none but still repaints every glyph.

- **Round 1's numbers were probably taken under `power-saver`.** Earlier today, with `power-saver` observed right after the runs (not recorded per run), the round-1 binary reproduced round 1 within a few percent: internal 4.92 vs 5.02 ms, GPUI 7.71 vs 7.42 ms (`round2-powersaver-round1-binary/`). Under `balanced` the same binary gets 3.01 ms internal. **The redraw pass is shown for `balanced` only.** The final round-2 build has not been measured under `power-saver`. An intermediate round-2 build, before the cheaper row build, measured internal 3.89 ms and GPUI 5.09 ms there; it is not used as evidence.
- **Per-run shaping misses 4 ms on full damage.** M1 spec §7.3 says text is "shaped per style run", with clean rows reusing cached shaped lines. On a full redraw nothing is clean. The passing path places cached glyphs per cell and shapes only multi-codepoint graphemes, so it has no ligatures. That's a spec question for the user, not settled here.
- **Where the build time went.** `rowbench` (`spike/gui/src/bin/rowbench.rs`, `round2/rowbench.txt`) shows each libghostty cell accessor is an FFI call costing about 9 ns under `balanced` and 12–15 ns under `power-saver`. Bulk raw reads and skipping redundant accessors took the full-damage build from 2.10 ms to 1.09 ms under `power-saver`, and to 0.65 ms headless under `balanced`.

### Round 2: 50 MiB `cat` and idle CPU

Same driver, regenerated file, gated start, `SPIKE_STATS=1`; battery, `balanced`.

| Run | R2 `cat` s | R2 paints while writing | R2 `vt_write` total | R1 binary `cat` s, same state | R1 binary paints while writing |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 1.079 | 64 | 778 ms | 0.939 | 7 |
| 2 | 1.262 | 75 | 898 ms | 0.934 | 1 |
| 3 | 1.249 | 74 | 898 ms | 0.955 | 3 |

- **Completeness and idle.** All runs exit 0 and consume 52,798,016 PTY bytes. Idle after run 1: **0.00%** average CPU over 30 one-second samples (`round2/idle-30s.txt`), 34 threads, state S.
- **Trade-off.** Round 2 now paints at about vsync rate (~60 fps) during the flood, while round 1 barely painted. On one thread, those 64–75 full redraws cost about 0.3 s of `cat` throughput: **+15–35 %** against the round-1 binary in the same state.
- **Feed internals.** Inference: parsing didn't get slower. Round 2's `vt_write` totals (0.78–0.90 s) fit inside the round-1 binary's whole `cat` time, and the extra time matches the redraws. Merging chunks didn't change `vt_write` time. A slice averages about 0.3 ms of parsing (`vt_write` total ÷ slices) and ends when the channel is briefly empty, not at the 4 ms budget.
- **Flood cap didn't apply.** `SPIKE_FLOOD_FRAME_MS=33` only acts on slices that use their whole budget, so its runs (`round2-flood33/`, 1.260–1.277 s, 75–76 paints) match the uncapped ones.
- **M1 implication.** In M1 the daemon parses on the pane thread and windows never slow the shell (§5.5 backpressure), so this throughput cost moves off the shell's path. The window's flood frame rate is still a design choice for M1.
- **Cold glyphs.** The first frame with new glyphs still costs 13–15 ms to resolve; internal max 21–23 ms.

### Round 2 limitations

- One serial run per mode, no load isolation. Battery state of charge falls during the session.
- The oversized surface extends off-screen, as in round 1.
- No GPU-completion or presentation-latency timers.
- For the final build, `power-saver` and AC power are unmeasured on the laptop. These numbers are evidence for the laptop only. Development has since moved to the desktop, which has its own measurements ("Second machine"); the M1 budgets get measured there.
- Edits after the timed runs were comments only; `round2/measured-sources-sha256.json` records the sources. Later, 94f8d92 ran `cargo fmt` over the active sources. That changed whitespace, trailing commas and one match arm's braces, so their hashes no longer match the recorded ones. `measurements/fmt-sources-sha256.json` maps each recorded hash to its new one. The measured binaries came from the recorded sources, and the archived `round1-sources/` are untouched.

## Snapshot fidelity

`./spike/target/release/snapcheck --programs --verbose` was rerun unchanged on the laptop: **25 synthetic scenarios + six real-program rows**, none removed. The table below is that laptop run. The desktop rerun is in "Second machine"; it differs only in the `less -R` row. It prints mismatches but exits 0; exit status alone is **not** a correctness assertion.

For each case, A is the original terminal; B replays the formatter dump; C decodes `encode_snapshot_alloc` via `Decoder`; D replays formatter plus the parser's continuation bytes. Each then receives the same follow-up output. Synthetic fixtures are 80×24; real PTYs are 120×40. Scrollback cap is 10,000 lines with no byte cap and 4096-byte continuation tracking on the source.

All snapshot-relevant formatter extras exposed by the pinned Rust API were enabled: palette, modes, scrolling region, tabstops, pwd, keyboard, cursor, style, hyperlink, protection, Kitty keyboard, charsets; VT output, `unwrap=false`, `trim=false`, no selection restriction. No failing comparison was weakened.

The comparison checks dimensions, active screen, total/scrollback row counts, all overlapping cells of the active screen/history (bottom aligned), text, width, styles, hyperlink URI, protection, wrapped-row and prompt metadata; cursor position/wrap/visibility/SGR pen/at-prompt; 42 mode flags, current Kitty flags, mouse tracking/shape, title, pwd, OSC 10/11/12 colors and palette. Row metadata mismatches are counted once per cell, so a large cell count does not necessarily mean that many changed glyphs.

| Scenario | Formatter restore | Formatter after follow-up | Formatter + continuation after | Binary restore / after |
| --- | --- | --- | --- | --- |
| Text, styles, wide chars, 300-line scrollback | FAIL: 160 cells (wrap metadata) | FAIL: 160 cells | FAIL: 160 cells | Match / Match |
| Cursor position, hidden, pen style | Match | Match | Match | Match / Match |
| Cursor shape (DECSCUSR bar) | Match* | Match* | Match* | Match* / Match* |
| Pending wrap at last column | Match | Match | Match | Match / Match |
| Input modes: DECCKM, keypad, paste, focus, mouse | Match | Match | Match | Match / Match |
| X10 / any-event mouse, UTF-8 encoding | Match | Match | Match | Match / Match |
| Insert, no-wrap, reverse video, LNM | Match | Match | Match | Match / Match |
| DEC 2026, 2027, 2031, 2048, 12, 45 | Match | Match | Match | Match / Match |
| Scroll region + origin mode | Match | Match | Match | Match / Match |
| Left/right margins (DECSLRM) | Match | Match | Match | Match / Match |
| Custom tab stops | Match | Match | Match | Match / Match |
| DEC graphics in G0 | Match | Match | Match | Match / Match |
| G1 graphics shifted in (SO) | Match | Match | Match | Match / Match |
| Kitty keyboard flags | Match* | Match* | Match* | Match* / Match* |
| modifyOtherKeys | Match* | Match* | Match* | Match* / Match* |
| Hyperlinks, closed and open | Match | Match | Match | Match / Match |
| Palette + OSC 10/11/12 colors | FAIL: fg/bg/cursor colors | Same failure | Same failure | Match / Match |
| OSC 2 title + OSC 7 pwd | FAIL: title; pwd matches | Same failure | Same failure | Match / Match |
| Saved cursor (DECSC) | Match | FAIL: 2 cells, cursor x/y/pen | Same failure | Match / Match |
| Alt screen 1049 over scrollback | Match | FAIL: 252 cells, cursor x/y, row counts | Same failure | Match / Match |
| Alt screen 47 | Match | FAIL: 8 cells | Same failure | Match / Match |
| OSC 133 prompt marks | FAIL: 160 cells, at-prompt | Same failure | Same failure | Match / Match |
| DECSCA protected chars | FAIL: 4 cells | FAIL: 4 cells | FAIL: 4 cells | Match / Match |
| Split inside CSI | Match | FAIL: 5 cells, cursor x | Match | Match / Match |
| Split inside UTF-8 | Match | FAIL: 1 cell | Match | Match / Match |
| nvim --clean, syntax, then quit | Match | FAIL: 117 cells, cursor y, row counts | Same failure | Match / Match |
| less -R, then quit | Match | Match | Match | Match / Match |
| btop, then quit | Match | FAIL: 94 cells, cursor y, row counts | Same failure | Match / Match |
| fzf inline, then cancel | Match | Match | Match | Match / Match |
| lazygit, then quit | Match | FAIL: 100 cells, cursor y, row counts | Same failure | Match / Match |
| Interactive bash, prompt marks | FAIL: 3000 cells, at-prompt | FAIL: 3120 cells | FAIL: 3120 cells | Match / Match |

**Match means only the checked observations match, not a complete-state guarantee.** In particular:

- `*` Cursor presentation shape is not compared: the harness's `cursor_style()` is SGR pen style, not DECSCUSR shape. No keyboard/mouse encoder-output comparison, modifyOtherKeys behavior, or Kitty push/pop-stack test was made.
- Inactive screens, saved cursor, margins, tab stops, charsets and parser state are not all directly inspected; targeted follow-up exposes some losses. No arbitrary resize/reflow, graphics, long grapheme/URI, corruption, multi-attach race or cross-revision tests were run. Fixed 32-codepoint/4096-byte cell buffers and error-to-default conversions limit the oracle.
- Real programs use quiet-time draining, not semantic readiness/exit assertions. All six expected rows appeared and the programs are installed, but shell-mediated launches and absent exit-status checks limit claims of successful application lifecycle coverage. The earlier session reported a less-after-quit failure; this unchanged rerun matched that row. Primary-screen loss is still independently reproduced by synthetic alt-screen cases and other TUIs; the less row does not negate it.

### Precisely what the formatter misses

Verified against the **pinned source**, not guessed APIs. Relevant copies, with original line numbers, are preserved as `spike/measurements/snapcheck-pinned-*`; detailed citations and limitations are in `snapshot-analysis.md` on the spike branch.

| Missing tested state | Pinned source / observed boundary |
| --- | --- |
| Soft-wrap row metadata | `formatter.zig:1347–1349` inserts newlines with `unwrap=false`; observed wrapped true→false. Another unwrap policy was not measured and is not claimed to solve the whole snapshot problem. |
| OSC 10/11/12 dynamic foreground/background/cursor colors | Rust `fmt.rs:35–163` exposes no such options. Palette emission is OSC 4 (`formatter.zig:363–373`). Internal Zig generic/page formatting has some fg/bg support, but the pinned C terminal options do not expose it (`c/formatter.zig:143–148`); that does not establish a current terminal-formatter solution. |
| OSC 2 title | No title extra in the pinned Rust/C terminal formatter options; pwd has its own extra and survives. |
| Saved cursor position and SGR pen | `formatter.zig:753–758` explicitly states the saved cursor is not emitted. DECRC follow-up reveals the loss. |
| Inactive primary screen and scrollback while alternate screen is active | Rust `fmt.rs:165–176` and Zig `formatter.zig:237–245,302–305` document active-screen-only formatting. Active-screen mode bits survive, but do not carry the screen beneath it. Later exit loses content/history. |
| OSC 133 semantic prompt/at-prompt metadata | No corresponding extra; row metadata/at-prompt comparisons fail. |
| Historical per-cell DECSCA protection | Existing protection extra emits the **current cursor's** protection (`formatter.zig:802–809`), not the protection of previously written cells; selective erase exposes it. |
| Unfinished CSI / partial UTF-8 parser state | No formatter parser-state option. Separate continuation bytes repair the two tested split cases, but leave the other failures. |

The spec's **explicit mode-list fallback is insufficient for these findings**. The formatter already emits every internal mode differing from defaults (`formatter.zig:409–425`), and this run reports no explicit mode-flag mismatches. A boolean list cannot reconstruct primary cells/history under alternate screen, title, dynamic colors, saved cursor, semantic/protected-cell metadata or parser bytes. No proposed list is presented as a lossless fix.

For audit only, the harness compares ANSI modes **2,4,12,20** and DEC private modes **1,3,4,5,6,7,8,9,12,25,40,45,47,66,69,1000,1002,1003,1004,1005,1006,1007,1015,1016,1035,1036,1039,1045,1047,1048,1049,2004,2026,2027,2031,2033,2048,5522**. That is a comparison inventory, **not an approved attach contract**; replaying screen/cursor transition modes blindly can itself mutate state.

### Snapshot costs

`snapcheck --bench`: 200×60 terminal, 10,000 styled input lines, **10,001 total rows**; no scrollback byte limit. Feeding 1,043 KiB took 41.9 ms. Three warm serial iterations; KiB sizes are integer-truncated. These costs include the harness's allocations/copies and do not measure transport, drawing, peak RSS, or a latency distribution.

| Iteration | Formatter dump ms | Formatter restore ms | Binary encode ms | Binary decode ms |
| --- | ---: | ---: | ---: | ---: |
| 1 | 26.5 | 33.1 | 7.9 | 19.6 |
| 2 | 21.1 | 28.2 | 8.4 | 19.3 |
| 3 | 20.2 | 23.0 | 8.0 | 15.7 |

Formatter output **1,224 KiB**; binary **2,801 KiB**; both restored 10,001 rows in this benchmark. The separately printed `tracking=false/true` pair is **not a valid tracking comparison**: `new_terminal` already enables continuation tracking before either branch. It is not used to infer speedups. Benchmark row-count equality alone is not fidelity evidence; the scenario comparisons above supply that evidence.

Binary snapshot is the only tested path that matched all checked scenarios, including subsequent output. However, pinned `snapshot.rs:76–77` explicitly calls **format v1 work in progress with no binary-compatibility guarantee**. It is not a stable cross-version wire format merely because its version number is 1. Encoding partial VT/UTF-8 needs continuation tracking enabled before the relevant input; decoded terminals default to future tracking disabled (`snapshot.rs:101–111,276–289`). A later attach implementation would need exact-pin compatibility rules, bounded/validated decoding, callback rebinding, and a sequenced atomic snapshot/output boundary. None of that integration is implemented or claimed proven here.

## User decisions after round 1, 2026-10-25

**Historical:** these led to round 2 and the draft. The later decisions are in "Decisions recorded in the spec".

1. **Renderer: bounded GPUI experiment approved.** GPUI stays. In the spike only: dirty/shaped-row reuse, removal of repeated full-grid work, and deliberate feed scheduling; then remeasure incremental **and** full-redraw costs with the same method. The 4 ms target is not relaxed. If the full redraw still misses, the renderer decision returns to the user. No spec change.
2. **Attach snapshot: exact-pin binary snapshot approved in principle.** Live attach would carry libghostty's binary snapshot, with daemon and window required to run the identical libghostty build (a mismatch is handled like a `hello` protocol mismatch), and bounded, validated decoding. Reboot persistence stays on formatter output with its documented losses for now, as a separate later decision. The agent drafts the spec text for the user's review; nothing in `docs/spec/` is changed or committed without that review.

Round 2 above is the renderer experiment; the spec draft below is the snapshot follow-up. The original decision request is kept after the draft.

## Decisions recorded in the spec, 2026-10-25

After round 2, the user approved these. They are now in `docs/spec/`:

1. **Keep GPUI** (decisions.md, decision 3).
2. **Rendering** (M1 spec §7.3): cached per-cell glyphs, no ligatures across cells, multi-codepoint grapheme clusters shaped as units, positions and widths from libghostty's cells. Dirty-row reuse stays, with one render state per terminal. Font family, size or line-height changes clear the glyph and row caches.
3. **Live attach** (§5.1, §5.5): libghostty binary snapshots, between daemon and window at the exact same core build. Protocol stays 1 because nothing implements it yet.
4. **`hello` core identity** (§5.3): built from the full pinned revisions plus the build settings that change terminal behaviour, compared byte for byte. A mismatch blocks attaching and offers a user-triggered restart. The restart stops the daemon through its persisting shutdown (`daemon.shutdown`, or `systemctl --user stop` or `SIGTERM` across a protocol mismatch), waits up to 5 s, never sends `SIGKILL`, then starts and restores.
5. **Reboot persistence keeps formatter output** (§6.3), but still saves the main screen and history under an alternate-screen program. In that case it formats a decoded scratch copy of a binary snapshot, never the live terminal.

`tasks.md` follows: T07, T11, T13, T15, T22, T41, T46, T48 and T50 changed; T07 adds the pin-upgrade rule.

## Review corrections, applied to the spec

The final review of the round-2 draft found four problems. Each is fixed in `docs/spec/` and, where it could be, checked on the desktop with `snapcheck --design` (`desktop-2026-10-25/snapcheck-design-*.log`).

1. **A 4096-byte continuation cap doesn't guarantee a snapshot.** Reproduced: with a 5,000-byte unfinished OSC 2, `encode_snapshot_alloc` and the continuation query both return `InvalidValue`, and `is_vt_ground` is false. A 4,000-byte control encodes normally. Given the next chunk, `vt_write_until_ground` stops right after the terminator (3 bytes for `yy`+BEL, 4 for `yy`+ST). A snapshot taken at that point decodes to a terminal equal to the source, before and after the rest of the chunk. Output with no ground point is consumed whole. **Spec (§5.5, "Mid-sequence snapshots"):** the waiting attach is served at the first ground point by splitting that read into two numbered items, so nothing is lost or duplicated and the shell is never blocked. Waiting has a 2 s deadline, then error 1002 `mid_sequence` with a user-triggered Retry. Resync uses the same rule and `pane.detached`. No polling, and no timer while idle.
2. **The 64 MiB frame limit doesn't bound decoding.** **Spec (§5.1, §5.5, "Decoding limits"):** the header and decoded size are checked (1–1,000 each, and they must match); the accepted continuation is capped at 4096 bytes (libghostty's default is 65 MiB); decoding allocations go through a counting allocator capped at 256 MiB; declared history is at most 100,000 rows per screen (and `scrollback.lines` is capped to match, §9.1); and the whole snapshot must be consumed. That last check needs care: on the desktop, one-shot `decode` **accepted 4 trailing bytes silently**. A reader-based decoder read exactly the snapshot (20,892 of 20,896 bytes; 714,283 of 714,287 for a 200 × 60 terminal with 42 history pages). So the spec has the client count the bytes taken. A truncated snapshot returned `InvalidValue`. The allocation cap and peak decode memory are not measured yet; T07 measures them.
3. **Reply ownership on decoded mirrors.** Callbacks aren't in a snapshot, so a decoded terminal starts with none. Without `on_pty_write`, libghostty drops replies. **Spec (§4.2, §5.5, "Mirror callbacks"):** only the pane's terminal in the daemon has reply callbacks. Mirrors and scratch terminals install none of the reply or metadata callbacks, only drawing callbacks such as `on_render_hold`, and finish setup before the first live byte, because the restored parser state can complete a query on that byte. Not run as a separate test here; T07 adds "a restored mirror sends no reply".
4. **The 4 ms target stays a per-frame target.** The checklist above quotes the laptop's full distribution: one of 600 frames at 4.11 ms. It does not restate the gate as a percentile.

The checks also corrected the first version of the new §6.3 text. `CSI ? 1049 l` alone does **not** leave an alternate screen entered with mode 47 or 1047: the scratch terminal stayed on it with no history. Sending `CSI ? 1049 l`, `CSI ? 1047 l`, `CSI ? 47 l` restored the main screen and its 77 history rows for all three modes, and the live terminal stayed on its alternate screen. §6.3 and T07 say so. After 47 and 1047 only the cursor differs, which doesn't matter for inert reboot restore.

## Second machine: desktop, 2026-10-25

Development moved here mid-day. Raw evidence: `spike/measurements/desktop-2026-10-25/` on `spike/t00`, with `environment.txt` and each folder's `power.jsonl`. **Different hardware, scale and power from the laptop. Don't pool or ratio the two.**

| Item | Desktop |
| --- | --- |
| Hardware | Dell Inspiron 3671, Intel Core i5-9400 (6 cores, no SMT), 11 GiB RAM |
| GPU | Intel UHD Graphics 630 (CoffeeLake-S GT2), Mesa 26.2.5 `vulkan-intel` (ANV); `vulkaninfo` not installed, so adapter selection wasn't captured |
| OS / compositor | Omarchy 4.0.6, Linux 7.2.8-9-omarchy; Hyprland 0.56.2, same commit as the laptop |
| Monitor | `HDMI-A-1`, Samsung C32F391, **1920 × 1080 @ 60 Hz, scale 1**, top reserved 26 px |
| Power | Desktop on mains: profile `performance`, governor `powersave`, EPP `performance`. sysfs lists only a wireless mouse battery, so `power_supply_online` shows that, not mains |
| Toolchain | Rust 1.98.1 via `spike/mise.toml`, Zig 0.17.0, mise 2026.10.17 |
| Binaries | Fresh-clone builds: GUI from 94f8d92 (same hash at 09c6967); snapcheck from 09c6967 (`49ecfcca…`, adds `--design`) |

**Frames** (`frames/`, `measure.py frames --scale 1`). Same driver, grid, font, 30 warm-ups and 600 reported frames as round 2. GPUI reported viewport 1800 × 1400, scale 1, cell 8.400 × 21.000, 200 × 60, `fits=true`. The 1400 px window is taller than the 1080 px monitor, so part of it is off-screen, as on the laptop. At scale 1 the surface has a quarter of the laptop's device pixels.

| Workload | Internal p50 / p95 / max ms | GPUI p50 / p95 / p99 / max ms | GPUI frames over 4 ms |
| --- | ---: | ---: | ---: |
| **Full-damage styled, cached glyphs (gate)** | **1.40 / 1.53 / 1.92** | **2.24 / 2.48 / 2.82 / 3.13** | **0 of 600** |
| Static (clean frames) | 0.58 / 0.64 / 0.98 | 1.42 / 1.68 / 1.88 / 3.44 | 0 |
| Typing, one cell per frame | 0.60 / 0.68 / 0.92 | 1.46 / 1.72 / 1.91 / 1.99 | 0 |
| Scroll, one log line per frame | 0.80 / 0.94 / 1.88 | 1.42 / 1.64 / 1.86 / 3.37 | 0 |
| Full-damage styled, shaped runs | 2.84 / 3.18 / 3.84 | 3.70 / 4.15 / 4.50 / 4.76 | 55 |

**`cat` 50 MiB and idle** (`cat-idle/`). 0.493 / 0.480 / 0.496 s; all exit 0 and consume 52,798,016 PTY bytes; 27 / 26 / 28 paints while `cat` wrote. After run 1, idle CPU over 30 one-second samples averaged **0.00%**, with 21 threads in state S.

**Snapshots** (`snapcheck-programs-*.log`). All 31 rows ran. Binary snapshot **31/31** at restore and **31/31** after follow-up. Formatter 25/31 at restore, 16/31 after, 18/31 with continuation. Against the laptop's final run, only `less -R, then quit` differs: here it fails after follow-up (94 cells, cursor y, row counts), as it did in an earlier laptop session. `snapcheck-bench-*.log`: formatter dump 16.0–16.6 ms plus restore 13.0–13.1 ms (1,224 KiB); binary encode 4.4–4.8 ms plus decode 9.4–9.6 ms (2,801 KiB); 10,001 rows each.

**Not measured on the desktop:** `--verify` cache-oracle runs, `rowbench`, other power profiles, and the by-eye checks.

## T00 closure

- Every gate in the checklist has a result. The two that failed, the formatter and the round-1 redraw, are resolved by the user's decisions, which are now in the spec.
- The spike is archived on `spike/t00` and never merged. `main` gets only `docs/spike-00.md`, `mise.toml` and the `docs/spec/` edits, committed with explicit paths.
- **Optional, needs the user:** a by-eye look on the desktop, including the round-2 scrolling recheck (below).
- **Proposed follow-ups, not started:** T01 should pin Rust in `mise.toml` as well as `rust-toolchain.toml`, or document the `RUSTUP_TOOLCHAIN` override; T07 measures peak decode memory; T50 decides the Font Logos glyphs.

## Original spec draft: binary snapshot for live attach (superseded)

**Historical.** This was the draft as written after round 2. The user approved the direction, and the spec was then written with the corrections in "Review corrections, applied to the spec" above. Where they differ, `docs/spec/` is authoritative. In particular, item 2's "the frame is already limited to 64 MiB" does **not** bound decoding, and item 3's 4096-byte cap does **not** guarantee a snapshot is available. API names are from the pinned bindings (`snapshot.rs`, `terminal.rs` at `9ede35b8`).

**Why.** The formatter loses state that a live mirror needs (table above). The binary snapshot matched all 31 checked cases. It's also cheaper at the attach budget's size (10,000 lines, 200 × 60): round-1 encode 7.9–8.4 ms plus decode 15.7–19.6 ms, against formatter dump 20.2–26.5 ms plus restore 23.0–33.1 ms. The cost is 2.3× the bytes (2,801 vs 1,224 KiB) and no compatibility guarantee: the pinned docs call format v1 "a work in progress".

**Proposed edits:**

1. **m1-spec §5.5 step 2.** Replace "dump a snapshot with the formatter (screen, scrollback, styles, cursor, modes)" with: *"encode a binary snapshot with libghostty's snapshot module (`encode_snapshot`): screen, inactive screen, scrollback, styles, cursor and saved cursor, modes, colors, title, prompt marks, and unfinished parser state."*
2. **m1-spec §5.5 step 4.** *"The client builds a fresh mirror by decoding the snapshot (`snapshot::Decoder`). Decoding is bounded: the frame is already limited to 64 MiB; a decode error, or decoded dimensions other than `cols × rows`, drops the mirror and re-attaches once, then shows an error banner. The mirror installs its own reply handler, which discards replies, then feeds every item in order."* Option: `Decoder::ready` gives a renderable terminal before history finishes decoding. That could help the 50 ms attach budget; not measured.
3. **m1-spec §4.2, pane runtime.** *"The pane's terminal enables continuation tracking (`set_continuation_max_bytes(4096)`) when it is created, so snapshots taken mid-sequence carry partial escape and UTF-8 state."*
4. **m1-spec §5.1.** SNAPSHOT keeps its layout; `bytes` becomes "libghostty binary snapshot".
5. **m1-spec §5.3 hello.** Add a `core` field, for example `"core":"ghostty-521cf2c5/libghostty-rs-9ede35b8"`, to both `params` and `result`. A mismatch gets the same treatment as a `protocol` mismatch: the window offers to restart the daemon (decision 5). Daemon and window come from one build, so a mismatch means a daemon still running from before an upgrade.
6. **m1-spec §6, persistence, unchanged for now.** `scrollback.vt.zst` stays formatter output. Reboot restore is inert: it replays old text under a "restored" divider and starts a fresh shell. The losses above (title, dynamic colors, the screen under the alternate screen, prompt marks) are tolerable there. Per the user, a separate decision.
7. **decisions.md decision 1 and the Risks table.** "That gives reattach and reboot restore almost for free" becomes "the formatter gives reboot restore; live attach uses the binary snapshot at the exact pin". In the snapshot-fidelity risk row, the fallback becomes "exact-pin binary snapshot; a pin upgrade re-runs the snapshot fidelity suite", replacing the mode list, which this spike showed cannot carry the missing state.
8. **tasks.md.**
   - T07: replace `snapshot_vt` with `snapshot` (binary encode) and `restore` (decode), and keep a `format_vt` for persistence. Golden tests cover a binary round trip comparing cells and modes, including the split-CSI, split-UTF-8, alternate-screen and saved-cursor cases. A test shows truncated or corrupted snapshots return errors without panicking.
   - T13: unchanged apart from the wording.
   - A libghostty pin upgrade task must re-run the snapshot fidelity suite.

**Open points to settle in T07, not assumed:**

- Whether a decoded terminal keeps the snapshot's default colors or the window's theme defaults.
- How `on_pty_write` and other callbacks behave on a decoded terminal.
- Peak decode memory.

Protocol number: no implementation exists yet, so this draft keeps `protocol: 1`; changing it is the user's call. M4 (remote) inherits the exact-build requirement across machines.

## Round 1 decision request (answered 2026-10-25)

1. **Renderer:** retain GPUI for an explicitly approved, bounded renderer experiment (dirty-row/shaped-row reuse, removal of repeated full-grid work, and deliberate feed scheduling), then remeasure both incremental and **full-redraw** costs; **or** approve the spec's **winit + wgpu + glyphon** fallback. Recommendation: diagnose the renderer before attributing the miss to GPUI. Neither option gets a pass today; no target is relaxed.
2. **Attach snapshot:** approve a spec change to the tested **binary snapshot at an exact libghostty revision**, with a compatibility contract and a separate decision for inert reboot persistence; **or** retain replayable VT and explicitly design the additional complete state representation needed. A mode list alone cannot satisfy the current attach requirement. Recommendation: evaluate binary for live attach rather than pretending formatter + modes is faithful. This is not approval to change protocol/persistence formats now.
3. By-eye answers are recorded: translucency, crisp text and wallpaper blur confirmed; input method skipped by the user's confirmed non-use. Scrolling was reported as “isn't ideal”, so manual smoothness remains an issue. Do not ask the user to repeat settled appearance or input-method answers.

Do not update decision 3, the AGENTS lookup pins, or main commits before those decisions and the remaining gate results. If a passing resolution is approved and verified, archive the standalone spike with `mise.toml`, Cargo.lock and required font attribution **only on `spike/t00`**, never merge it. Put only the approved report, `mise.toml`, and any genuinely needed approved setup/spec updates on `main`; use explicit paths, never `git add -A` there. A fresh-checkout build still needs proof. **T01 follows T00 only once this gate is resolved, in a fresh session.**

## By-eye / input-method checks — user observations recorded

On the laptop, in the graphical session:

```sh
cd /home/phagmaier/Code/shoal/spike && ./target/release/spike-gui
```

Default requested size is **900×600**, font **14 px**, app_id **`dev.shoal.Spike`**, transparent background fill **0.88 opacity**. Hyprland tiling can override the requested size. Normal interaction has no forced benchmark redraw loop. User observations:

1. **Translucency and wallpaper blur confirmed:** initial answer “Yes it's translucent”; after the explicit question whether the wallpaper was actually blurred rather than merely visible, the user answered “As for your clarification yes.”
2. **Crisp text confirmed:** “Yes the text I would say is crisp.”
3. **Smoothness not fully satisfactory:** “Text is pretty smooth scrolling isn't ideal.” Record the scrolling issue as observed, without attributing a cause or treating the manual check as passed. No new measurement was run to reconfirm the user's observation.
4. **Skipped (no input method):** user confirms “No input method i type only in english and don't plan to type in anything else.” No IME functionality test is needed for this user's T00 gate; this is not a general claim that IME support works.
5. **Scrolling, optional recheck after round 2:** round 2 carries fractional wheel and touchpad deltas over instead of rounding each event, and draws during floods. Not yet re-checked by the user. Same command as above: run `seq 1 5000`, then scroll back with the touchpad and the mouse wheel.
6. **Desktop, optional:** observations 1–3 were made on the laptop at scale 2. Nobody has looked at the spike on the desktop (scale 1). Its Hyprland config has blur explicitly off (`decoration:blur:enabled` false, set), so wallpaper blur won't appear there until blur is enabled. From a fresh-clone build: `cd <clone>/spike && ./target/release/spike-gui`.

At measurement time blur was globally off (`hyprctl getoption decoration:blur:enabled` → false). If blur does not appear, this is a compositor configuration issue to check first, not a GPUI failure. For the installed Lua configuration, the user can opt into:

```lua
hl.config({ decoration = { blur = { enabled = true } } })
hl.window_rule({
    match = { class = "^dev[.]shoal[.]Spike$" },
    no_blur = false,
    opacity = "1.0 override 1.0 override",
})
```

The opacity rule removes additional compositor active/inactive dimming while retaining the application's 0.88-alpha background; it does not force the client opaque. For an exact floating by-eye size, optional rule fields are `float = true, size = {900, 600}`. Global blur affects other windows too: no such configuration was applied by the agent. The [0.56.2 example](https://github.com/hyprwm/Hyprland/blob/v0.56.2/example/hyprland.lua) documents `hl.config` blur configuration; [official window rules](https://github.com/hyprwm/hyprland-wiki/blob/main/content/configuring/core/rules/window-rules.md) document `no_blur` and opacity overrides. This suggested rule itself has not been applied or visually verified.

## Pinned primary sources

- [Bindings formatter options and active-screen contract](https://github.com/uzaaft/libghostty-rs/blob/9ede35b8ea94a55ed3608408d67e102883129f1e/crates/libghostty-vt/src/fmt.rs)
- [Bindings snapshot API and compatibility warning](https://github.com/uzaaft/libghostty-rs/blob/9ede35b8ea94a55ed3608408d67e102883129f1e/crates/libghostty-vt/src/snapshot.rs)
- [Bindings Ghostty pin and build logic](https://github.com/uzaaft/libghostty-rs/blob/9ede35b8ea94a55ed3608408d67e102883129f1e/crates/libghostty-vt-sys/build.rs)
- [Ghostty formatter implementation](https://github.com/ghostty-org/ghostty/blob/521cf2c5d30bd48028ca61546c64e7913764073d/src/terminal/formatter.zig)
- [Ghostty C formatter options](https://github.com/ghostty-org/ghostty/blob/521cf2c5d30bd48028ca61546c64e7913764073d/src/terminal/c/formatter.zig)
- [GPUI frame timer scope](https://github.com/zed-industries/zed/blob/8dd255c3eafb5231056fe4e6f601cedd011f1c93/crates/gpui/src/window.rs#L1884-L1903)

Raw artifacts and detailed implementation evidence remain on the spike worktree; they must not be swept onto `main`. This report carries the decision-relevant tables independently of that throwaway tree.
