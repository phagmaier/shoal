# Milestone 1 tasks

32 tasks in six phases, each sized for one agent session. The daemon is finished and tested end to end through the CLI before the window exists, so the riskiest UI work is off the critical path.

**Current status:** T01–T06 done and merged into `main` (2026-10-25), including T03's standalone review follow-up. T06 was merged and pushed with the user's approval after `just check` passed on the merged `main`. T07 (terminal wrapper) done on `t07-terminal-wrapper`, awaiting approval to merge; its GUI-facing half is now T08, next in the safe order.

**How to use this list**

- Hand an agent one task id plus `AGENTS.md`. Everything in the task's **Needs** must be done first.
- **Done when** is the definition of done: those checks pass in `just check`, or, for manual checks, the user has run them.
- If a task is bigger than one sitting, the agent stops, splits it, and writes the split back into this file.
- "M1 spec §N" means section N of `docs/spec/m1-spec.md`.
- Start each task in a fresh agent session. A prompt that works:

  > Do task T<NN> from docs/spec/tasks.md. Read AGENTS.md and the spec sections the task cites first. Check its Needs are done, work on a branch `t<NN>-<short-name>`, and stop when its Done when passes or when the spec is unclear.

**Order and parallelism**

```
Phase 0  Foundations      T00 → T01 → T02..T08
Phase 1  Daemon           T10..T16          (needs Phase 0)
Phase 2  Client and CLI   T20..T22          (needs Phase 1)
Phase 3  Shell + prompt   T31 after T06; T30 after T11   (runs alongside Phase 1)
Phase 4  Window           T40..T48          (needs Phase 2)
Phase 5  Ship             T50..T52          (needs everything)
```

A simple safe order: T00, T01, T02, T03, T04, T05, T06, T07, T08, T31, T10, T11, T30, T12, T13, T14, T15, T16, T20, T21, T22, T40, T41, T42, T43, T44, T45, T46, T47, T48, T50, T51, T52.

## Phase 0 · Foundations

### T00 · Spike: GPUI and libghostty on Hyprland

- **Status:** done 2026-10-25. `docs/spike-00.md` has the results. The spike stays on `spike/t00`. The snapshot and rendering decisions it led to are in M1 spec §5 and §7.3.
- **Needs:** nothing. Throwaway code on a `spike/t00` branch, never merged to `main`.
- **Build:** a GPUI window with a transparent background running `bash` through a PTY into a libghostty-vt terminal, drawn as a plain grid with the bundled symbols font as fallback. Plus a small harness that dumps a terminal with the formatter, restores it into a second terminal, and compares cells and modes.
- **Done when:** `docs/spike-00.md` (committed to `main`) records the go/no-go checklist from `decisions.md` ("Risks and the spike") with numbers (frame time at 200 × 60, `cat` of 50 MB, idle CPU, snapshot fidelity per mode, input-method result), the chosen GPUI and Ghostty commits, the Zig version, and exact build steps from a clean checkout. The user has confirmed the by-eye checks. `mise.toml` with the pinned Zig is committed to `main` with the doc. If any check fails, the doc records why and the agent stops for the user's decision: for GPUI it proposes the winit + wgpu fallback as a spec change; for snapshot fidelity it lists the missing modes and proposes the explicit mode list from `decisions.md`; for a missed number it records the gap and what might close it.

### T01 · Repository scaffold

- **Status:** done 2026-10-25 on `t01-scaffold`. Implementation, direct review and automated verification complete; the user reported success for install-mode setup plus `just check` in the prepared fresh clone. Merged into `main` and pushed.
- **Verified:** `just check` passes locally and in a fresh candidate clone after `mise install`, source fetching and setup `--check`. All 178 isolated dependency-policy probes pass. The actual pinned GPUI/platform graph is accepted in the GUI and rejected in the daemon; adding CLI→term makes `just check` fail at the deny gate. Shell syntax, repeated fetch, fetch integrity/failure paths, setup trust/missing-source failures and all three empty binary launches were exercised.
- **Review:** fixed symlink read-only validation, accounted for upstream GPUI platform imports without permitting other Shoal crates to import them, and kept the deny policy scoped to dependency boundaries. No unresolved in-scope findings.
- **Needs:** T00.
- **Build:** the Cargo workspace with all nine crates as empty skeletons (M1 spec §2); `rust-toolchain.toml` pinning a stable Rust with rustfmt and clippy; extend T00's `mise.toml` to also pin `just`, `cargo-deny`, `cargo-insta` and `cargo-fuzz`; a `justfile` with `check` (fmt, clippy with warnings as errors, tests, cargo-deny), `test`, `dev-daemon`, `dev-gui`, `fuzz-proto` (cargo-fuzz needs nightly, so this recipe alone uses `cargo +nightly fuzz`); `deny.toml` encoding the crate dependency rules; `third_party/fetch.sh`, a script that fetches read-only copies of the pinned GPUI source and Ghostty headers (fetched files gitignored), which `scripts/setup-machine.sh` runs; update `scripts/setup-machine.sh` for anything new the build needs; remove the bootstrap note from `AGENTS.md`.
- **Done when:** `just check` passes on a fresh clone after `scripts/setup-machine.sh`, and adding a forbidden dependency (e.g. gpui in `shoal-daemon`) makes it fail.

### T02 · Model types

- **Status:** done 2026-10-25; merged into `main` and pushed.
- **Verified:** `just check` passes. A reviewed insta snapshot of the wire JSON covers two sessions, nested splits and a zoomed pane; round-trip, spec-shape and wrong-id-kind tests; 13 `validate` rejection tests, one per broken invariant, each asserting the exact error.
- **Follow-up (2026-10-25, `t02-validate-rules`):** the user confirmed cross-kind id uniqueness and the pane-key check, and asked `validate` to also reject repeated session names and split ratios outside 0.1..=0.9 or NaN, before T03's property test relies on it. M1 spec §3 now lists all of these.
- **Needs:** T01.
- **Build:** `shoal-model` types from M1 spec §3 with serde, prefixed-string ids, and `validate`.
- **Done when:** insta JSON snapshot tests cover a workspace with two sessions, nested splits and a zoomed pane; `validate` rejects each broken invariant in its own test.

### T03 · Layout operations

- **Status:** done 2026-10-25 on `t03-layout-ops`; merged into `main` with user approval, including the standalone review follow-up. The user settled caller-supplied split ids, fractional resize steps, integer rounding and zoom/close edge cases; recorded in M1 spec §3 before implementation.
- **Verified:** `mise exec -- just check` passes (fmt, clippy with warnings denied, all 51 model tests, cargo-deny and dependency boundaries). Added 22 unit tests covering all seven operations, plus a proptest of random operation sequences that calls `validate` after every step, checks rejected edits are atomic, and independently checks pane/divider coverage pixel by pixel. Existing JSON snapshots remain unchanged.
- **Review:** direct review of implementation, tests, dependency/lockfile changes and spec contracts complete. The property test exposed a capped gap propagating incorrectly to child splits; a focused regression failed before the fix and passes afterward, and its generated seed is saved. No unresolved in-scope findings. No manual checks required for this pure-model task.
- **Standalone review follow-up (2026-10-25):** resolved the low-priority off-by-one floor for very large accepted pixel extents using exact scaling and integer division. Extended the existing large-area test with independently worked expectations at both ratio bounds on both axes; the regression failed before the fix and passes afterward. Direct review and a fresh `mise exec -- just check` pass, with all 51 model tests and no new dependencies.
- **Needs:** T02.
- **Build:** `split`, `close`, `rects`, `neighbor`, `resize`, `set_ratio`, `toggle_zoom` (M1 spec §3).
- **Done when:** unit tests for each function's edge cases, plus a proptest running random operation sequences that checks every invariant and that `rects` tiles the area exactly.

### T04 · Protocol crate

- **Status:** done 2026-10-25 on `t04-protocol`, starting from `main`; user-confirmed frame limit, JSON envelopes and scalar types recorded in M1 spec §5 before implementation. Merged into `main` and pushed with user approval; a fresh integration `mise exec -- just check` passes.
- **Verified:** `mise exec -- just check` passes, including formatting and clippy for the separate fuzz workspace. All 18 protocol tests and the existing 51 model tests pass. Literal JSON and binary fixtures cover every M1 method/result/notification, error codes and error data, truncation, unknown kinds, concatenation, snapshot header limits and the exact 64 MiB boundary; a proptest covers arbitrary binary ids, sequences and payloads.
- **Fuzz:** `mise exec -- just fuzz-proto -- -max_total_time=600 -timeout=5 -rss_limit_mb=2048 -max_len=65536 -print_final_stats=1` passes with AddressSanitizer: 13,632,246 inputs in 601 seconds, no findings. Versioned seeds and a separate committed lockfile make the target reproducible; deterministic tests cover the full frame-size boundary.
- **Review:** direct review of implementation, tests, fixtures, dependency/lockfile changes and spec contracts complete. Focused checks caught unknown-method misclassification, array-shaped empty parameters and errors, and nonfinite ratios. Fuzzing exposed a JSON float changing value on reparse; its saved fixture fails before enabling serde_json's `float_roundtrip` feature and passes afterward. No unresolved in-scope findings, manual checks or follow-up tasks.
- **Needs:** T02.
- **Build:** `shoal-proto`: frame encoder and decoder with the size limit, all JSON request, response and notification types for M1 spec §5, binary frame structs, error codes.
- **Done when:** round-trip tests for every message, decoder tests for truncated, oversized and unknown frames, and a `cargo-fuzz` target for the decoder that runs clean for 10 minutes.

### T05 · Config core and keybinds

- **Status:** done 2026-10-25 on `t05-config-core`, starting from `main` at `617e4dd`; merged into `main` and pushed under the user's standing authorization. User-confirmed validation, keybinding override semantics and the required path segment are recorded in M1 spec §9.1.
- **Verified:** `mise exec -- just check` passes (formatting, clippy with warnings denied, all 19 config tests plus 69 model/protocol tests, cargo-deny and dependency boundaries). Public config docs build with warnings denied. Literal fixtures independently cover every schema default and all 48 default bindings; tests cover partial files, exact file/line errors, unknown-key warnings, unbinding, invalid values, chord normalization and an arbitrary-text property test. XDG tests cover overrides, defaults, invalid roots and non-UTF-8 paths without mutating the process environment.
- **Watcher:** real temporary-file tests cover one reload for a burst of writes, ignored unrelated writes, atomic saves, last-good retention, invalid startup and recovery, deletion/recreation, missing/replaced parent directories, symlink target edits and retargeting. Watch installation precedes the initial read; idle workers block without timers. No real user config, state or socket was touched.
- **Review:** direct requirements, implementation, test-quality and dependency/lockfile review complete. Focused regressions failed before fixing opacity rounding at `1.00000001` and a duplicate normalized chord reporting the table header instead of its own line. No existing dependency versions were upgraded. No unresolved in-scope findings or manual checks remain; T06 stays unstarted.
- **Needs:** T01.
- **Build:** `shoal-config`: the schema with defaults from M1 spec §9.1, loading with line-numbered errors, unknown-key warnings, the chord parser, the default keymap from §10, a debounced file watcher, and the XDG path helpers.
- **Done when:** tests for defaults, partial files, bad values (error names file and line), every default chord parsing, `"none"` unbinding, and the watcher firing once for a burst of writes.

### T06 · Omarchy theme loading

- **Status:** done 2026-10-25 on `t06-omarchy-theme`, starting from `main` at `854559d`; merged into `main` and pushed with the user's approval; `just check` passes on the merged `main`. The user settled missing-key derivation (Omarchy's own fallbacks, then the built-in Tokyo Night palette), all Omarchy legacy aliases, the flat `shoal.toml` format and skip-and-warn bad values; they are recorded in M1 spec §9.2.
- **Verified:** `mise exec -- just check` passes (formatting, clippy with warnings denied, all 27 config tests plus the model and protocol suites, cargo-deny and dependency boundaries). Public config docs build with warnings denied. The dark and light fixtures are real Omarchy themes (osaka-jade, with uppercase hex, and lupine), chosen so most roles have distinct colors. The legacy fixture covers short names, `colorN`, `purple` and every derivation rule; its expected values were worked by hand and match Omarchy's own `omarchy-theme-color --all`. The `shoal.toml` fixture covers alias spellings beating `colors.toml` and a bad value falling through with its line. Other tests cover the built-in palette, Omarchy 4 beating a symlinked 3.x folder, `[theme.colors]` and `cursor.color`, file sources with relative, `~/` and absolute paths, a missing file, and invalid TOML or UTF-8. The watcher tests replay `omarchy-theme-set`: staging in `next-theme` causes no reload, the swap plus `theme.name` causes exactly one, and the later `background` repoint causes none. They also cover in-place edits, 3.x symlink repointing, unrelated theme edits and an Omarchy 4 folder appearing later. 15 repeated runs of both watcher suites passed.
- **Review:** direct review of requirements, implementation and tests. The config watcher's engine is now shared, with one deliberate change: a notify error triggers a debounced reread rather than an immediate report. A focused regression caught inotify reporting a symlinked folder's events under its target's path, so a missing file now also tracks its canonical spelling; the test failed before the fix and passes afterward. Mutation checks confirmed that the swap test fails when sibling events count, and the legacy fixture fails when a derivation order changes. No new dependencies, no manual checks, and no unresolved in-scope findings. Daemon and GUI use of the watcher (notices for a missing theme folder, recoloring) belongs to T1x, T41 and T48.
- **Needs:** T05.
- **Build:** the lookup order and mapping from M1 spec §9.2, both Omarchy locations, legacy key names, the theme-folder watcher.
- **Done when:** fixture themes (dark, light, legacy names, one with `shoal.toml`) map to exact expected palettes; a simulated theme swap (rename a staging folder into place, write `theme.name`) fires exactly one reload.

### T07 · Terminal wrapper

- **Needs:** T01 (pins from T00).
- **Status:** done 2026-10-25 on `t07-terminal-wrapper`, starting from `main` at `bb4dabc`; committed on the branch, awaiting the user's approval to merge and push. Split the same day: the GUI-facing wrappers (render-state access, key and mouse encoders) moved to T08. This task's Done when tests none of them, and their shape is best set alongside their consumers, T41 and T42.
- **Measured:** peak decode memory for 200 × 60 with 10,000 styled lines (a 3.5 MiB snapshot) is **15.6 MiB** of resident growth (`tests/memory.rs`). The counting allocator saw only 0.26 MiB, because Ghostty maps terminal pages straight from the OS. A 1000 × 60 snapshot with 99,000 lines is within every other limit but would need about 390 MiB. The user chose to add a declared-cell budget (32 Mi cells over both screens); it refuses that snapshot after 0.4 MiB. M1 spec §5.5 records this.
- **Verified:** `mise exec -- just check` passes (all 117 tests, including 21 new terminal tests, plus cargo-deny and boundaries). Public docs build with warnings denied. Goldens with hand-worked text cover writes, scrolling into history, reflow on resize (wider, narrower, hard breaks) and the alternate screen. Fifteen T00 snapshot cases round-trip at restore and after follow-up output, comparing VT captures of every cell, cursor, screens, history, title, directory, the 42-mode T00 inventory and parser state; they include split CSI, split UTF-8, both alternate-screen modes and the saved cursor. Other tests cover a CPR reply reaching the sink while a mirror restored mid-query answers nothing, the 5,000-byte OSC mid-sequence error with recovery at the terminator, every decode limit (truncated, corrupted at every 7th byte, trailing bytes, wrong and out-of-range sizes, history just over and just under 100,000 rows, the cell budget and an allocator refusal), and `format_vt` under 47, 1047 and 1049 against a never-alternate reference. `core_id()` is checked against `Cargo.lock`, the sys crate's own Ghostty pin and `third_party/fetch.sh`.
- **Review:** direct review; fixed a debug-only panic path in `snapshot` and dropped the OSC 4 palette from persistence output, so old program overrides don't mask the theme after reboot. Mutation checks: disabling continuation tracking fails the round trips. Leaving the alternate screen with `?1049l` alone is *not* caught: at this pin it returns to the primary screen but leaves the 47 or 1047 flag set, which only matters to a formatter emitting modes. M1 spec §6.3 now states that refinement of T00's finding, and the triple exit stays. libghostty's default 10,000-byte scrollback cap is replaced by the line limit. New dependency: `allocator-api2` 0.4.0, the trait libghostty-vt's `allocator_api` feature adapts.
- **Build:** `shoal-term`: `Terminal::new(cols, rows, scrollback)` with 4096-byte continuation tracking (M1 spec §4.2), `feed`, `feed_until_ground`, `resize`, `snapshot` (binary encode, with a distinct mid-sequence error), `restore` (bounded incremental decode with every limit in M1 spec §5.5), `format_vt` for persistence (main screen beneath an alternate screen, §6.3), `core_id()` (§5.3), a reply sink for terminal queries, `capture_text`, mode queries. Mirrors built by `restore` have no reply sink. No other crate may import libghostty.
- **Done when:** golden tests for write, resize with reflow, alternate screen, and a binary snapshot round trip comparing cells and modes, including the split-CSI, split-UTF-8, alternate-screen and saved-cursor cases from T00; a test proving the reply sink receives the answer to a cursor-position query, and that a restored mirror sends no reply; a 5,000-byte unfinished OSC gives the mid-sequence error, and `feed_until_ground` plus `snapshot` recovers at the terminator; truncated, corrupted, trailing-byte, wrong-size, over-history and over-allocation snapshots return errors without panicking; `format_vt` with alternate screens 47, 1047 and 1049 active saves the main screen and history, and leaves the live terminal on its alternate screen; `core_id()` matches the revisions in `Cargo.lock`; peak decode memory for 200 × 60 with 10,000 lines of history is measured and recorded.
- **Pin upgrades:** any later libghostty pin change re-runs these snapshot tests and the T00 fidelity cases before it merges.

### T08 · Terminal wrapper: rendering and input

- **Needs:** T07.
- **Build:** the GUI-facing half of `shoal-term`, split from T07: render-state access for one render state per terminal (dirty rows, bulk cell reads, styles and colors, graphemes, cursor; M1 spec §7.3), the key encoder using the terminal's current modes, and the mouse, focus and bracketed-paste encoders (§5.6, §7.4). All of it is exposed through Shoal-owned types, so the GUI never names libghostty.
- **Done when:** golden tests that encode keys under normal and application cursor modes and Kitty keyboard flags, mouse events under X10, SGR and any-event tracking, focus reports, and pastes with and without bracketed paste; and a render-state test whose dirty rows, cell text, widths, styles and cursor match a fed terminal and become clean after a frame.

## Phase 1 · The daemon

### T10 · Daemon skeleton and test kit

- **Needs:** T04, T05.
- **Build:** `shoald` startup from M1 spec §4.1 (runtime folder, lock, socket permissions, peer uid check), the `hello` handshake, unknown-method errors, logging (§4.6), clean exit on `SIGTERM`. `shoal-testkit`: a `TempEnv` that points every XDG folder and `HOME` at temp folders, spawns the built daemon, gives tests a raw connection, and provides `eventually`.
- **Done when:** tests show `hello` succeeds; a non-`hello` first message closes the connection; a second daemon exits 0 with "already running"; a stale socket file is recovered; `SIGTERM` exits 0 and removes the socket.

### T11 · Pane runtime

- **Needs:** T07, T10.
- **Build:** the pane thread from §4.2: PTY spawn, environment rules, the terminal setup (continuation tracking, reply and metadata callbacks), the `poll` loop, output batching, sequence numbers, the reply sink, the command channel, kill, exit detection, the early-exit guard. Internal API only.
- **Done when:** tests cover an echo round trip; `stty size` matching after a resize; `exit 3` giving `Exited { code: 3 }`; kill ending `sleep 100` within 2.5 s; a cursor-position query answered; the required environment variables present; the thread sleeping (state `S` in `/proc`) when the shell is idle.

### T12 · Workspace actor and methods

- **Needs:** T03, T11.
- **Build:** the single actor that owns the workspace; every §5.4 method except `pane.attach`, `pane.detach` and `pane.set_size`; `workspace.changed` notifications coalesced to 16 ms; close cascades; smart open (§4.4).
- **Done when:** integration tests cover each method's success path plus its not-found and invalid-state errors, and two simultaneous `session.open_smart` calls get different sessions.

### T13 · Attach and streaming

- **Needs:** T12.
- **Build:** `pane.attach`, `pane.detach`, the `SNAPSHOT`, `OUTPUT`, `RESIZED` and `INPUT` frames, 8 MiB subscriber queues with resync, mid-sequence snapshots split at the first ground point with the 2 s deadline and `pane.detached`, the `core` check on attach, size arbitration by focus (§5.3, §5.5–5.6).
- **Done when:** a mirror built from an attach taken mid-output equals the daemon's capture; an attach inside a 5,000-byte unfinished OSC completes at the terminator with no gap or duplicate, and without a terminator fails after 2 s while the shell keeps running; 100,000 lines stream with no sequence gap; a stalled reader gets a resync while a second client is unaffected; a connection with a different `core` gets 1003 from `pane.attach`; `pane.set_size` from an unfocused connection is ignored.

### T14 · Pane metadata

- **Needs:** T12.
- **Build:** §4.3: foreground program, directory (OSC 7, then `/proc`), the title rule, bell, event-driven triggers and throttling.
- **Done when:** tests show `cd` updating directory and title; `sleep 2` showing `sleep` then clearing; OSC 0 titles used only with `tabs.title = "program"`; no metadata work during 5 s of idle (an internal counter stays flat).

### T15 · Persistence writer

- **Needs:** T13.
- **Build:** §6.1–6.3: atomic writes, debounce rules, the formatter on the pane thread with compression on a blocking worker, the full-screen-program rule (snapshot on the pane thread, scratch decode and formatter on the worker), `scrollback.persist = false`, cleanup of closed panes.
- **Done when:** `workspace.json` matches a schema snapshot; a scrollback file appears after 2 s of quiet and decompresses to text containing a marker line; with `nvim` on the alternate screen, the file holds the marker and the main screen beneath it, not nvim's screen; `SIGTERM` writes everything; `persist = false` deletes the files; a simulated crash mid-write leaves the previous file intact.

### T16 · Restore

- **Needs:** T15.
- **Build:** §6.4 steps 1–7, plus the format loader with a saved format-1 fixture.
- **Done when:** the automated "Sessions survive" checks in M1 spec §13 pass, including the corrupt file, the missing folder, and new pane ids always larger than restored ones.

## Phase 2 · Client and CLI

### T20 · Client library

- **Needs:** T04, T10.
- **Build:** `shoal-client`, running its own tokio runtime on a background thread: connect with an injectable auto-start strategy (tests never call `systemctl`), `hello`, requests matched by id with timeouts, the notification stream, an attach-stream API that delivers snapshot then items in order and re-attaches on a gap, reconnect with backoff from 100 ms to 2 s. Public API is channels only; no tokio types leak out.
- **Done when:** tests against a real daemon cover each feature, including killing the daemon mid-session and seeing the client reconnect and re-attach once it's back.

### T21 · CLI commands

- **Needs:** T20, T12.
- **Build:** every command in M1 spec §11 except `prompt` and `shell-init`; target resolution including `.`; `--json`; the exit codes.
- **Done when:** snapshot tests of `shoal ls` (human and JSON), errors on stderr with the right exit codes, and a `send` then `capture` round trip.

### T22 · Daemon lifecycle

- **Needs:** T20, T16.
- **Build:** `shoald.service`; `shoal daemon start | stop | status | persist`; the auto-start strategy (§4.1); protocol, core and version mismatch detection in `hello` and the restart helper from §5.3 (shutdown that persists, wait up to 5 s, start, reconnect).
- **Done when:** a test exercises the fallback spawn in a `TempEnv` with no `systemctl`; manually, with the unit installed, `shoal` starts the daemon and the daemon outlives closing every window.

## Phase 3 · Shell integration and prompt

### T30 · Shell integration

- **Needs:** T11.
- **Build:** `share/shoal/shell/bash/` and `.../zsh/` scripts (M1 spec §8.1–8.2), the daemon's spawn wiring for `auto` mode, `shoal shell-init`, the load-once guard.
- **Done when:** tests run real bash and zsh in panes with fixture rc files and show: the rc's variable is set; OSC 133 `A`, `B`, `C`, `D;<code>` and OSC 7 appear in the output stream; an existing `PROMPT_COMMAND` still runs; `prompt.mode = "shell"` leaves `PS1` alone.

### T31 · Native prompt

- **Needs:** T06.
- **Build:** `shoal prompt` (§8.3): segments, both styles, per-shell escaping, git with its timeout, the runtime-version cache, the 100 ms hard cap.
- **Done when:** golden tests in temporary git repositories (clean, dirty, ahead and behind a local bare remote, detached, none) for both shells; a fake slow `git` on `PATH` produces `?` within the cap; a `hyperfine` check meets the budgets.

## Phase 4 · The window

Every GUI task keeps its logic in plain functions (workspace to chrome, render state to draw list, keys to actions) with unit tests, so only the final painting needs a human to look at it.

### T40 · Window shell

- **Needs:** T00, T06, T20.
- **Build:** `shoal-gui`: a GPUI window with the right `app_id`, title and transparent background; connects through `shoal-client` (sends `env`, smart open or `--session`, `window.show_session`, subscribe); draws the top bar (session chip and tab labels, no interaction yet) and pane rectangles from `rects` with theme colors and the focus border. `shoal` launches `shoal-gui` detached.
- **Done when:** unit tests for the workspace-to-chrome function; manually, the window appears translucent on Hyprland in the theme, its boxes match `shoal ls`, and changes made with the CLI appear live.

### T41 · Terminal rendering

- **Needs:** T07, T08, T40.
- **Build:** pane mirrors fed by attach streams, decoded within the §5.5 limits with no reply callbacks; the draw-list builder from render state (one render state per mirror, dirty rows, merged runs, M1 spec §7.3 color and alpha rules); cached per-cell glyph placement with grapheme clusters shaped as units and libghostty's cell widths (§7.3); the GPUI element that paints it; the font chain with the embedded symbols font; shapes for box drawing, blocks and powerline; cursor styles; the mid-sequence and decode-error banners with Retry; keeping mirrors for the 8 most recent tabs.
- **Done when:** draw-list golden tests from fixture terminal states (styled text, wide characters, emoji, box drawing, each cursor shape); manually, nvim, htop and btop render correctly and the logged full redraw of 200 × 60 is under 4 ms.

### T42 · Keyboard, paste and mouse

- **Needs:** T41, T05, T08.
- **Build:** the keymap dispatcher wired to every §10 action that exists so far; key encoding through the mirror's modes; input-method text; bracketed paste; mouse reporting versus local selection with Shift; focus reports; Ctrl+Insert and Shift+Insert.
- **Done when:** unit tests for keymap resolution and paste wrapping; manually, the key items in §13 "Window, keys and layout" pass.

### T43 · Layout interactions and tab bar

- **Needs:** T03, T41.
- **Build:** grid sizing and `pane.set_size` (§7.2), divider dragging, click to focus, zoom drawing, tab bar clicks (select, middle-click close, double-click or Ctrl+Shift+R rename inline, `+`), split buttons with key-cap tooltips, the bell dot.
- **Done when:** unit tests for the grid-size math; manually, drag, zoom and rename work.

### T44 · Session switcher

- **Needs:** T43.
- **Build:** §7.6, plus `session.next`, `session.prev` and `window.new`.
- **Done when:** a unit test for filter ranking; manually, switching, creating, renaming and closing sessions work from the keyboard.

### T45 · Scrollback, selection and links

- **Needs:** T41 (and T30 for the prompt-jump check).
- **Build:** §7.5 and the link behavior in §7.3.
- **Done when:** unit tests for selections staying anchored under new output and for URL detection; manually, smooth scrolling, the new-output pill, copy, primary selection, prompt jumps and Ctrl+click work.

### T46 · Reconnect and banners

- **Needs:** T40, T22.
- **Build:** §7.7: the reconnect overlay, the protocol, core and version mismatch banners with the user-triggered restart (§5.3), notice and config-error banners.
- **Done when:** manually, `kill -9` of the daemon with a window open recovers by itself, and a broken config line shows a banner naming it.

### T47 · Close confirmations and exited panes

- **Needs:** T43, T14.
- **Build:** the confirm bar for closing over a running program, the error view for panes that exit right after starting.
- **Done when:** a unit test for the "something is running" rule; manually, closing over `nvim` asks first and a bad `shell.program` shows its error instead of vanishing.

### T48 · Live theme and font changes

- **Needs:** T41, T06.
- **Build:** recolor on theme reload without re-attaching; per-window font zoom that resizes grids; applying `window.*`, `cursor.*` and `font.*` changes from config reloads, clearing the glyph and row caches on any font change (§7.3).
- **Done when:** manually, `omarchy-theme-set` recolors in under a second, and Ctrl+= grows the font while `stty size` in the pane shrinks to match.

## Phase 5 · Ship it

### T50 · Install and packaging

- **Needs:** T22, T30, T48.
- **Build:** `just install PREFIX=~/.local` (binaries, systemd unit, shell scripts, `dev.shoal.Shoal.desktop`, icon, the bundled font's license texts), `just uninstall`, a draft `PKGBUILD`, and `docs/omarchy.md` explaining how to make Super+Enter open Shoal (check Omarchy 4's current default-terminal mechanism first; don't guess). The bundled Symbols Nerd Font has mixed licenses and an unlicensed icon set (Font Logos); ask the user whether to subset those glyphs out or obtain terms before shipping it (`docs/spike-00.md`, "Font provenance").
- **Done when:** a clean install works on the user's machine, uninstall leaves nothing but config and state, and the installed font ships with the license texts for every icon set it contains.

### T51 · Performance harness

- **Needs:** T13, T41.
- **Build:** a script per measure in M1 spec §12; the headless ones run in `just check` with the 20% regression threshold.
- **Done when:** `docs/perf.md` has the machine's numbers and every budget is met, or each miss has its own task.

### T52 · Acceptance pass

- **Needs:** everything above.
- **Build:** nothing new: run M1 spec §13 end to end and turn every failure into a task.
- **Done when:** every box in §13 is ticked, and the one-week daily-use gate begins.
