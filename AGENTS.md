# AGENTS.md — Shoal

Shoal is a terminal emulator and multiplexer for Linux (Wayland, Hyprland, Omarchy).
A per-user daemon, `shoald`, owns every shell, pane and layout. Windows, `shoal-gui`,
mirror it. The `shoal` CLI scripts it and draws the native prompt.

Nearly all of this code is written by agents. The rules below are strict on purpose:
they are what lets one agent change one crate without breaking another.

## Read first

- `docs/spec/` is the spec:
  - `decisions.md`: the key technical decisions and the milestone plan.
  - `m1-spec.md`: the milestone 1 spec. Tasks cite it as "M1 spec §N".
  - `tasks.md`: every M1 task with its **Needs**, **Build** and **Done when**.
- The spec wins over intuition. If the spec and the code disagree, or the spec is
  ambiguous, stop and ask. Do not invent behaviour.
- The "Never build" list at the end of this file is permanent.

## Architecture on one screen

```
shell ⇄ PTY ⇄ pane thread (shoal-term terminal) ──seq-numbered bytes──▶ Unix socket
      ──▶ shoal-client ──▶ window mirror (shoal-term) ──▶ draw list ──▶ GPUI
```

The daemon is the only source of truth. A window is a mirror: it receives a snapshot
of each pane it shows, then every new output byte with a sequence number, and feeds
both into its own terminal copy.

| Crate | Binary | Owns |
| --- | --- | --- |
| `shoal-model` | | Workspace types, ids, layout tree and operations, rectangle math |
| `shoal-proto` | | Frame codec, message types, binary frame layouts |
| `shoal-config` | | Config schema, keybinds, Omarchy theme, palette, file watching, XDG paths |
| `shoal-term` | | The only wrapper around libghostty-vt |
| `shoal-daemon` | `shoald` | Socket server, workspace actor, pane threads, persistence, restore |
| `shoal-client` | | Connection, auto-start, requests, notifications, attach streams |
| `shoal-cli` | `shoal` | CLI, GUI launcher, `shoal prompt`, `shoal shell-init` |
| `shoal-gui` | `shoal-gui` | Window, chrome, pane views, input, rendering |
| `shoal-testkit` | | Test-only: temp XDG environment, daemon spawning, fixtures |

### Hard boundaries (cargo-deny enforces 1, 2, 5 and 6)

1. Only `shoal-term` imports libghostty-vt.
2. Only `shoal-gui` imports GPUI.
3. `shoal-model` has no I/O, no async, no clock and no randomness. Pass time in.
4. `shoal-proto` does not depend on tokio.
5. `shoal-cli` never depends on `shoal-term` or GPUI, so the prompt starts fast.
6. The daemon never depends on the GUI.

`scripts/check-boundaries.sh` additionally rejects tokio in `shoal-proto` and
non-dev dependencies on `shoal-testkit`; cargo-deny's wrappers cannot express
dependency kind. `just check` runs both gates.

### Concurrency rules

- Workspace state lives only in the workspace actor. Change it by sending messages.
  Never block the actor; send file I/O to `spawn_blocking`.
- Each pane's terminal and PTY live on that pane's thread. `shoal-term` types are not
  `Send` by design. Do not wrap them in `Arc<Mutex<_>>`; send commands to the thread.
- Window mirrors never write to the PTY. Only the daemon answers terminal queries.
- No timers that fire while idle. Near-zero idle CPU is a requirement, not a goal.

## Commands

```sh
scripts/setup-machine.sh --check   # report what this machine is missing (agents: --check only)
mise install          # pinned Rust, Zig and development tools
just check            # fmt, clippy -D warnings, tests, cargo-deny; must pass before a commit
just test             # tests only
just dev-daemon       # shoald in a temp XDG sandbox, logging to stderr
just dev-gui          # a window against the dev daemon
just fuzz-proto       # fuzz the frame decoder
cargo test -p shoal-model
cargo insta review    # accept snapshot changes deliberately, never blindly
```

## Looking things up

- **libghostty-vt:** `third_party/ghostty/include/ghostty/vt.h` and the pinned
  `libghostty-vt` crate source. Never guess a function name.
- **GPUI:** `third_party/gpui/` is the pinned source; read it and its examples. Online
  GPUI docs often lag the pin; the vendored source wins.
- **Zed's terminal view** is a useful reference for drawing a grid in GPUI, but Zed's
  app crates are GPL-3.0: read them for ideas, never copy code from them.

## Conventions

- Rust 2024. `thiserror` in libraries, `anyhow` only in binaries' `main`.
- No `unwrap()` outside tests. `expect("invariant: ...")` only for true invariants.
- Logging with `tracing`. Never log pane output, typed input or environment values.
- Serde: `rename_all = "snake_case"`; ids serialize as prefixed strings (`"p40"`).
- A protocol or persistence-format change updates `docs/spec/` in the same commit,
  bumps the version or format number as the spec says, and adds a fixture test.
- Files stay under about 600 lines; one concept per module.
- New dependencies need a line in the commit message: what it does and why an
  existing one won't do.
- Linux only. No `cfg` branches for macOS or Windows.
- Paths come from `shoal-config`'s XDG helpers. Never hard-code `~/.local`.
- User-facing text is short, plain and in sentence case.
- Never run `sudo`. If a system package is missing, tell the user the exact
  `pacman` command and wait.
- Without asking, you may install tools pinned in `mise.toml` (`mise install`), Rust
  toolchains and components (`rustup`), and cargo tools (`cargo install --locked`).
  Ask before installing anything else outside the repository.
- `scripts/setup-machine.sh` sets up a machine for this repo. The user runs it; it
  uses `sudo`. When a task needs a new system package or tool, add it to the script
  in the same commit.

## Testing

- Each task's **Done when** lists its tests. Write them first where practical.
- Pure logic: unit tests and `proptest` property tests in the same crate.
- Daemon behaviour: integration tests with `shoal_testkit::TempEnv`, which isolates
  every XDG folder and `HOME`, runs the real `shoald`, and uses real bash and zsh with
  fixture rc files. Never touch the real `~/.local/state/shoal` or the real socket.
- Snapshot tests (`insta`) for JSON, CLI output and draw lists.
- Anything that waits has a timeout (default 5 s). Never `sleep` to synchronize; use
  `shoal_testkit::eventually` with a deadline.
- GUI: keep logic in pure functions with tests. List the manual checks the user needs
  to run, with the exact commands, and wait for their results.

## Working on a task

1. Read the task in `tasks.md` and the spec sections it cites.
2. Check that everything in **Needs** is done.
3. Stay in scope. Extra work you discover goes in your summary as a proposed new task.
4. If the task is bigger than one sitting, stop, propose a split, and update `tasks.md`.
5. Work on a branch named `t<NN>-<short-name>`. Commit when **Done when** passes.
   The one exception is the T00 spike, on `spike/t00`, which is never merged.
6. Merging needs no approval (the user's standing decision, 2026-10-25): once
   **Done when** passes, `just check` passes on the commit, any manual checks the
   user ran came back clean and the review found nothing unresolved, fast-forward
   `main` to the task commit and push it. Then commit `docs: record T<NN>
   integration and next task` to update `tasks.md`, and push that too. Hold the
   merge and say why if any of these is not true.
7. Finish with a summary: what changed, how it was tested, manual checks the user must
   run, and follow-ups.

## Gotchas

- Reading a PTY returns `EIO` when the child exits. That is end of file, not an error.
- Set close-on-exec on every file descriptor the daemon opens, so shells don't
  inherit them.
- Let portable-pty handle the new session and controlling terminal; don't
  re-implement it.
- Sequence numbers are per pane. A gap means re-attach; never skip ahead.
- Snapshot bytes go into a fresh terminal, never into an existing one.
- Omarchy swaps the theme folder by renaming it into place. Watch the parent folder,
  not files inside the old one.
- GPUI state changes happen on the main thread. Receive bytes from `shoal-client`'s
  channel in a foreground task.
- Hyprland blurs translucent windows. Do not implement blur.
- An agent's shell may lack `HYPRLAND_INSTANCE_SIGNATURE` even under Hyprland. Check
  `pgrep -x Hyprland` or `$XDG_RUNTIME_DIR/hypr/` instead.
- GPUI builds use a lot of memory. If the linker or rustc is killed, retry with
  `CARGO_BUILD_JOBS=8` (or lower) rather than changing code.

## Never build

From the vision's cut list, permanently: a bundled agent or an agent-specific UI
protocol; macOS, Windows, mobile or browser clients; a SQLite browser; notebooks; a
kanban board; a task manager; screen sharing; an embedded web browser; a full git
client; a file or media viewer; a code editor; a plugin SDK.

Not in milestone 1 (they have their own milestones): the command palette,
notifications, scrollback search, settings screen, theme picker, layout presets,
drag-to-rearrange, inline images, agent panes, remote hosts, git palette actions,
the Markdown pane and orchestration.
