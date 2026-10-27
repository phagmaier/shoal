# Milestone 1 tasks

32 tasks in six phases, each sized for one agent session. The daemon is finished and tested end to end through the CLI before the window exists, so the riskiest UI work is off the critical path.

**Current status:** T00–T48 done and merged into `main` (2026-10-26, T48 `f52912f`, 410 Rust tests). **Next: T49 integration.** T48's full gate and guide passed (`target/t48-check-f52912f.log`, `target/manual/t48.txt`); the known CLI auto-start flake in the first run (`target/t48-check-14cc333.log`) passed a focused retry, 20 repeats and the full rerun. T00–T46 history is archived verbatim in `docs/spec/completed.md`; later task history is in Git.

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
| T47 | Close confirmations and exited panes | done 2026-10-26 `b5f8ac6` |
| T48 | Live theme and font changes | done 2026-10-26 `f52912f` |
| T49 | Visual polish pass | accepted for this milestone; integration pending |
| T53 | Hardening review | blocked on T49 |
| T50 | Install and packaging | blocked on T53 |
| T51 | Performance harness | blocked on T13, T41 |
| T52 | Acceptance pass | blocked on everything above |

**How to use this list**

- Hand an agent one task id plus `AGENTS.md`. Everything in the task's **Needs** must be done first.
- **Done when** is the definition of done: those checks pass in `just check`, or, for manual checks, the user has run them.
- Active tasks keep full **Needs / Build / Done when** below. Completed tasks keep one table row here; T00–T46's verbatim record is in `docs/spec/completed.md`, and later task history is in Git.
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

### T49 · Visual polish pass

- **Visual exploration:** the user chose compact spacing, glass, adjustable opacity and no pane-top grips. The centered switcher follows `~/Pictures/screenshot-2026-10-26_22-15-53.png`; `~/Pictures/screenshot-2026-10-26_22-21-39.png` shows the desired glass, wallpaper, tabs and terminal content. Judge the actual prompt, colors and glyphs in the real terminal. Remote indicators and switcher categories in references do not add those features to M1.
- **Status:** user feedback applied and accepted for this milestone on branch `t49-polish-hardening`. The agent checks accessible behavior and captures the real window; `just manual t49` offers optional free exploration and free-form feedback. The accepted polish/hardening checkpoint was split: T49 is the visual pass; the systematic hardening review is T53. Concrete defects found while polishing are fixed here.
- **Latest iteration:** grip-free panes, subtle active edge and centered responsive switcher; the user asked for a slightly brighter active edge, now applied. The agent's window checks and their limitations are recorded in `target/t49-continuation/run2-results.md`; the full gate after the light-theme contrast fix passed (`target/t49-continuation/run2-check.log`).
- **User feedback:** the saved partial run `target/manual/t49.handoff-preserved.txt` passed through the narrow switcher check. The user requested agent-run checks and optional free exploration, and accepted the result for a prototype while noting that it is not visually ideal. The earlier airy-size timeout's cause remains unconfirmed. A subsequent agent pass improved faint light-theme switcher hints and placeholder text.
- **Previous second pass (superseded pane grips and switcher placement):** 1.4 row spacing, solid explicit backgrounds, layered glass preserving pane opacity, borderless panes with focus lines and grips, Noto Sans chrome. Screenshots: `target/t49-second-pass/`. Spec, goldens and guide updated. Full gate passed (`target/t49-second-pass/check-rerun.log`, 418 Rust tests); the empty-systemctl-log flake in `check.log` passed alone and 20 repeats (`startup-retry.log`, `startup-20.log` in the same folder). Code review found no unresolved issues.
- **First pass result:** the user's guide run (`target/t49-evidence/user-guide-run-1.txt`) passed every surface check but declined sign-off: "good for a beta" but not Tern's glassy, professional polish; the user wanted it closer to Tern. The one functional failure (panes stayed shifted after dismissing a banner until the next event) is fixed: GPUI ignores a notify made while drawing, so the measured banner height is now applied after the frame. A later agent preview rechecked dismissal.
- **Needs:** T47, T48.
- **Settled with the user, first pass (2026-10-26):** the prompt keeps the §8.3 colors and gains rounded start and end caps; the active tab becomes a Tern-style filled pill with `accent` text, and the session chip gains a glyph and a divider. (Mono chrome text and per-pane header strips were also chosen, then reversed for the second pass below.)
- **Settled with the user, second pass (2026-10-26):** close the remaining gap to Tern with these choices, amended by the latest visual decisions:
  - *Airy text, vivid colors:* default `font.line_height` about 1.4 instead of 1.0, and explicit cell backgrounds drawn solid (Ghostty's default) instead of at `window.opacity`. Changes §9.1 and §7.3.
  - *Glass surface:* the whole window, gaps and padding included, is one tinted glass surface instead of raw wallpaper; the focused pane has a faint `accent` edge with a soft line along its top. Changes §7.2 (and the §13 opacity check's wording if needed).
  - *Clean pane tops (latest decision):* remove every grip and its reserved top zone. Keep the shared content rectangle for grid sizing, painting, pointer coordinates, selection and IME. No title strip. Drag-to-rearrange stays out of M1.
  - *UI sans for chrome:* tabs, chip, cards, banners and tooltips use a proportional UI font; the terminal stays in `font.family`.
  - *Session switcher (latest decision):* a wide centered rounded panel with a search row, selected session row and compact key-cap hints, following the supplied switcher screenshot. Preserve M1 filtering, creation, switching, rename and close; no command palette categories or new actions. Keep B's compact pane spacing and configurable opacity.
  - Not now: Tern's look is customizable in its settings; Shoal's settings screen is M2. Prompt customization beyond `prompt.style`/`prompt.segments` was raised but not decided.
- **Build:**
  - *M1 surfaces only* (top bar, panes, prompt, banners, switcher), guided by the "M1 polish checkpoint" section of `docs/design/tern-vs-shoal.md` and the decisions above: typography, spacing, colors, chrome, card-style overlays with key caps, interaction consistency. No M2+ features: no command palette, settings screen, theme picker, or new block kinds.
  - Update M1 spec §7.2, §8.3 and §9.2 for the settled changes, and `docs/using-shoal.md`.
- **Done when:** draw-list/chrome golden tests cover the changed rendering; `just check` passes; accessible interactions and rendering are checked in the real window; the user gives genuine visual acceptance after reviewing the window or evidence and can provide free-form feedback; review finds nothing unresolved.

### T53 · Hardening review

- **Needs:** T49.
- **Build:** a systematic review of lifecycle races, recovery paths, resource cleanup, test gaps and unnecessary complexity across daemon, client and window. Fix concrete issues as they arise; no speculative broad refactor and no extra product features. Informed by actual everyday use: the user dogfoods the window and reports friction; each finding becomes a focused fix, not a redesign.
- **Known findings (from T49):** a config error banner stays after the file is fixed, until dismissed; the daemon's copy of the notice is also shown again in windows opened later. Decide whether a valid reload clears it (§7.7 is silent) before changing the window's notices or the daemon's.
- **Done when:** each finding is fixed with focused regression coverage or recorded as its own task with a reason; `just check` passes; review finds nothing unresolved.

## Phase 5 · Ship it

### T50 · Install and packaging

- **Desktop integration follow-up from T42:** Omarchy's terminal classification must include the app id `dev.shoal.Shoal`, so universal copy/paste chooses Ctrl+Insert / Shift+Insert. These shortcuts passed in T42 with a transient terminal tag on only the test window; no desktop config was changed.

- **Needs:** T22, T30, T48, T49, T53 (the polish and hardening checkpoint runs before packaging).
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
