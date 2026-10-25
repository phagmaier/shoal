# Milestone 1 spec

Milestone 1 delivers a terminal you can use every day: one window per Hyprland tile with sessions, tabs and splits, your Omarchy theme, a native prompt, and a workspace that survives closing the window and rebooting. Everything here is binding for agents; where the spec says "must", tests check it.

## 1. Scope

**In milestone 1**

- The daemon, `shoald`: sessions › tabs › split panes, a shell per pane, persistence and restore after reboot.
- The window, `shoal-gui`: tabs and splits, keyboard and mouse control, a minimal session switcher, translucent Omarchy-themed look.
- The CLI, `shoal`: launches windows, a small set of scripting commands, the native prompt and shell integration for bash and zsh.
- Clickable links (Ctrl+click), copy and paste including Omarchy's Super+C / Super+V, and jumping between prompts.

**Not in milestone 1** (each lands in the milestone named in `decisions.md`)

- Command palette, notifications, scrollback search, settings screen, theme picker, layout presets, drag-to-rearrange and the pane drag handle (M2).
- Showing inline images. libghostty already parses them; drawing them waits for M2.
- Agent panes, remote hosts, git palette actions, the Markdown pane, orchestration (M3 to M7).
- Everything on the vision's "explicitly not building" list, permanently.

## 2. Components and boundaries

One Cargo workspace, nine crates. The dependency rules in the last column are enforced by `cargo-deny` and review.

The last column lists the other Shoal crates each crate may use, plus the main external ones. Small utility crates that the conventions in `AGENTS.md` call for (`thiserror`, `tracing`, `clap` in binaries) are fine without being listed. Anything else heavy needs a reason in the commit message. A crate never depends on a Shoal crate that isn't in its column.

| Crate | Kind | Owns | May depend on |
| --- | --- | --- | --- |
| `shoal-model` | lib | Workspace types, ids, the layout tree and its operations, rectangle math. Pure: no I/O, no async, no clock. | serde, jiff (types only, never `now()`) |
| `shoal-proto` | lib | Frame codec, every message type, binary frame layouts | `shoal-model`, serde, serde_json |
| `shoal-config` | lib | Config schema and defaults, keybind parsing, Omarchy theme loading, palette, file watching | serde, toml, notify |
| `shoal-term` | lib | The only crate that uses libghostty-vt: terminal, snapshot and restore, key and mouse encoders, render-state access | libghostty-vt |
| `shoal-daemon` → `shoald` | bin | Socket server, workspace actor, pane threads, PTYs, metadata, persistence, restore | model, proto, config, term, tokio, portable-pty, rustix |
| `shoal-client` | lib | Connect and auto-start, requests, notifications, attach streams, reconnect | model, proto, tokio |
| `shoal-cli` → `shoal` | bin | CLI commands, GUI launcher, `shoal prompt`, `shoal shell-init` | client, config. Never term or gpui. |
| `shoal-gui` → `shoal-gui` | bin | Window, chrome, pane views, input, rendering | client, config, model, term, gpui |
| `shoal-testkit` | lib, tests only | Temporary XDG environment, daemon spawning, fixture shells | client, proto |

**Rules that matter most:** only `shoal-term` imports libghostty; only `shoal-gui` imports GPUI; `shoal-model` stays pure; the daemon never depends on the GUI.

**Where things live on disk**

| What | Path |
| --- | --- |
| Daemon socket and lock | `$XDG_RUNTIME_DIR/shoal/shoald.sock`, `shoald.lock` (folder mode 0700) |
| Config | `~/.config/shoal/config.toml` |
| State | `~/.local/state/shoal/`: `workspace.json`, `panes/<pane-id>/`, `logs/` |
| Cache | `~/.cache/shoal/` (prompt runtime versions) |
| Shell integration | `<prefix>/share/shoal/shell/bash/`, `.../zsh/` |
| systemd unit | `<prefix>/lib/systemd/user/shoald.service` |

Config, state and cache use `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` when set to absolute paths; the table shows their defaults under an absolute `HOME`. Empty or relative overrides are ignored, following the [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir-spec/latest/). Path helpers do not create directories. Runtime paths require an absolute `XDG_RUNTIME_DIR`; callers that do not need a daemon socket can resolve config/state/cache without it.

## 3. Data model

The whole workspace is one tree of plain data in `shoal-model`, shared by daemon, window and CLI. The daemon holds the only writable copy; everyone else receives full copies.

```rust
// Ids are u64 inside and prefixed strings on the wire: "s3", "t12", "p40", "x7".
// All four kinds share one counter, `next_id`; ids are never reused and the counter is persisted.
pub struct SessionId(u64); pub struct TabId(u64); pub struct PaneId(u64); pub struct SplitId(u64);

// jiff::Timestamp, serialized as an RFC 3339 UTC string ("2026-10-24T14:02:11Z").
// The model never reads the clock: callers pass the current time in.
pub type Timestamp = jiff::Timestamp;

pub struct Workspace {
    pub rev: u64,                       // +1 on every change
    pub next_id: u64,                   // the next id to hand out, of any kind
    pub sessions: Vec<Session>,         // in the user's order
    pub panes: BTreeMap<PaneId, Pane>,  // every pane in any tab's layout, keyed by id
}

pub struct Session {
    pub id: SessionId,
    pub name: String,           // unique; first is "main", then "2", "3", ...
    pub tabs: Vec<Tab>,         // never empty: closing the last tab closes the session
    pub active_tab: TabId,
    pub created_at: Timestamp,
    pub last_active_at: Timestamp,
    pub windows: u32,           // runtime only (not persisted): windows showing it now
}

pub struct Tab {
    pub id: TabId,
    pub name: Option<String>,   // set by the user; None = automatic title
    pub layout: Layout,
    pub focused: PaneId,
    pub zoomed: Option<PaneId>,
}

pub enum Layout {
    Pane(PaneId),
    Split {
        id: SplitId,
        axis: Axis,             // Row = side by side, Column = stacked
        ratio: f32,             // share of `first`, clamped to 0.1..=0.9
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

pub struct Pane {
    pub id: PaneId,
    pub kind: PaneKind,
    pub size: GridSize,         // { cols: u16, rows: u16 }
    pub cwd: Option<PathBuf>,
    pub program: Option<String>,// foreground process name, e.g. "nvim"
    pub title: String,          // computed display title (section 4.3)
    pub status: PaneStatus,
    pub bell: bool,             // rang since it was last focused
    pub restored_from: Option<Timestamp>,
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaneKind {
    Shell { argv: Vec<String> }, // empty = the user's shell
    // reserved, do not implement in M1: Agent { .. } (M3), Markdown { .. } (M6)
}

#[serde(tag = "state", rename_all = "snake_case")]
pub enum PaneStatus {
    Running,
    Exited { code: Option<i32>, signal: Option<i32> },
    // reserved: Working, Idle, NeedsInput (agents, M3)
}
```

**Layout operations** are pure functions in `shoal-model`, each covered by unit tests and property tests (random sequences of operations must keep every invariant).

| Function | What it does |
| --- | --- |
| `split(tab, pane, Right \| Down, new, split_id)` | Replaces the pane with a 50/50 split of old and new; focuses the new pane and clears zoom. The caller supplies both fresh ids, allocated from `next_id`. |
| `close(tab, pane)` | Replaces the parent split with the sibling; returns the pane to focus, or `None` if it was the last pane (the caller then closes the tab). Clears `zoomed` if needed. |
| `rects(layout, area, gap)` | Turns the tree into pixel rectangles. Rectangles never overlap and, with the gaps, exactly fill the area. |
| `neighbor(rects, pane, Left \| Right \| Up \| Down)` | The closest pane in that direction that overlaps on the other axis; ties go to the larger overlap, then the top-left one. |
| `resize(layout, pane, direction, step)` | Moves the edge of the nearest ancestor split on that axis, so the focused pane grows toward `direction`. |
| `set_ratio(layout, split_id, ratio)` | Used by mouse drags on a divider. |
| `toggle_zoom(tab, pane)` | Zoomed pane fills the tab; the others keep running at their old size. |
| `validate(workspace)` | Every pane in `panes` is stored under its own id, appears in exactly one tab's layout, and every layout pane is in `panes`; ids are unique across all four kinds (they share one counter) and all below `next_id`; `active_tab` points inside its session; `focused` and `zoomed` point inside their tab; every session has a tab; session names are unique; every split `ratio` is within 0.1..=0.9 (NaN fails). Runs after every change in debug builds. |

**Layout operation details (T03, settled 2026-10-25)**

- The caller owns workspace bookkeeping: allocating globally fresh ids, inserting or
  removing pane records, close cascades and revision changes. The operations mutate
  only the supplied tab or layout. Mutating operations return an error for missing
  targets or invalid arguments without changing it; `neighbor` returns `None` for
  a missing pane or absent neighbor. New pane and split ids are checked for numeric
  collisions within the supplied tab; freshness across the workspace is the caller's
  contract.
- Rectangles use nonnegative integer pixel coordinates and extents (`u32`); an area
  whose right or bottom coordinate overflows is rejected. At each split, subtract
  the gap, floor the first child's share, and give the remainder to the second.
  Clamp the gap to that split's available extent, so tiny or zero areas still tile
  exactly (some panes can have zero extent). `rects` always computes the underlying
  layout. For zoom, the caller displays only the zoomed pane at the full tab area;
  hidden panes keep their previous terminal sizes.
- Neighbor distance is the separation between facing edges in the requested
  direction. Candidates must lie beyond that edge and have positive overlap on the
  other axis. Ties use greater overlap, then topmost and leftmost position. Empty
  rectangles have no neighbor and are not candidates. There is no wraparound.
- Resize `step` is a nonnegative fraction of the selected split's usable extent,
  after its gap, not a cell count. Walk toward the root to find the nearest ancestor
  on the requested axis with the pane on the side that can grow toward the requested
  direction. Increase the ratio for right/down growth of `first`, decrease it for
  left/up growth of `second`, and clamp using `Layout::RATIO_RANGE`. No eligible
  ancestor, a zero step, or an already clamped ratio means no change; a clamped
  ancestor does not cause a more distant divider to move. Callers convert protocol
  `cells` to this fraction using geometry and cell dimensions. Negative or nonfinite
  steps and nonfinite `set_ratio` values are errors; finite ratios are clamped.
- Zoom focuses its target. Toggling the same zoomed pane clears zoom; toggling a
  different pane switches zoom. A single-pane tab can be zoomed and unzoomed.
- Close preserves surviving focus and zoom. When the focused pane closes, focus
  moves to the first leaf of its sibling subtree (first child before second).
  Closing a zoomed pane clears zoom. Closing the last pane clears zoom and returns
  `None`; the caller must remove that tab immediately, since `Layout` has no empty
  variant. An unknown pane is an error, distinct from closing the last pane.

## 4. Daemon behaviour

### 4.1 Starting and staying single

1. Create `$XDG_RUNTIME_DIR/shoal/` with mode 0700.
2. Take a non-blocking `flock` on `shoald.lock`. If another daemon holds it, log "already running" and exit 0.
3. Remove a stale socket file, bind `shoald.sock`, set mode 0600. Accept only peers whose `SO_PEERCRED` uid matches ours.
4. Load config and theme (the daemon needs theme colors to answer programs that ask the terminal for its colors).
5. Restore the saved workspace (section 6), then start accepting connections.

**Settled details (T10, 2026-10-25).** An existing runtime folder must be a directory owned by the daemon's user, or `shoald` exits 1. Its mode is reset to 0700. The lock file is created 0600 and never removed. A peer with another uid is closed without a reply. On stop, the socket is removed *before* the lock is released, so a daemon that takes the lock next never loses its fresh socket. A start error exits 1 with `shoald: <reason>` on stderr and in the log. A persistent accept error, such as running out of descriptors, pauses accepting for 100 ms.

**Config at startup (T11, 2026-10-25).** Step 4 never stops the daemon. An unreadable or invalid `config.toml` means defaults, and theme problems fall through to the next color source (§9.2). Each problem is logged; T12 adds the watch and `daemon.notice`.

**How it gets started.** `shoald.service` is a systemd user unit (`Type=simple`, `Restart=on-failure`, `TimeoutStopSec=10`). Any client that can't connect runs `systemctl --user start shoald.service` and polls the socket for up to 2 s. If that fails or systemd is absent, it spawns `shoald` detached (`setsid`, stdio to `/dev/null`). Shells run inside the service's cgroup, so closing windows or the Hyprland session doesn't touch them, and stopping the service ends them cleanly.

### 4.2 Pane runtime

**Spawning a shell.** Open a PTY at the pane's size (portable-pty). The command is the pane's `argv`, else `shell.program` from config, else `$SHELL`, else the passwd entry, else `/bin/bash`, started as an interactive shell. Working directory: the one requested, else the source pane's current one (for splits and new tabs), else `$HOME`.

**Environment.** Start from the daemon's environment. Overlay the latest connected window's values for a fixed allowlist, because a daemon started by systemd may hold stale session variables: `WAYLAND_DISPLAY`, `DISPLAY`, `HYPRLAND_INSTANCE_SIGNATURE`, `SSH_AUTH_SOCK`, `DBUS_SESSION_BUS_ADDRESS`, `XDG_CURRENT_DESKTOP`, `XDG_SESSION_ID`. Then set `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=shoal`, `TERM_PROGRAM_VERSION`, `SHOAL_PANE=<pane id>`, `SHOAL_SOCKET=<socket path>`, plus the shell-integration variables from section 8.

**The pane thread.** Each pane gets one OS thread, named `pane-<id>`, that owns the PTY and the `shoal-term` terminal. It waits with `poll` on the PTY and an eventfd for commands, so it uses no CPU while quiet.

**The pane's terminal.** When it is created, the terminal enables continuation tracking with a 4096-byte cap (`set_continuation_max_bytes(4096)`), so a snapshot taken mid-sequence carries the unfinished escape or UTF-8 bytes (5.5). It turns on Unicode grapheme clustering (DEC mode 2027) before the shell starts, as Ghostty does, so a cluster such as an emoji with a skin tone takes one cell, two columns wide (settled with the user 2026-10-25, T08). A program can still turn it off with `CSI ? 2027 l`; the mode travels in snapshots, so mirrors match. It installs `on_pty_write` and the other reply callbacks, which write to the PTY, and the callbacks the metadata needs (bell, title, directory, prompt marks). The pane's terminal is the only terminal in Shoal with reply callbacks; mirrors and scratch terminals have none.

- **Output:** read up to 64 KiB, feed the terminal, append to the pane's stream with the next sequence number, wake subscribers. Batch into one frame per pane at most every 4 ms or 64 KiB.
- **Replies:** when a program asks the terminal something (cursor position, colors, device attributes), the reply goes straight back into the PTY from this thread.
- **Commands:** `Write(bytes)`, `Resize(size)`, `Snapshot(reply)`, `Subscribe`, `Unsubscribe`, `Capture`, `Kill`.
- **Kill:** `SIGHUP` to the process group, then `SIGKILL` after 2 s.
- **Exit:** on EOF, collect the exit status and set `Exited`. With `close_pane_on_exit = true` (default) the workspace actor closes the pane, which may close its tab and session. Guard: if a pane exits within 1 s of starting with a non-zero code, keep it open showing the error.

**Settled details (T11, 2026-10-25).**

- *Starting:* `shell.program` names one program, with no arguments. A shell gets no arguments, so it is interactive and not a login shell. The passwd step reads `/etc/passwd` for the daemon's uid. A requested directory that isn't one falls back to `$HOME`. A program that can't be started is a spawn error for the caller; nothing is shown in a pane. The thread is named `pane-<number>` (`pane-40`), which fits Linux's 15-character thread names.
- *Replies:* the pane terminal answers what Ghostty answers: DA1 `CSI ? 62 ; 22 c`, DA2 `CSI > 1 ; 0 ; 0 c`, DA3 unit id 0, XTVERSION `shoal <version>`, XTGETTCAP `TN` as `xterm-256color`, and XTWINOPS and mode 2048 sizes in cells with zero pixels. The color scheme report (`CSI ? 996 n`) says dark when the theme background's luminance is below half. Color queries (OSC 4, 10–12) get the theme's colors (§9.2) unless the program set its own. ENQ and clipboard reads get no answer, and window resize requests are ignored. Mode 2027 is the reset default, so a full reset (`ESC c`) keeps it on.
- *Stream:* sequence numbers start at 1 and number output items and resizes alike. Output that arrives 4 ms or more after the last item goes out at once, so echo isn't delayed. Otherwise it waits until 4 ms have passed, and an item never holds more than 64 KiB. A snapshot or a new subscriber first numbers any output still waiting, so the snapshot at `N` holds exactly the items up to `N`. A resize to the current size is ignored.
- *Input:* the PTY is non-blocking. Input and replies queue in order and are written as the shell reads, so a shell that stops reading never stalls output.
- *Kill:* `SIGHUP` goes to the shell's process group and the PTY's foreground group, as a hangup does, and `SIGKILL` follows to both after 2 s. Dropping a pane's handle kills it the same way, and the thread ends once the shell has.
- *Exit:* after EOF (`EIO`), the thread waits on a pidfd for the status, so a slow exit never blocks it. A process outside those groups that keeps the terminal open keeps the pane running. The guard counts only a non-zero exit code; a death by signal is not an early failure. An exited pane's thread keeps answering snapshots and captures, ignores input, and resizes only its terminal.

### 4.3 Pane metadata

Event-driven, never polled on a timer. Triggers: output followed by 300 ms of quiet, an Enter key sent to the pane, an OSC 7 directory report, an OSC 0/2 title change, a bell. Throttle to 4 updates per second per pane.

- **Program:** `tcgetpgrp` on the PTY gives the foreground process group; `/proc/<pgid>/comm` gives its name. If it's the shell itself, `program` is empty.
- **Directory:** the latest OSC 7 report; otherwise `readlink /proc/<pgid>/cwd`.
- **Title (`tabs.title = "auto"`):** the user's tab name if set; otherwise a running program's name; otherwise the directory's last part (`~` for home). With `tabs.title = "program"`, an OSC 0/2 title wins when present. A tab shows its focused pane's title.
- **Bell:** sets `bell = true` unless the pane is focused in some window; focusing clears it.

### 4.4 Sessions and windows

- A window declares its session with `window.show_session`. The daemon counts windows per session and drops the count when the connection closes.
- **Smart open** (`session.open_smart`, what a plain `shoal` launch calls): pick the session with no windows that was active most recently. If every session is on screen, create a new one named with the lowest free number from 2 (the very first is `main`), with one tab and one shell in the requested directory or `$HOME`. Done inside the workspace actor, so two windows launched at once never grab the same session.
- **Focus is shared.** Active tab and focused pane belong to the session, so two windows on it stay in sync.
- **Closing:** last pane closes its tab; last tab closes its session. Closing a window closes nothing.

### 4.5 Shutdown

On `SIGTERM`, `SIGINT` or `daemon.shutdown`: persist everything (section 6), `SIGHUP` every pane, wait up to 2 s, `SIGKILL` stragglers, remove the socket, exit 0. Must finish within 3 s with 50 panes.

### 4.6 Logging

`tracing` to `~/.local/state/shoal/logs/shoald.log`, rotated daily, kept 7 days, level from `SHOAL_LOG` (default `info`). Never log pane output or typed input.

**Settled details (T10, 2026-10-25).** The current file is always `shoald.log`. The first line written on a later UTC day renames it to `shoald.log.<YYYY-MM-DD>` (the day its lines belong to; a file left from an earlier day rotates under its modification date), and rotated files more than 7 days older than today are deleted. Rotation happens on writes, never on a timer. `SHOAL_LOG` is a level name (`trace`, `debug`, `info`, `warn`, `error`, `off`); anything else logs a warning and uses `info`. When stderr is a terminal, as under `just dev-daemon`, lines also go there; under systemd or the detached spawn they don't.

## 5. Protocol

Protocol version 1. Every change updates this section in the same commit. The T00 spike revised version 1 (binary snapshots, `core` in `hello`) before anything implemented it. T04's user-confirmed wire clarifications below also precede the first implementation, so it stays version 1.

### 5.1 Frames

```
frame = len:u32 | kind:u8 | payload        len = bytes of kind + payload; integers little-endian

0x01 JSON      both ways        UTF-8 JSON object (5.2)
0x10 OUTPUT    daemon → client  pane:u64 seq:u64 bytes...
0x11 SNAPSHOT  daemon → client  pane:u64 seq:u64 cols:u16 rows:u16 bytes...
0x12 RESIZED   daemon → client  pane:u64 seq:u64 cols:u16 rows:u16
0x20 INPUT     client → daemon  pane:u64 bytes...
```

Maximum declared `len` is 64 MiB (kind + payload); the four-byte length prefix is additional. A bigger frame or unknown kind closes the connection with a logged protocol error. Binary frames carry the numeric pane id (`3`), JSON the prefixed form (`"p3"`). SNAPSHOT `bytes` are a libghostty binary snapshot (5.5). The frame limit bounds bytes on the wire, not what decoding a snapshot allocates; 5.5 sets the decoding limits.

### 5.2 JSON messages

JSON-RPC 2.0 shapes: a request has `id`, `method`, `params`; a response has `id` plus `result` or `error {code, message, data?}`; a notification has `method` and `params` but no `id`. The daemon handles one connection's requests in order.

**Wire details (T04, settled 2026-10-25).** Every JSON message has `jsonrpc: "2.0"`. Request ids are strings or integers; an error response may have `id: null` when no valid request id is available. A success response has a valid id and exactly one `result`; an error response has exactly one `error`. Methods with no parameters emit `params: {}` and also accept omitted `params`. Unknown method names remain decodable so the daemon can return -32601; invalid method parameters are distinct from an invalid envelope. JSON batches are not objects and are outside this protocol.

### 5.3 Handshake

The first frame must be `hello`; anything else gets an error and the connection closes. The window sends the allowlisted environment variables from 4.2 in `env`.

```
→ {"jsonrpc":"2.0","id":1,"method":"hello","params":{"protocol":1,"client":"gui","version":"0.1.0","core":"<core id>","env":{"WAYLAND_DISPLAY":"wayland-1"}}}
← {"jsonrpc":"2.0","id":1,"result":{"protocol":1,"daemon_version":"0.1.0","core":"<core id>","pid":4312}}
```

**Connection rules (T10, 2026-10-25).** Before a successful `hello`, any other frame (request, notification, response, invalid envelope or INPUT) gets error 1002 `hello must come first`, with the request's id or null, and the connection closes. A `hello` that fails (1003 protocol mismatch, 1004 bad parameters) also closes it. After `hello`: an invalid envelope gets -32600 with a null id and the connection stays open, a second `hello` gets 1002, and client notifications and responses are ignored. OUTPUT, SNAPSHOT or RESIZED from a client is a protocol error and closes the connection, like an unknown kind.

**Core identity.** Binary snapshots decode reliably only between identical libghostty builds (5.5), so `core` names the exact terminal core a binary was built with. `shoal-term` builds the string at compile time from the full pinned revisions and the build settings that can change terminal behaviour:

```
ghostty=<40-hex commit>;libghostty-rs=<40-hex commit>;version=<build_info::version_string>;optimize=<mode>;cpu=<model>;simd=<bool>;kitty_graphics=<bool>;tmux=<bool>
```

Two `core` strings are compatible only when they are byte-for-byte equal. The window sends `core`; the CLI omits it, because it never decodes snapshots. The daemon always returns its own.

**Mismatches.**

- **Different `protocol`:** `hello` fails with error 1003 (`data: {daemon_protocol, pid}`) and the connection closes.
- **Same `protocol`, different `core`:** `hello` succeeds and every other method works, but `pane.attach` fails with error 1003 (`data: {daemon_core}`). The window never decodes a snapshot from a different core.

In both cases the window shows a banner: "shoald is from a different build. Restart it to show panes. Running programs will end; layout and scrollback come back." It has a **Restart daemon** button. Nothing restarts until the user clicks it. Then:

1. **Stop the old daemon through its normal shutdown (4.5),** which persists everything before ending the panes. With the same protocol, send `daemon.shutdown`. With a different protocol, JSON can't be trusted: if `systemctl --user show -p MainPID shoald.service` reports `pid`, run `systemctl --user stop shoald.service`; otherwise send `SIGTERM` to `pid`.
2. **Wait up to 5 s for the socket to disappear.** If it doesn't, show "shoald didn't stop. Run `shoal daemon stop`." and stop there. Never `SIGKILL`: that loses up to 30 s of scrollback.
3. **Start the daemon as in 4.1 and reconnect.** It restores the workspace (6.4) and starts fresh shells.

### 5.4 Methods

| Method | Params | Result |
| --- | --- | --- |
| `hello` | `protocol`, `client` (`gui` \| `cli`), `version`, `core?` (5.3), `env` | `protocol`, `daemon_version`, `core`, `pid` |
| `workspace.get` | none | `rev`, `workspace` |
| `workspace.subscribe` | none | `{}`, then `workspace.changed` notifications |
| `session.open_smart` | `cwd?` | `session` (rule in 4.4) |
| `session.create` | `name?`, `cwd?` | `session` |
| `session.rename` | `session`, `name` | `{}`; error 1002 if the name is taken |
| `session.close` | `session` | `{}` |
| `window.show_session` | `session` | `{}` |
| `window.set_focused` | `focused` (bool) | `{}` |
| `tab.create` | `session`, `cwd?`, `index?`, `argv?` | `tab`, `pane` |
| `tab.close` | `tab` | `{}` |
| `tab.rename` | `tab`, `name` (null clears) | `{}` |
| `tab.move` | `tab`, `index` | `{}` |
| `tab.activate` | `tab` | `{}` |
| `pane.split` | `pane`, `direction` (`right` \| `down`), `cwd?` | `pane` |
| `pane.close` | `pane` | `{}` |
| `pane.focus` | `pane` | `{}` (also activates its tab) |
| `pane.resize` | `pane`, `direction`, `cells` | `{}` |
| `split.set_ratio` | `split`, `ratio` | `{}` |
| `pane.zoom` | `pane`, `zoomed` (bool) | `{}` |
| `pane.attach` | `pane` | `seq`, `cols`, `rows`, sent after a SNAPSHOT frame (5.5) |
| `pane.detach` | `pane` | `{}` |
| `pane.set_size` | `pane`, `cols`, `rows` | `cols`, `rows` now in force |
| `pane.send_text` | `pane`, `text` | `{}` (CLI and tests) |
| `pane.capture` | `pane`, `format` (`text` \| `vt`), `scrollback` (bool) | `data` (vt is base64) |
| `daemon.status` | none | `version`, `pid`, `uptime_s`, `sessions`, `panes`, `last_persist_at` |
| `daemon.persist` | none | `{}` once written (tests) |
| `daemon.shutdown` | none | `{}`, then the daemon exits (4.5) |

`index`, `cells`, counts (`sessions`, `panes`) and `uptime_s` are nonnegative integers; uptime is in whole seconds. `last_persist_at` is an RFC 3339 UTC timestamp, or null before any save. `session`, `tab` and `pane` results above are prefixed ids, as in 5.7.

**Notifications from the daemon**

- `workspace.changed {rev, workspace}`: the whole workspace after any change, at most one per 16 ms per connection.
- `daemon.notice {level, message, key?}`: banner-worthy problems (config error, theme folder not found, a restore that skipped a broken file); `level` is `info`, `warn` or `error`.
- `pane.detached {pane, reason}`: the daemon stopped streaming a pane to this connection without being asked. M1's only reason is `mid_sequence` (5.5).
- `pane.event {pane, event}`: reserved for agent panes (M3). Not sent in M1.

**Error codes:** `-32600` invalid request, `-32601` unknown method, `1001` not found, `1002` invalid state, `1003` version mismatch (data: `daemon_protocol` and `pid` from `hello`, `daemon_core` from `pane.attach`; 5.3), `1004` bad params, `1005` internal.

### 5.5 Attaching to a pane

A snapshot is libghostty's binary snapshot (`Terminal::encode_snapshot`). It holds the active and inactive screens, scrollback, styles, cursor and saved cursor, modes, colors, title, prompt marks, and unfinished parser state. The T00 spike matched all 31 checked cases with it, where the formatter lost state (`docs/spike-00.md`). Its format (version 1) carries no compatibility guarantee, so a window decodes only snapshots from a daemon with the same `core` (5.3).

1. The client sends `pane.attach`.
2. On the pane's thread, in one step: encode a snapshot, note the current sequence number `N`, and register the subscriber for items after `N`. If the parser is mid-sequence, see "Mid-sequence snapshots" below.
3. The daemon sends `SNAPSHOT(pane, N, cols, rows, bytes)`, then the JSON result `{seq: N, cols, rows}`, then `OUTPUT` and `RESIZED` frames numbered `N+1`, `N+2`, ... A snapshot that would exceed the frame limit fails the attach with error 1002 (`data: {"reason": "too_large"}`).
4. The client decodes the snapshot into a fresh mirror within the decoding limits below, sets up its callbacks, then feeds it every item in order. A `RESIZED` frame resizes the mirror at exactly that point.
5. A later `SNAPSHOT` for an attached pane means "replace your mirror". A sequence gap means the mirror is stale: the client sends `pane.attach` again.

**Mid-sequence snapshots.** The 4096-byte continuation cap (4.2) does not make a snapshot always available. If the parser is inside an escape sequence longer than the cap, encoding fails until the parser returns to ground. A long OSC 52 clipboard write or a Kitty graphics transfer can do this. T00 reproduced it with a 5,000-byte unfinished OSC. Encoding worked again after the terminator. When an attach finds the snapshot unavailable:

- The pane thread keeps reading, feeding and numbering output as usual. The shell is never blocked, other subscribers see no change, and the waiting subscriber gets nothing yet.
- It feeds new output with `vt_write_until_ground`. At the first ground point it splits that read in two. The prefix becomes item `N`, the snapshot is encoded right after it, and the rest becomes item `N+1`. The snapshot at `N` holds every byte up to the split, so nothing is lost or delivered twice. Each waiting attach costs one more encode, with no polling.
- If the parser hasn't reached ground within 2 s of the request, `pane.attach` fails with error 1002 (`data: {"reason": "mid_sequence"}`). The pane shows "This pane is in the middle of an escape sequence." with a **Retry** button. The window doesn't retry by itself. The daemon still has all the output, so a later retry shows everything.
- The 2 s deadline is a `poll` timeout that exists only while an attach waits. An idle pane has no timer.
- Any other encode error fails the attach with error 1005 and is logged.

**Decoding limits.** The 64 MiB frame limit bounds bytes on the wire, not what decoding allocates. The client decodes within these limits and checks each one:

- `cols` and `rows` in the SNAPSHOT header are each between 1 and 1,000. The decoded terminal's `cols()` and `rows()` equal them.
- The decoder accepts at most the daemon's 4096-byte continuation (`set_max_continuation_bytes(4096)`; libghostty's default is 65 MiB). It does not retain continuation tracking.
- The decoder and the terminal it returns use a counting allocator (`Decoder::new_with_alloc`). Allocations during decoding are capped at 256 MiB; a refused allocation fails the decode. The cap lifts once FINISH is validated. From then on, `scrollback.lines` bounds the mirror like any terminal.
- **Terminal pages bypass that allocator (T07, settled 2026-10-25).** Ghostty maps page memory straight from the OS (`page.zig`, "You cannot use custom allocators with this structure"), so the counting allocator sees only the small rest. Decoding 200 × 60 with 10,000 lines grew resident memory by 15.6 MiB, while the allocator saw 0.26 MiB. A 0.8 MiB snapshot of 1000 × 60 with 99,000 lines is within every other limit, yet needs about 390 MiB of pages. Page memory is therefore bounded by declared cells: the sum over both screens of (history rows + `rows`) × `cols` is at most 32 Mi (33,554,432) cells, about 130–260 MiB at the measured 4–8 bytes per cell. The default 10,000-line scrollback fits at 1000 columns, and 100,000 lines fit up to 320 columns.
- The client decodes incrementally (`Decoder::ready`, then `next` until FINISH). The declared history of each screen (`history_rows_primary`, `history_rows_alternate`) is at most 100,000 rows, and the declared cells are within the budget above. At the pinned bindings these are readable through each page's `Progress`, so the client checks them at the first history page and stops there if they're over. That happens before any history page is mapped: the refused 1000 × 60 snapshot grew resident memory by 0.4 MiB.
- Decoding reaches FINISH and consumes the whole snapshot. No bytes may follow FINISH in the frame. A one-shot decode accepts trailing bytes silently, so the client decodes from a reader that counts the bytes taken and checks that the count equals the snapshot length. In T00 the decoder read exactly up to FINISH and no further.

Any failure, including an allocation refusal, is an error, never a panic. The window drops the mirror and re-attaches once. A second failure shows an error banner in the pane with a **Retry** button.

**Mirror callbacks.** Callbacks are not part of a snapshot, so a decoded terminal has none. A mirror never installs `on_pty_write` or any other callback that answers the program (`on_enquiry`, `on_xtversion`, `on_size`, `on_color_scheme`, `on_device_attributes`, `on_clipboard_read`). libghostty then drops every reply, and only the daemon answers queries (decision 4, rule 1). A mirror installs no bell, title, directory, clipboard-write or notification callbacks either, because the daemon owns that metadata. Callbacks that only affect drawing, such as `on_render_hold`, are allowed. The mirror sets its scrollback limit from config and finishes setup before the first live byte is fed. The restored parser state can complete a query on the very next byte.

**Backpressure:** each subscriber has an 8 MiB queue. On overflow the daemon drops the queue and sends a fresh `SNAPSHOT` at the current sequence number, under the same mid-sequence rule. If the 2 s deadline passes, the daemon unsubscribes the window and sends `pane.detached {pane, reason: "mid_sequence"}`, and the pane shows the same Retry banner. A slow window never slows the shell or other windows.

### 5.6 Input and size

- `INPUT` frames carry bytes the window has already encoded: keys through `shoal-term`'s key encoder using the mirror's current modes, and pastes wrapped in bracketed-paste markers when the program asked for them. The daemon writes them to the PTY in order and ignores input to exited panes.
- `pane.set_size` is applied only from the connection that most recently reported `window.set_focused {focused: true}`; others get the size in force back and draw the pane at that size, padded or clipped.

### 5.7 Example session

```
→ session.open_smart {cwd: "/home/parker"}      ← {session: "s1"}
→ window.show_session {session: "s1"}           ← {}
→ workspace.subscribe                           ← {} ; ← workspace.changed {rev: 42, ...}
→ pane.attach {pane: "p3"}                      ← SNAPSHOT(3, 1880, 120, 40, ...) ; ← {seq: 1880, cols: 120, rows: 40}
                                                ← OUTPUT(3, 1881, ...)
→ INPUT(3, "ls\r")                              ← OUTPUT(3, 1882, ...)
```

## 6. Persistence and restore

Reboot restore brings back sessions, tabs, splits, each pane's directory and its old scrollback, then starts a fresh shell under a dim divider. Running programs are not restored.

### 6.1 Files

```
~/.local/state/shoal/                 0700
  workspace.json                      0600  layout and pane metadata
  panes/p4/scrollback.vt.zst          0600  zstd-compressed terminal bytes
  panes/p4/                                 reserved for per-pane data (agent transcripts in M3)
  logs/shoald.log
```

### 6.2 `workspace.json`, format 1

```json
{
  "format": 1,
  "saved_at": "2026-10-24T14:02:11Z",
  "shoal_version": "0.1.0",
  "next_id": 57,
  "sessions": [{
    "id": "s1", "name": "main",
    "created_at": "2026-10-20T09:00:00Z", "last_active_at": "2026-10-24T14:01:58Z",
    "active_tab": "t2",
    "tabs": [{
      "id": "t2", "name": null, "focused": "p4", "zoomed": null,
      "layout": { "split": { "id": "x3", "axis": "row", "ratio": 0.6,
                  "first": { "pane": "p4" }, "second": { "pane": "p5" } } }
    }]
  }],
  "panes": {
    "p4": { "kind": { "kind": "shell", "argv": [] },
            "cwd": "/home/parker/code/shoal", "size": { "cols": 120, "rows": 40 },
            "program": "nvim", "scrollback": "panes/p4/scrollback.vt.zst" }
  }
}
```

A format change bumps `format` and ships a migration from every older format, with a test that loads a saved fixture of each.

### 6.3 When it writes

- **Layout:** after any structural change, debounced to once per second, and on shutdown.
- **Scrollback:** for a pane whose output changed, after 2 s of quiet, at most once every 30 s; and every pane on shutdown. The pane thread runs the formatter (or the encoder, below); decoding, compressing and writing happen on a blocking worker, never on the workspace actor.
- **What is saved:** formatter output (terminal bytes) for the main screen plus scrollback up to `scrollback.lines`, with colors and styles. If a full-screen program is up, only the main screen underneath is saved. The formatter sees only the active screen, so in that case the pane thread encodes a binary snapshot (5.5) instead and hands the bytes to the blocking worker. The worker decodes them into a scratch terminal with no callbacks, leaves the alternate screen there, formats it, compresses and writes. To leave the alternate screen it sends `CSI ? 1049 l`, `CSI ? 1047 l`, `CSI ? 47 l`, because each mode's flag clears only through its own exit. T07 refined T00's finding here. `CSI ? 1049 l` alone does return a screen entered with 47 or 1047 to the primary screen and its history, but it leaves the 47 or 1047 flag set. T00's formatter emitted modes, so its replay re-entered the alternate screen. Shoal's persistence formatter emits no modes or palette, and the scratch terminal is left clean anyway. The live terminal is never changed. If the snapshot is unavailable mid-sequence (5.5), that write is skipped until the next trigger. On shutdown the previous file is kept.
- **Safely:** write `*.tmp`, fsync, rename over the old file, fsync the folder.
- **Cleanup:** a closed pane's folder is deleted at the next layout write. With `scrollback.persist = false`, no scrollback files are written and existing ones are deleted; the layout is still saved.

**Privacy note:** saved scrollback contains whatever you printed, secrets included. Files are owner-only and persisting can be switched off.

### 6.4 Restore on daemon start

1. No `workspace.json`: start empty. The first window's smart open creates `main`.
2. Unreadable file or unknown format: rename it to `workspace.json.broken-<timestamp>`, start empty, send a `daemon.notice` to the first window.
3. For each pane, build a terminal at the saved size and feed it the saved scrollback. A corrupt scrollback file is skipped with a note in the divider.
4. Make the old content inert: leave any alternate screen, reset styles and modes, show the cursor.
5. Write a dim divider line, e.g. `── restored · Fri Oct 24 14:02 · was running nvim ──` (program part only when one was running).
6. Start a fresh shell in the saved directory. If the folder is gone, use `$HOME` and add `(folder gone)` to the divider.
7. Ids continue from `next_id`. `rev` restarts at 1; clients always reload the workspace on connect.

**Budget:** 20 panes with 10,000 lines each restored in under 500 ms; shells start in parallel.

## 7. Window and rendering

### 7.1 The window

A GPUI window with `app_id` `dev.shoal.Shoal`, title `<session> — Shoal`, a transparent background and no client-side decorations; Hyprland tiles and blurs it. One window shows one session at a time. Running `shoal` starts `shoal-gui`, which calls `session.open_smart`; `shoal attach <name>` passes a session explicitly.

### 7.2 Layout and chrome

Sizes are logical pixels before scaling.

- **Top bar, 34 px.** Fill `darker_background` at the window opacity. Left: the session name as a chip (click or Ctrl+Shift+S for the switcher). Then the tabs: a terminal glyph and the title, at most 200 px with an ellipsis. Active tab: `foreground` text and a 2 px `accent` underline; inactive: `dark_foreground`; a ringing tab gets a small `accent` dot. A `+` button follows the tabs. Right: split-right and split-down buttons. Every button's tooltip shows its shortcut as key caps.
- **Pane area.** 8 px padding (`window.padding`), 6 px between panes (`window.gap`). Each pane is a rounded rectangle (10 px radius) filled with `background` at `window.opacity`, with 6 px inside the edge before the grid starts.
- **Focus.** The focused pane has a 1.5 px `accent` border; others a 1 px `lighter_background` border at 60% alpha. A tab with one pane shows no focus border.
- **Zoom.** The zoomed pane fills the pane area, and its tab shows a zoom glyph.
- **Dividers.** The gap between panes is the drag target: resize cursor, `split.set_ratio` at most 60 times a second, final value sent on release.
- **Grid size.** `cols = floor(inner width / cell width)`, rows likewise; leftover pixels split evenly as padding. The window sends `pane.set_size` when it changes, debounced 50 ms during a drag.

### 7.3 Drawing the grid

- **Mirrors.** Each attached pane has a mirror: a `shoal-term` terminal plus exactly one render state. A second render state on the same terminal would consume the first one's dirty flags. New bytes are fed in and a frame is requested; nothing redraws while nothing changes.
- **Per frame.** Only dirty rows are rebuilt, chosen by the render state's dirty flags: merged background rectangles, text, underline and strike lines, selection, cursor. Clean rows reuse their cached entries. The renderer produces an internal draw list first, so it is testable without a GPU and portable to the fallback toolkit.
- **Text.** Glyphs are placed cell by cell from a cache keyed by font, style and grapheme. A cell holding one codepoint uses its cached glyph. A cell holding a multi-codepoint grapheme cluster is shaped as one unit. Positions and widths always come from libghostty's cells, never from the shaper. There are no ligatures across cells. Changing font family, size or line height clears the glyph cache and every row cache. The T00 spike measured this design: per-run shaping missed the 4 ms full-redraw target, and cached per-cell glyphs met it (`docs/spike-00.md`).
- **Colors.** Default-background cells are not painted, so the pane's translucent fill shows. Explicit backgrounds are painted at `window.opacity`. Dim text is drawn at 60% alpha.
- **Fonts.** `font.family` (default JetBrains Mono), then the bundled Symbols Nerd Font Mono, then the system emoji font, then system fallback. Wide characters take two cells, as libghostty reports. The bundled font is embedded from Nerd Fonts v3.5.1. It is not under one license: its icon sets carry MIT, CC BY 4.0, OFL 1.1 and Apache 2.0 terms, and one set (Font Logos) has no license at all. T50 settles packaging and attribution (`docs/spike-00.md`, "Font provenance").
- **Drawn as shapes, not glyphs:** box drawing (U+2500–257F), block elements (U+2580–259F) and powerline separators (U+E0B0–E0B7).
- **Cursor.** The shape a program asks for, else `cursor.style`; color `cursor.color` (default `accent`). An unfocused pane shows a hollow block.
- **Links.** OSC 8 links and plain URLs on the hovered row underline while Ctrl is held; Ctrl+click opens them with `xdg-open`.
- **Images** (Kitty graphics) are not drawn in M1.

### 7.4 Input

- **Keys.** A chord matching a binding runs its action. Anything else goes to the focused pane: committed text (including input-method text) as UTF-8, other keys through `shoal-term`'s key encoder using the mirror's modes, sent as `INPUT` frames.
- **Paste.** Wrapped in bracketed-paste markers when the program enabled them; otherwise newlines become carriage returns.
- **Mouse.** If the program turned on mouse reporting, events are encoded and sent (hold Shift to select locally instead). Otherwise: click focuses, drag selects, double-click selects a word, triple-click a line, the wheel scrolls 3 lines per notch, touchpads scroll by pixel, middle-click pastes the primary selection.
- **Focus reports** are sent when a program asked for them and pane or window focus changes.

### 7.5 Scrollback and selection

- Scrolling is smooth (fractional rows). New output while scrolled up doesn't move the view; a "new output" pill appears bottom right. Typing jumps to the bottom.
- Selections are anchored to content. Selecting sets the Wayland primary selection; copy puts it on the clipboard (`selection.copy_on_select` does both).
- Ctrl+Shift+Up and Down jump to the previous or next prompt, using shell-integration marks.

### 7.6 Session switcher

A popover under the session chip: each session with its tab count and an "on screen" marker for sessions shown in another window. Type to filter, Enter to switch, a "New session" row, F2 to rename, Ctrl+Backspace to close (with confirm), Esc to dismiss.

### 7.7 Banners and confirmations

- `daemon.notice` and config errors show as a dismissable one-line banner at the top of the pane area.
- If the daemon connection drops: a "Reconnecting to shoald…" overlay, retries with backoff from 100 ms to 2 s, starting the daemon if needed. Mirrors are rebuilt from fresh snapshots.
- If the daemon's `protocol` or `core` differs from the window's: the banner and user-triggered restart in 5.3. Panes stay blank until the restart.
- If only `daemon_version` differs: a banner with a "Restart daemon" button that runs the same restart steps. Panes still attach.
- Closing a pane, tab or session where something other than the shell is running shows an inline bar: "nvim is running. Close it? Enter / Esc" (`behavior.confirm_close_running`).

### 7.8 Instant tab switching

The window keeps mirrors for the 8 most recently viewed tabs attached. Other tabs are detached and cost the window nothing.

## 8. Shell integration and the native prompt

The native prompt is a normal shell prompt computed by `shoal prompt` and installed by Shoal's shell integration. Shoal bundles the icon font and draws powerline shapes itself, so it needs no Starship, p10k or Nerd Font setup.

### 8.1 Loading the integration

`shell.integration` is `auto` (default), `manual` or `off`.

- **bash, auto:** start bash with `--posix` and `ENV` pointing at `share/shoal/shell/bash/inject.bash`. That file switches posix mode off, unsets `ENV`, sources the startup files bash normally reads for an interactive shell (`/etc/bash.bashrc`, `~/.bashrc`), then loads `shoal.bash`. Same trick Ghostty uses.
- **zsh, auto:** set `ZDOTDIR` to `share/shoal/shell/zsh/`, remembering the user's value in `SHOAL_ORIG_ZDOTDIR`. The shim `.zshenv` restores the original `ZDOTDIR`, sources the user's `.zshenv`, and arranges for `shoal.zsh` to load after the user's `.zshrc`.
- **manual:** `eval "$(shoal shell-init bash)"` or `eval "$(shoal shell-init zsh)"` in the rc file. A guard variable stops double loading.
- **Other shells** run without integration in M1; titles and directories fall back to `/proc` (4.3).

### 8.2 What the integration does

- Prompt marks (OSC 133): `A` at prompt start, `B` where input begins, `C` before command output, `D;<exit code>` when it finishes.
- Reports the directory (OSC 7) at every prompt.
- Times each command (`EPOCHREALTIME` in bash 5 and zsh).
- With `prompt.mode = "native"` (default): sets `PS1` (bash) or `PROMPT` (zsh) at every prompt from `shoal prompt`. Existing `PROMPT_COMMAND` and `precmd` hooks still run, first. With `prompt.mode = "shell"`: marks and directory only.

### 8.3 `shoal prompt`

```
shoal prompt --shell bash|zsh --status <exit> --duration-ms <n> --jobs <n> --cols <COLUMNS>
```

Prints one line ready for the shell, with non-printing escapes wrapped (`\[ \]` for bash, `%{ %}` for zsh), followed by the prompt character. It reads the same theme as the window.

| Segment | Shows | Comes from | Hidden when |
| --- | --- | --- | --- |
| `user` | `parker`, plus `@host` over SSH | `$USER`, `$SSH_CONNECTION`, hostname | removed from `prompt.segments` |
| `path` | `~/code/shoal`, last 3 parts after `…/` | `$PWD` | never |
| `git` | branch icon, `main`, `●` when dirty, `⇡2 ⇣1`; short hash when detached | `git status --porcelain=v2 --branch` with an 80 ms timeout; on timeout, branch from `.git/HEAD` plus `?` | not in a repository |
| `runtime` | language icon and version, e.g. Node `22.11` | a marker file (`package.json`, `.nvmrc`, `pyproject.toml`, `.python-version`, `Cargo.toml`, `go.mod`, `Gemfile`); version from `<tool> --version`, cached in `~/.cache/shoal/runtimes.json` by binary path and mtime | no marker file |
| prompt character | `❯` in `accent`; `red` after failure; `took 12s` when a command ran over 2 s | arguments | never |

**Styles:** `powerline` (default): `user` on `magenta`, `path` on `blue`, `git` on `green` (`yellow` when dirty), `runtime` on `cyan`, each with `background`-colored text, joined by arrow separators. `lean`: same colors as text, no blocks.

**Speed budget:** under 5 ms p95 outside a repository, under 30 ms inside a ~5,000-file repository. Hard cap 100 ms: a segment still running is skipped and shows `?`.

**Settled details (T31, 2026-10-25).**

- *Arguments:* `--shell` is required; the others default to 0 and take `--name value` or `--name=value`. Bad usage exits 2. `--jobs` above 0 shows `✦<n>` in `cyan` before the prompt character (settled with the user). `--cols` is accepted and ignored: M1 prompts don't change with width.
- *Line:* segments in `prompt.segments` order, then ` took 12s` in `yellow` (`12s`, `3m 4s`, `1h 5m`; strictly over 2 s), then `✦<n>`, then the prompt character and one space. Colors are 24-bit SGR from the same resolved palette as the window (`[theme.colors]`, theme files, built-in), so a theme switch applies from the next prompt. Icons are Nerd Font code points: separator U+E0B0, branch U+E0A0, Node U+E718, Python U+E73C, Rust U+E7A8, Go U+E627, Ruby U+E739.
- *Escaping:* literal text escapes `\`, `$` and `` ` `` with a backslash for both shells, and zsh also doubles `%`. Nothing in a path or branch name ever runs, whatever the user's `promptvars` or `prompt_subst` settings. Text shows exactly under bash's default `promptvars` and under zsh's `prompt_subst`. With `prompt_subst` off, those three characters show a backslash in front, and as root bash shows a literal `$` as `#`. Control characters show as U+FFFD.
- *Path:* `$PWD` when absolute, else the real directory. `$HOME` becomes `~` only at a component boundary, and `~` counts as one of the three parts.
- *User:* hidden when `$USER` is empty. Over SSH (`$SSH_CONNECTION` non-empty) the host is the kernel hostname.
- *Git:* the repository is found by walking up from the real directory to a `.git` directory or file. The command is `git --no-optional-locks status --porcelain=v2 --branch`, so a prompt never takes the index lock. Any entry, untracked files included, is dirty. `⇡`/`⇣` show only when nonzero, and a detached head shows 7 hex digits. On timeout or failure, the branch comes from `HEAD`, following a `.git` file to its git directory, plus `?`. Segments wait until 95 ms after start, and git also stops at its 80 ms timeout; a child still running is killed.
- *Runtime:* only the current directory's marker files count; the first in the table's order wins. Programs are `node`, `python` then `python3`, `rustc`, `go` (asked `go version`; it has no `--version`) and `ruby`. Each is found on absolute `PATH` entries, and the segment is hidden when the program isn't there. The version shows as `major.minor`, or `?` when unknown. The cache maps each program path to its mtime (seconds and nanoseconds) and version, and is replaced atomically. An unreadable cache counts as empty. *Shims are never cached* (settled with the user): a version manager's stand-in keeps one path and mtime while the version follows each project's pin. That covers a program whose real target is `mise` or `rustup`, or one in a `shims` directory (asdf, pyenv, rbenv, nodenv). Those are asked every prompt, alongside git.

## 9. Config and theme

### 9.1 `~/.config/shoal/config.toml`

Every key is optional; values below are defaults. Window and daemon watch the file and apply changes within a second. An invalid file keeps the last good config and shows a banner with file, line and reason. Unknown keys are a warning.

```toml
[font]
family = "JetBrains Mono"   # icons, powerline and box drawing work with any family
size = 11.0                 # points
line_height = 1.0

[window]
opacity = 0.88              # pane background alpha; Hyprland blurs what's behind
padding = 8
gap = 6
radius = 10

[theme]
source = "omarchy"          # "omarchy" or a path to a colors.toml-style file
[theme.colors]              # optional overrides, any colors.toml key
# accent = "#ff40a3"

[cursor]
style = "block"             # block | bar | underline
blink = false
color = "accent"            # a palette key or "#rrggbb"

[scrollback]
lines = 10000               # at most 100000 (the attach decoding limit, 5.5)
persist = true

[shell]
program = ""                # empty: $SHELL, then /etc/passwd
integration = "auto"        # auto | manual | off

[prompt]
mode = "native"             # native | shell
style = "powerline"         # powerline | lean
segments = ["user", "path", "git", "runtime"]

[selection]
copy_on_select = false

[tabs]
title = "auto"              # auto | program

[behavior]
close_pane_on_exit = true
confirm_close_running = true

[keybinds]                  # "chord" = "action"; "none" unbinds. Defaults in section 10.
# "ctrl+shift+t" = "tab.new"
```

**Chord grammar:** modifiers `ctrl`, `shift`, `alt`, `super` joined with `+`, then one key: a letter or digit, `f1`–`f12`, `enter`, `tab`, `escape`, `space`, `backspace`, `insert`, `delete`, `home`, `end`, `pageup`, `pagedown`, `up`, `down`, `left`, `right`, or a punctuation character such as `=`, `-` or `\`. Keys match by what they type on the layout.

**Validation and overrides (T05, settled 2026-10-25).** A missing config file uses defaults. A malformed file or any invalid value rejects the entire edit; a reload retains the last good config. Font size and line height must be finite and positive, opacity must be finite and in 0..=1, window padding/gap/radius must be nonnegative, and scrollback lines must be in 0..=100000. Unknown config keys produce warnings; unknown action ids or prompt segment names are errors. `prompt.segments` must contain `path` exactly once.

Keybinding overrides apply per chord: adding a shortcut preserves an action's default shortcuts until those chords are explicitly assigned `"none"` or another action. Chords are case-insensitive; repeated modifiers and multiple entries that normalize to the same chord are errors. `+` as the key is written `+` alone or after the modifier separator (for example `ctrl++`). Super has no default binding but is available in user chords.

**Config watching.** Use native filesystem notifications, debounce 100 ms after the last relevant event, and reload at least once per 750 ms during continuous edits. Block while idle; no polling timer. Watch parent directories so atomic saves, directory replacement and initially absent config directories work. A symlink config also follows changes to its target. Reload callbacks carry the last good config alongside any error; the initial read follows watch installation so startup saves are not missed.

### 9.2 Theme mapping

Lookup order: `[theme.colors]` in config, then `shoal.toml` in the active Omarchy theme folder, then that folder's `colors.toml`, then a built-in dark palette. Omarchy's older short names (`bg`, `fg`, ...) are accepted.

| Shoal uses | `colors.toml` key |
| --- | --- |
| Terminal background (at `window.opacity`) | `background` |
| Text | `foreground` |
| Cursor | `accent` |
| Selection background / text | `selection` / `bright_foreground` |
| ANSI 0 black · 8 bright black | `background` · `muted` |
| ANSI 1–6 | `red`, `green`, `yellow`, `blue`, `magenta`, `cyan` |
| ANSI 9–14 | `bright_red` ... `bright_cyan` |
| ANSI 7 white · 15 bright white | `foreground` · `bright_foreground` |
| Top bar fill | `darker_background` |
| Focus border, active-tab underline, bell dot, prompt character | `accent` |
| Unfocused pane border | `lighter_background` |
| Inactive tab text, tooltips | `dark_foreground` |

**Resolution rules (T06, settled 2026-10-25).** These follow Omarchy's own resolver (`omarchy-theme-color`), so Shoal shows what every other Omarchy app shows.

- *Active folder:* `$XDG_STATE_HOME/omarchy/current/theme/` when it is a directory (Omarchy 4), otherwise `$XDG_CONFIG_HOME/omarchy/current/theme/` (3.x, where it may be a symlink), otherwise none, and the built-in palette applies. With `theme.source` set to a file path, that file replaces both theme-folder files: config, then the file, then the built-in palette. A relative path is relative to `~/.config/shoal/`; `~/` expands to `$HOME`.
- *File format:* `shoal.toml` uses the same flat keys as `colors.toml`. Keys Shoal does not use (`mode`, `hyprland_active_border`, ...) are ignored.
- *Legacy names:* every Omarchy alias is accepted. These are the short names `bg`, `fg`, `dark_bg`, `darker_bg`, `lighter_bg`, `dark_fg`, `light_fg` and `bright_fg`, plus `color0`–`color15`, `purple` and `bright_purple`. Within one file, a canonical name beats its alias. Across sources, the higher source wins whichever spelling it uses. `color0`, `color7` and `color15` are `background`, `foreground` and `bright_foreground`. `color8` and `selection_background` only feed the derivations below.
- *Missing keys:* after layering, a missing key is derived from the theme's present keys exactly as Omarchy does it. `bright_foreground` comes from `foreground`, `lighter_background` from `background`, and `dark_foreground` from `color8`, else `foreground`. `muted` comes from `color8`, else `dark_foreground`. `selection` comes from `selection_background`, else `color8`, else `background`. `darker_background` is `background` mixed 50 % with black. Each `bright_<ansi>` is its base color mixed 20 % with white, where a mix rounds each channel half up. Any key still missing, `accent` included, comes from the built-in dark palette: Omarchy's Tokyo Night `colors.toml`.
- *Bad values:* a key Shoal uses whose value is not a `#rrggbb` string is skipped with a warning naming file, line and key, and that key falls through to the next source. A theme file that is not valid TOML or UTF-8 is skipped whole, with a warning. Theme problems are warnings, never errors.
- *Cursor:* `cursor.color` names a palette key or a `#rrggbb` value; the default, `accent`, matches the table.

**Following theme switches:** watch `~/.local/state/omarchy/current/` (Omarchy 4) or `~/.config/omarchy/current/` (3.x) for the theme folder being swapped and `theme.name` being written. Debounce 100 ms, re-read, recolor every window. The daemon watches too, so programs that query terminal colors get the new ones. Only changes to `theme`, `theme.name`, the active `colors.toml` and `shoal.toml` (or their symlink targets) count. `omarchy-theme-set` builds the next theme in `current/next-theme/` and repoints `current/background` after the swap, and neither causes a reload. Both locations are watched, so a theme folder that appears later is picked up. Watching uses the config watcher's nonrecursive ancestor watches, debounce and 750 ms cap from 9.1.

## 10. Default keybindings

Super is never bound (Hyprland owns it). Every action id can be rebound in `[keybinds]`.

| Action id | Default | Notes |
| --- | --- | --- |
| `tab.new` | Ctrl+Shift+T | Opens in the focused pane's directory |
| `tab.close` | Ctrl+Alt+W | Confirms if a program is running |
| `tab.next` / `tab.prev` | Ctrl+Tab / Ctrl+Shift+Tab, also Ctrl+PageDown / Ctrl+PageUp | |
| `tab.goto_1` ... `tab.goto_9` | Alt+1 ... Alt+9 | Alt+9 is the last tab |
| `tab.move_left` / `tab.move_right` | Ctrl+Shift+PageUp / Ctrl+Shift+PageDown | |
| `tab.rename` | Ctrl+Shift+R | Also double-click the tab |
| `pane.split_right` | Ctrl+Shift+O | Ghostty default |
| `pane.split_down` | Ctrl+Shift+E | Ghostty default |
| `pane.close` | Ctrl+Shift+W | Last pane closes the tab |
| `pane.focus_left/right/up/down` | Ctrl+Alt+Arrow | Ghostty default |
| `pane.resize_left/right/up/down` | Ctrl+Alt+Shift+Arrow | 2 cells per press |
| `pane.zoom` | Ctrl+Shift+Enter | Toggles |
| `session.switcher` | Ctrl+Shift+S | Section 7.6 |
| `session.next` / `session.prev` | Ctrl+Alt+PageDown / Ctrl+Alt+PageUp | |
| `window.new` | Ctrl+Shift+N | Same smart rule as Super+Enter |
| `clipboard.copy` | Ctrl+Shift+C, Ctrl+Insert | Omarchy's Super+C sends Ctrl+Insert |
| `clipboard.paste` | Ctrl+Shift+V, Shift+Insert | Omarchy's Super+V sends Shift+Insert |
| `font.increase` / `font.decrease` / `font.reset` | Ctrl+= / Ctrl+- / Ctrl+0 | Per window |
| `scroll.page_up` / `scroll.page_down` | Shift+PageUp / Shift+PageDown | |
| `scroll.top` / `scroll.bottom` | Shift+Home / Shift+End | |
| `scroll.prev_prompt` / `scroll.next_prompt` | Ctrl+Shift+Up / Ctrl+Shift+Down | Needs shell integration |
| `config.edit` | Ctrl+Shift+, | Opens `config.toml` in `$EDITOR` in a new tab |

**Mouse:** click to focus a pane or tab; drag a gap to resize; drag to select (Shift forces local selection when a program captures the mouse); double-click a word, triple-click a line; middle-click pastes the primary selection; middle-click a tab closes it; Ctrl+click opens a link; click the session chip for the switcher.

## 11. CLI in milestone 1

| Command | What it does |
| --- | --- |
| `shoal` | Opens a window using the smart rule, starting the daemon if needed |
| `shoal attach <session>` | Opens a window on a named session |
| `shoal new [-n NAME] [--cwd DIR]` | Creates a session without a window; prints its id |
| `shoal ls [--json]` | Sessions › tabs › panes with titles, directories and status |
| `shoal kill <target>` | Closes a session, tab or pane |
| `shoal send <pane> <text>` | Types text into a pane; `\n` presses Enter |
| `shoal capture <pane> [--scrollback] [--vt]` | Prints a pane's contents as text or raw terminal bytes |
| `shoal daemon start \| stop \| status \| persist` | Manages `shoald` |
| `shoal prompt ...` | Renders the native prompt (8.3) |
| `shoal shell-init bash \| zsh` | Prints the integration for manual mode |

**Targets** are ids (`s3`, `t12`, `p40`), a session name, or `.` for the pane the command runs in (from `$SHOAL_PANE`). **Exit codes:** 0 success, 1 error, 2 bad usage, 3 daemon unreachable.

## 12. Performance budgets

Measured on the user's machine and recorded in `docs/perf.md`. A regression of more than 20% on an automated measure fails the check.

| Measure | Budget | How |
| --- | --- | --- |
| Window shows content, daemon already running | ≤ 200 ms | Launch timestamp to first presented frame, from the log |
| Daemon start, restoring 20 panes × 10,000 lines | ≤ 500 ms | Test harness |
| Input frame to echoed output at the daemon | ≤ 2 ms p99 | Test harness echoing through `cat` |
| Attach a pane with 10,000 lines | ≤ 50 ms | Test harness |
| Switch to a recently viewed tab | within one frame | Window trace log |
| Idle CPU, daemon + window, 10 panes, 60 s | < 0.5% combined | `pidstat` |
| `cat` of a 100 MB file | ≤ 1.5 × Ghostty's time | Script |
| Daemon memory, 20 panes × 10,000 lines | ≤ 200 MB | `/proc/<pid>/status` |
| Prompt | Section 8.3 | `hyperfine` |

## 13. Acceptance criteria

Milestone 1 is done when every box is ticked. **Auto** items are tests run by `just check`; **manual** items the user runs. The final gate is a week of daily use with no lost work.

**Sessions survive**

- [ ] Auto: run `sleep 1000` in a pane, close the window; `shoal ls` still shows the pane running `sleep`. Reopen: same layout, `sleep` still in the foreground.
- [ ] Auto: two windows on one session show identical output; typing in either appears in both.
- [ ] Manual: with one window open, Super+Enter reopens the most recent off-screen session; with every session on screen, it creates session `2`.
- [ ] Auto: build 3 sessions with splits in different folders and print a marker line in each pane; `shoal daemon stop`, then start. The layout matches, each marker sits above a "restored" divider, and each shell's `pwd` is the saved folder.
- [ ] Manual: the same check across a real reboot.
- [ ] Auto: after `kill -9` of the daemon and a restart, at most the last 30 s of scrollback is missing.
- [ ] Auto: a pane whose folder was deleted restores into `$HOME` with "(folder gone)" in its divider.
- [ ] Auto: a corrupt `workspace.json` is renamed `.broken-*`, the daemon starts empty, and the window shows a notice.

**Terminal correctness**

- [ ] Auto: `shoal-term` golden tests pass (write, resize, alternate screen, snapshot round trip).
- [ ] Auto: with `nvim` open, detach and reattach; the mirror equals the daemon's capture cell for cell, and their modes match.
- [ ] Auto: after resizing splits, `stty size` in each pane equals the grid the window draws.
- [ ] Manual: nvim, htop, btop, lazygit, less and fzf draw correctly and take keyboard and mouse input.
- [ ] Manual: in nvim with the Kitty keyboard protocol, Ctrl+Enter and Ctrl+I are told apart from Enter and Tab.

**Window, keys and layout**

- [ ] Auto: 10,000 random sequences of layout operations keep every invariant.
- [ ] Manual: every default binding in section 10 works; rebinding one in `config.toml` applies without a restart.
- [ ] Manual: dragging a gap resizes; zoom toggles; focus moves to the pane that is visually next.
- [ ] Manual: tab titles follow: `cd ~/code` shows `code`, running `nvim` shows `nvim`, quitting shows `code` again.
- [ ] Manual: Omarchy's Super+C and Super+V copy and paste.
- [ ] Manual: text typed through an input method arrives in the pane (skip if none is used).
- [ ] Manual: Ctrl+click opens a URL printed by `echo`.

**Theme and look**

- [ ] Manual: `omarchy-theme-set <another theme>` recolors open windows within 1 s; the prompt follows at the next Enter.
- [ ] Manual: the wallpaper shows through the panes, blurred by Hyprland; `window.opacity = 1.0` makes them opaque.
- [ ] Manual: with every Nerd Font uninstalled, prompt icons and powerline separators still render.
- [ ] Auto: a fixture `colors.toml` maps to exactly the palette in 9.2.

**Prompt and shell integration**

- [ ] Auto: golden prompt output for bash and zsh in fixture repositories (clean, dirty, ahead/behind, detached, none).
- [ ] Auto: prompt speed budgets from 8.3 are met.
- [ ] Auto: with auto integration, the user's own `.bashrc` and `.zshrc` still run.
- [ ] Manual: with `prompt.mode = "shell"`, the user's own prompt is untouched and jump-to-prompt still works.

**Robustness**

- [ ] Auto: a client that stops reading gets a resync snapshot; the shell and other clients are unaffected.
- [ ] Auto: an attach requested inside an unfinished OSC longer than 4096 bytes completes at the terminator, and the mirror then equals the daemon's capture; with no terminator it fails after 2 s with `mid_sequence` while the shell keeps running.
- [ ] Auto: a window whose `core` differs from the daemon's gets error 1003 from `pane.attach`.
- [ ] Auto: truncated, corrupted, oversized and trailing-byte snapshots fail to decode with an error, never a panic.
- [ ] Auto: with nvim on the alternate screen, the saved scrollback holds the main screen and history beneath it.
- [ ] Auto: the frame decoder survives 10 minutes of fuzzing without a panic.
- [ ] Manual: killing the daemon with a window open shows "Reconnecting", restarts it, and restores the panes.

**Performance**

- [ ] Every budget in section 12 is met and recorded in `docs/perf.md`.
