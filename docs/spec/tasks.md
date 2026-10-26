# Milestone 1 tasks

32 tasks in six phases, each sized for one agent session. The daemon is finished and tested end to end through the CLI before the window exists, so the riskiest UI work is off the critical path.

**Current status:** T00–T46 done and merged into `main` (2026-10-26, T46 `ff0f47f`, 399 Rust tests). **Next: T47, then T48.** Full history for completed tasks is archived verbatim in `docs/spec/completed.md`.

| ID | Title | Status |
| --- | --- | --- |
| T00 | Spike: GPUI and libghostty on Hyprland | done 2026-10-25 |
| T01 | Repository scaffold | done 2026-10-25 |
| T02 | Model types | done 2026-10-25 |
| T03 | Layout operations | done 2026-10-25 |
| T04 | Protocol crate | done 2026-10-25 |
| T05 | Config core and keybinds | done 2026-10-25 |
| T06 | Omarchy theme loading | done 2026-10-25 |
| T07 | Terminal wrapper | done 2026-10-25 |
| T08 | Terminal wrapper: rendering and input | done 2026-10-25 |
| T10 | Daemon skeleton and test kit | done 2026-10-25 |
| T11 | Pane runtime | done 2026-10-25 |
| T30 | Shell integration | done 2026-10-25 |
| T12 | Workspace actor and methods | done 2026-10-25 `fca28c8` |
| T13 | Attach and streaming | done 2026-10-25 `b7a7881` |
| T14 | Pane metadata | done 2026-10-25 `4b66273` |
| T15 | Persistence writer | done 2026-10-25 `188b158` |
| T16 | Restore | done 2026-10-25 `f064096` |
| T20 | Client library | done 2026-10-25 `86d7926` |
| T21 | CLI commands | done 2026-10-25 `4169b2b` |
| T22 | Daemon lifecycle | done 2026-10-25 `326214a` |
| T31 | Native prompt | done 2026-10-25 |
| T40 | Window shell | done 2026-10-25 `ab29254` |
| T41 | Terminal rendering | done 2026-10-26 `093cf3f` |
| T43 | Layout interactions and tab bar | done 2026-10-26 `8a8495d` |
| T42 | Keyboard, paste and mouse | done 2026-10-26 `74919c7` |
| T44 | Session switcher | done 2026-10-26 `87e1f74` |
| T45 | Scrollback, selection and links | done 2026-10-26 `56df772` |
| T46 | Reconnect and banners | done 2026-10-26 `ff0f47f` |
| T47 | Close confirmations and exited panes | ready (next) |
| T48 | Live theme and font changes | ready |
| T49 | Polish and hardening pass | ready after T48 (accepted checkpoint, see below) |
| T50 | Install and packaging | blocked on T22, T30, T48 |
| T51 | Performance harness | blocked on T13, T41 |
| T52 | Acceptance pass | blocked on everything above |

**How to use this list**

- Hand an agent one task id plus `AGENTS.md`. Everything in the task's **Needs** must be done first.
- **Done when** is the definition of done: those checks pass in `just check`, or, for manual checks, the user has run them.
- Active tasks keep full **Needs / Build / Done when** below. Completed tasks keep one table row here; their verbatim record is in `docs/spec/completed.md`.
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

A simple safe order: T00, T01, T02, T03, T04, T05, T06, T07, T08, T31, T10, T11, T30, T12, T13, T14, T15, T16, T20, T21, T22, T40, T41, T43, T42, T44, T45, T46, T47, T48, T49, T50, T51, T52.

## Phase 4 · The window (remaining)

### T47 · Close confirmations and exited panes

- **Status:** ready; next in the safe order after T46. T43 and T14 are complete.
- **Needs:** T43, T14.
- **Build:** the confirm bar for closing over a running program, the error view for panes that exit right after starting.
- **Done when:** a unit test for the "something is running" rule; manually, closing over `nvim` asks first and a bad `shell.program` shows its error instead of vanishing.

### T48 · Live theme and font changes

- **Needs:** T41, T06.
- **Build:** recolor on theme reload without re-attaching; per-window font zoom that resizes grids; applying `window.*`, `cursor.*` and `font.*` changes from config reloads, clearing the glyph and row caches on any font change (§7.3).
- **Done when:** manually, `omarchy-theme-set` recolors in under a second, and Ctrl+= grows the font while `stty size` in the pane shrinks to match.

### T49 · Polish and hardening pass

- **Status:** accepted checkpoint (2026-10-26, with the user): runs after T48, before T50 packaging. Scoped below; not started.
- **Needs:** T47, T48.
- **Build:**
  - *Visual polish, M1 surfaces only* (top bar, panes, prompt, banners, switcher), guided by the "M1 polish checkpoint" section of `docs/design/tern-vs-shoal.md`: typography, spacing, colors, chrome, banners, interaction consistency. No M2+ features: no command palette, settings screen, theme picker, or new block kinds.
  - *Hardening review:* lifecycle races, recovery paths, resource cleanup, test gaps, unnecessary complexity. Fix concrete issues as they arise; no speculative broad refactor and no extra product features.
  - Informed by actual everyday use: the user dogfoods the window and reports friction; each finding becomes a focused fix, not a redesign.
  - If bigger than one sitting, split (e.g. visual vs hardening) and write the split back into this file, per AGENTS.md.
- **Done when:** draw-list/chrome golden tests cover the changed rendering; `just check` passes; a guided manual check (new `scripts/manual/t49.sh`) walks the reference doc side by side and the user signs off; review finds nothing unresolved.

## Phase 5 · Ship it

### T50 · Install and packaging

- **Desktop integration follow-up from T42:** Omarchy's terminal classification must include the app id `dev.shoal.Shoal`, so universal copy/paste chooses Ctrl+Insert / Shift+Insert. These shortcuts passed in T42 with a transient terminal tag on only the test window; no desktop config was changed.

- **Needs:** T22, T30, T48, T49 (the polish/hardening checkpoint runs before packaging).
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
