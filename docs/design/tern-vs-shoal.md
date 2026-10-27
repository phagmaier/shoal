# Tern vs Shoal: visual reference for the polish checkpoint

Reference only — not spec, not tasks. Input to the bounded polish + review
checkpoint the user accepted after T48, now T49 (visual polish) and T53
(hardening review) in `docs/spec/tasks.md`. No fixes here.

## Tern reference shots (user-supplied)

Drop the three PNGs into `docs/design/tern-reference/` with these exact names:

| File | What it shows |
| --- | --- |
| `tern-blocks-tabs.png` | Full window: three blocks (terminal, scratch markdown note, system task manager), tab strip on top, full top bar |
| `tern-palette.png` | Command palette overlay: filter tabs, panes, sessions, git actions |
| `tern-settings.png` | Settings open as a block beside a coding-agent block |
| `tern-agent-block.png` | Agent block beside a terminal: Claude Code welcome, hints, composer with model/effort pickers |

### Shot 1 — blocks, tabs, top bar

Full Tern window on a dimmed wallpaper. Top bar, left to right: red status dot +
truncated session name, avatar, `Desktop` chip, tabs
(`ue T46 implementation | shoal`, `+ OC | Project docs for agents`, `shoal`,
`shoal`), `+` button, then icon buttons (copy, split, layout), search, `/`,
notification bell. Left block is a terminal with a powerline prompt (user, path,
`main` branch, `v1.98.1`) and icon-rich `ls` output. Right column stacks two
heterogeneous blocks: a scratch markdown note (title bar with back/forward,
`Text · Ln 1, Col 23`, eye/code/`...` buttons) above a system task manager
(CPU 1%, `Memory 3.0 GB of 12 GB` with bar, List/Tree toggle, pause; table with
Name/PID/%CPU/Memory/%GPU columns and per-row icons). Dark navy palette;
wallpaper faintly visible around block edges.

### Shot 2 — command palette

Centered overlay, `Run a command or go to a pane...` with `esc` hint. Filter tabs:
All (active, magenta) / Commands / Files / Branches / Commits / Sessions /
Hosts. Sections: Panes (3 rows: icon, cyan name, dim path, `Ctrl Shift 1/2/3`
key-cap chips), Sessions (`Default 1 tab`; `Desktop 3 tabs · <host> · current`
with checkmark), Git (`Fetch all`, `Pull` with icons). Bottom hint bar:
`navigate`, `run`, `esc close`.

### Shot 3 — settings as a block

Settings opened as a new block in the same tab: agent conversation on the left,
Settings card UI on the right. Search field with magenta border; icon category
row (general, profile, appearance, terminal, sessions, ...); section
`01 · General` / `Shell, tabs and selection` with description line; setting
cards (`Shell` with `/usr/bin/zsh` path field; `Tabs and splits` with
Shell/Agent and Current directory/Start folder segmented toggles; `Start
folder`). Reads like a GUI, not a config file.

### Shot 4 — agent block (additional shot)

Two blocks side by side. Left is a terminal with a powerline prompt (user,
`.../uniichat`, `main`, node `v26.7.0`). Right is a Claude Code agent block:
large cyan asterisk logo + `Claude Code 0.95.0`, a one-line hint row (`Esc`
stops a turn, Shift+Tab switches mode, `/agent` switches agents, `@` attaches
files), and a bottom composer (`Ask Claude Code — / commands · @ files`) with
pills for model (`Opus 5.5`), effort (`High`), mode (`Auto`), branch (`main`)
and plan (`Claude Pro`), plus a send button. The composer carries an accent
left border. Top bar shows avatar, `Default` chip, `Claude Code` and `Webapp`
tabs, `+`, action icons, search, `/`, bell. Note the pattern: the block's
identity (logo), its affordances (hints), and its input (composer) are all
visible at once with no chrome wasted.

## Shoal current state (M1, T46)

In `docs/design/shoal-current/`, copied from T43 guided-remote evidence:

| File | What it shows |
| --- | --- |
| `shoal-splits-tooltip.png` | Three terminal panes in a split, native powerline prompt, `Split right` tooltip with key caps |
| `shoal-topbar-tabs.png` | Top-bar close-up: `main` chip, two `shoal` tabs, `+`, split buttons |

What Shoal already matches: translucent panes over blurred wallpaper, tab strip
with active-tab accent underline, powerline prompt with git/runtime segments,
icon-rich `ls` via the bundled symbols font, key-cap tooltips on chrome buttons.

## Gaps, scoped by milestone

### M1 polish checkpoint (fair game)

These concern surfaces M1 already owns — top bar, panes, prompt, banners:

- **Top bar density.** Tern's bar carries session, avatar, tabs, window actions,
  search and bell in one strip. Shoal's 34 px bar carries chip + tabs + two
  split buttons. Sparse by comparison; bell, search and window actions have no
  home (several are M2 features — do not build the features, but leave room).
- **Pane tops.** The latest user decision removes grips and their reserved
  space, including the grip shown in Tern. T49 uses clean pane tops and a
  subtle active edge with a fading top line. Keep the compact spacing and
  glass treatment selected during the visual exploration.
- **Card-style overlays.** Shoal's banners, confirm bar (T47) and switcher (T44)
  are the closest M1 analogues to Tern's palette/cards. Tern's spacing,
  segmented toggles, key-cap chips and one-line notice style are the concrete
  model here — Tern's palette chips vs Shoal's tooltip caps already rhyme.
- **Prompt and icon tuning.** Both prompts are powerline; Tern's segment colors,
  separators and file-icon treatment are the side-by-side target T41's manual
  check already asked for.

### Later milestones (do NOT pull into polish)

Shown in the shots but explicitly out of M1 — reference for their own tasks:

- Command palette → M2. Settings screen → M2. Task manager / system blocks →
  no milestone (never-build-adjacent; do not spec it). Markdown block → M6.
  Agent block → M3 (composer pills, hint row and welcome are the model for
  that task's composer). Themes beyond Omarchy sync → M2 theme picker.
