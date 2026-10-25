# Snapshot evidence — 2026-10-25

## Observed result

Pinned libghostty-rs `9ede35b8ea94a55ed3608408d67e102883129f1e`, Ghostty `521cf2c5d30bd48028ca61546c64e7913764073d`. No Rust changes or GUI runs were made for this investigation.

- Full fidelity table and all verbose differences: [snapcheck-programs-20261025T075721Z.log](snapcheck-programs-20261025T075721Z.log), lines 3–35 and 37–276. Command `./spike/target/release/snapcheck --programs --verbose`, UTC 07:57:21–07:57:34, exit 0. Exit 0 is not a fidelity assertion: main prints reports without failing on mismatches (`spike/snapcheck/src/main.rs:220–250`).
- All **31 scenarios retained**: 25 synthetic (`scenarios.rs:40–138`) plus six real programs (`programs.rs:161–186`). Binary restore and binary after-followup both report **31/31 match**. Formatter restore: **25/31 match**; formatter after: **17/31 match**; formatter + continuation after: **19/31 match**. The log preserves every per-scenario byte count and failure; no failure has been excluded.
- Binary followup is genuinely fed into decoded terminal C in the same loop as A, B, and D (`main.rs:102–124`), then compared again (`126–135`). Real programs include quitting alternate-screen programs, exposing primary history, not just comparing an initial display.

## Cost measurements

Command `./spike/target/release/snapcheck --bench`, UTC 07:57:34–07:57:35, exit 0; complete output in [snapcheck-bench-20261025T075721Z.log](snapcheck-bench-20261025T075721Z.log).

200×60 terminal, 10,000 styled input lines, 10,001 total rows, 10,000-line scrollback cap, no byte cap. Input 1,043 KiB fed in 41.9 ms. Serialized formatter 1,224 KiB, binary 2,801 KiB (integer KiB truncation).

| Iteration | Formatter dump ms | Formatter restore ms | Binary encode ms | Binary decode ms | Restored rows fmt / bin |
| --- | ---: | ---: | ---: | ---: | --- |
| 1 | 26.5 | 33.1 | 7.9 | 19.6 | 10001 / 10001 |
| 2 | 21.1 | 28.2 | 8.4 | 19.3 | 10001 / 10001 |
| 3 | 20.2 | 23.0 | 8.0 | 15.7 | 10001 / 10001 |

The separately printed `tracking=false: 37.4 ms` versus `tracking=true: 33.7 ms` is **not an on/off experiment**: `new_terminal` already enables 4096-byte continuation tracking (`main.rs:43–46`); the false branch never disables it (`164–172`). All formatter restores in the benchmark track continuation, whereas decoded binary terminals default to tracking disabled (`snapcheck-pinned-snapshot.rs:276–289`). Costs include formatter/binary vector copies and object allocation boundaries exactly as implemented (`main.rs:175–195`), are three serial warm iterations, and do not measure network transfer, GUI paint, peak RSS, cold starts, or benchmark fidelity. There is no distribution or confidence interval. Preserved benchmark VT output: `snapcheck-bench-dump.vt`.

## What comparison establishes (and does not)

`compare.rs:179–255` checks dimensions, active screen, total/scrollback row counts; every overlapping cell in the active screen including its scrollback, bottom-aligned; text, wide flag, style, hyperlink URI, protection, wrap flag, semantic prompt (`127–156`); cursor x/y, pending wrap, visibility, pen style, at-prompt; 42 explicit mode flags (`7–50`); current kitty flags, mouse tracking/shape, title, pwd, OSC 10/11/12 colors and all 256 palette entries. Mismatch examples stop at four cells; row-level metadata differences count once per cell, so “3000 cells differ” is not necessarily 3000 glyph differences. Unequal dimensions/history report separately while only overlapping cells are enumerated.

No direct comparison of inactive-screen cells, saved cursor, scroll margins, tab stops, charset state, parser internals, modifyOtherKeys state, kitty keyboard stack, cursor presentation shape, callbacks, reply bytes, image contents, or configuration. Some are exercised indirectly by targeted followup: saved cursor, margins/origin, tabs, charsets, alternate screen exits, split CSI and UTF-8. **The DECSCUSR cursor-shape scenario's match is not proof of shape preservation:** `cursor_style()` here means SGR pen attributes, not cursor presentation shape. modifyOtherKeys and input protocol scenario matches likewise do not demonstrate encoded keyboard/mouse event equivalence; no key encoder comparison occurs.

Real-program trials use 120×40 PTYs, `TERM=xterm-256color`, `COLORTERM=truecolor`; A alone answers queries (`programs.rs:47–119`). Quiet windows decide attach and completion (400/500 ms), with no semantic application-ready/completed assertion, captured raw PTY stream, or program exit-status validation. `run_all` can silently omit failed starts via `filter_map`; this run nevertheless printed all six expected rows. Shell failures can still look like a program run because each launches bash. Program versions/startup availability need environment evidence. This is one local run, not cross-platform, exhaustive protocol, race, transport, corruption, or resize validation.

## Exact formatter gap inventory

All exposed snapshot-relevant extras were enabled: `main.rs:50–69`; pinned Rust options are preserved in `snapcheck-pinned-fmt.rs:35–163`. This inventory separates absent API/state coverage from shortcomings in existing output.

| Tested missing state | Evidence | Classification |
| --- | --- | --- |
| Wrapped-row metadata | 160 cells retain text/styles but row-wrapped true becomes false; log 37–55 | Existing unwrap/layout option tradeoff, not a missing `with_modes` flag. Harness chooses `unwrap=false`. Implementation explicitly adds newlines when `!unwrap`, `snapcheck-pinned-formatter.zig:1347–1349`; alternate unwrap choice was not measured. |
| OSC 10 foreground, OSC 11 background, OSC 12 cursor color | log 58–73 | No dynamic-color options in Rust/C terminal options. Palette option emits OSC 4 only (`snapcheck-pinned-formatter.zig:363–373`). Internal Zig generic Options has optional foreground/background (`104–113`) and PageFormatter can emit OSC 10/11 (`1187` onward), but C constructor passes only emit/unwrap/trim (`snapcheck-pinned-c-formatter.zig:143–148`); cursor color is not covered by those fields. Not “Ghostty has no color support.” |
| Title (pwd survives) | log 76–85 | No title option in pinned Rust or terminal Extra; pwd explicitly exists. |
| Saved DECSC position and SGR pen | log 88–101 | No saved-cursor option; implementation expressly says saved cursor is not emitted (`snapcheck-pinned-formatter.zig:753–758`). Initial cursor matches, DECRC followup fails. |
| Inactive primary screen + history underneath alternate screen | synthetic 1049/47, log 104–137; nvim/btop/lazygit after exit, 197–254 | Formatter API deliberately active-screen-only (`snapcheck-pinned-fmt.rs:165–176`; `snapcheck-pinned-formatter.zig:237–241,302–305,487–490`). Modes do restore active screen bit; they do not serialize inactive contents. 1049 case loses 77 history rows, nvim loses 161. |
| OSC 133 semantic row prompt marks and at-prompt | log 140–161,257–276 | No semantic-prompt option among Rust/C extras; observed metadata loss, not necessarily visible glyph loss. |
| Per-cell DECSCA protection | log 164–182 | Existing protection option restores current cursor protection only (`snapcheck-pinned-formatter.zig:802–809`); does not reproduce previously protected cells. Followup selective erase removes PROT incorrectly. |
| Unfinished CSI and partial UTF-8 | log 185–194 | Formatter has no parser-state option. Separate continuation bytes fix **these two** tested after-followup failures (`main.rs:91–100`). This does not fix other 12 formatter+continuation failures. |

Other state is **untested**, not an observed failure: cursor presentation shape (despite named scenario), cursor color stack, title stack, kitty keyboard push/pop stack, modifyOtherKeys input behavior, >32-codepoint graphemes (>4096-byte URIs), split OSC/DCS/APC and over-4096-byte unfinished sequences, graphics/images, inactive alternate screen while primary active, saved mode state, selection/viewport, cross-geometry resize/reflow, and arbitrary combinations/order of these states. Fixed comparison buffers and `.ok()`/default conversions can hide API errors or oversized values (`compare.rs:138–156`); these fixtures do not establish behavior there.

## Explicit mode fallback

The actual 42-flag list is `compare.rs:7–50`: ANSI KAM(2), IRM(4), SRM(12), LNM(20); DEC ?1,3,4,5,6,7,8,9,12,25,40,45,47,66,69,1000,1002,1003,1004,1005,1006,1007,1015,1016,1035,1036,1039,1045,1047,1048,1049,2004,2026,2027,2031,2033,2048,5522. This is the harness's comparison inventory, not a new supported attach protocol. It includes screen/cursor transitions (47/1047/1048/1049), so arbitrary sequential replay is not equivalent to assigning booleans; order/defaults and destructive effects matter.

Pinned formatter already iterates every internal mode and emits values differing from Ghostty defaults before content (`snapcheck-pinned-formatter.zig:409–425`); none of the observed reports lists mode mismatches. An explicit mode list is therefore not a fix for these measured failures. [INFERENCE] Flags alone cannot reconstruct underlying primary cells/history, saved cursor/pen, title/dynamic colors, prompt/protected-cell metadata, or unfinished parser bytes: those contain data not encoded by booleans. A mode list could be an explicit input-state contract, but a lossless attach fallback would need an independently specified state/serialization solution and new evidence, not merely this list.

## Compatibility and integration constraints (no implementation)

Binary version 1 is expressly work-in-progress **without binary compatibility guarantee** (`snapcheck-pinned-snapshot.rs:76–77`). Envelope requires exact version (`snapcheck-pinned-envelope.zig:27–29,50–57`); equality of version 1 across future commits is not a guarantee because version 1 may still change. Freeze/compare exact library revisions for experimental peers; negotiate/reject incompatible implementations rather than calling it a stable public protocol. No cross-revision or cross-platform interchange was measured. The documented encoding is little-endian, ordered CRC32C-protected records rather than raw pointer memory (`snapcheck-pinned-snapshot.rs:17–74`).

Decode tested only one-shot in memory through FINISH, not progressive READY/history, bounded decoding, corrupted/untrusted payloads or transport recovery. A live integration must freeze mutation over snapshot acquisition and align a byte-stream handoff boundary with subsequent output; harness serializes these naturally (`main.rs:87–124`). Unfinished VT/UTF-8 requires continuation tracking enabled before the relevant bytes, otherwise binary encode can reject (`snapcheck-pinned-snapshot.rs:101–111,134–137`). Returned terminal has parser continuation restored but future continuation tracking disabled by default (`276–289`); another snapshot later in a partial sequence requires deliberate tracking configuration. Runtime callbacks/PTY ownership need rebinding rather than assuming snapshot contains executable host wiring; no callback-restoration test was run.

## Preserved sources

`snapcheck-pinned-fmt.rs`, `snapcheck-pinned-snapshot.rs`, and `snapcheck-pinned-build.rs` are unchanged copies from the pinned libghostty-rs checkout; build source pins Ghostty at lines 5–7 and resolves override/fetch at 125–137. `snapcheck-pinned-formatter.zig`, `snapcheck-pinned-c-formatter.zig`, `snapcheck-pinned-envelope.zig` are unchanged copies from `spike/target/release/build/libghostty-vt-sys-a459bae458945036/out/ghostty-src/src/terminal/` (formatter, c/formatter, snapshot/envelope respectively). Their line numbers match cited originals; evidence survives /tmp cleanup.
