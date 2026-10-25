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
| `split(tab, pane, Right \| Down, new)` | Replaces the pane with a 50/50 split of old and new; focuses the new pane. |
| `close(tab, pane)` | Replaces the parent split with the sibling; returns the pane to focus, or `None` if it was the last pane (the caller then closes the tab). Clears `zoomed` if needed. |
| `rects(layout, area, gap)` | Turns the tree into pixel rectangles. Rectangles never overlap and, with the gaps, exactly fill the area. |
| `neighbor(rects, pane, Left \| Right \| Up \| Down)` | The closest pane in that direction that overlaps on the other axis; ties go to the larger overlap, then the top-left one. |
| `resize(layout, pane, direction, step)` | Moves the edge of the nearest ancestor split on that axis, so the focused pane grows toward `direction`. |
| `set_ratio(layout, split_id, ratio)` | Used by mouse drags on a divider. |
| `toggle_zoom(tab, pane)` | Zoomed pane fills the tab; the others keep running at their old size. |
| `validate(workspace)` | Every pane in `panes` appears in exactly one tab's layout and every layout pane is in `panes`; ids are unique and all below `next_id`; `active_tab` points inside its session; `focused` and `zoomed` point inside their tab; every session has a tab. Runs after every change in debug builds. |

## 4. Daemon behaviour

### 4.1 Starting and staying single

1. Create `$XDG_RUNTIME_DIR/shoal/` with mode 0700.
2. Take a non-blocking `flock` on `shoald.lock`. If another daemon holds it, log "already running" and exit 0.
3. Remove a stale socket file, bind `shoald.sock`, set mode 0600. Accept only peers whose `SO_PEERCRED` uid matches ours.
4. Load config and theme (the daemon needs theme colors to answer programs that ask the terminal for its colors).
5. Restore the saved workspace (section 6), then start accepting connections.

**How it gets started.** `shoald.service` is a systemd user unit (`Type=simple`, `Restart=on-failure`, `TimeoutStopSec=10`). Any client that can't connect runs `systemctl --user start shoald.service` and polls the socket for up to 2 s. If that fails or systemd is absent, it spawns `shoald` detached (`setsid`, stdio to `/dev/null`). Shells run inside the service's cgroup, so closing windows or the Hyprland session doesn't touch them, and stopping the service ends them cleanly.

### 4.2 Pane runtime

**Spawning a shell.** Open a PTY at the pane's size (portable-pty). The command is the pane's `argv`, else `shell.program` from config, else `$SHELL`, else the passwd entry, else `/bin/bash`, started as an interactive shell. Working directory: the one requested, else the source pane's current one (for splits and new tabs), else `$HOME`.

**Environment.** Start from the daemon's environment. Overlay the latest connected window's values for a fixed allowlist, because a daemon started by systemd may hold stale session variables: `WAYLAND_DISPLAY`, `DISPLAY`, `HYPRLAND_INSTANCE_SIGNATURE`, `SSH_AUTH_SOCK`, `DBUS_SESSION_BUS_ADDRESS`, `XDG_CURRENT_DESKTOP`, `XDG_SESSION_ID`. Then set `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=shoal`, `TERM_PROGRAM_VERSION`, `SHOAL_PANE=<pane id>`, `SHOAL_SOCKET=<socket path>`, plus the shell-integration variables from section 8.

**The pane thread.** Each pane gets one OS thread, named `pane-<id>`, that owns the PTY and the `shoal-term` terminal. It waits with `poll` on the PTY and an eventfd for commands, so it uses no CPU while quiet.

- **Output:** read up to 64 KiB, feed the terminal, append to the pane's stream with the next sequence number, wake subscribers. Batch into one frame per pane at most every 4 ms or 64 KiB.
- **Replies:** when a program asks the terminal something (cursor position, colors, device attributes), the reply goes straight back into the PTY from this thread.
- **Commands:** `Write(bytes)`, `Resize(size)`, `Snapshot(reply)`, `Subscribe`, `Unsubscribe`, `Capture`, `Kill`.
- **Kill:** `SIGHUP` to the process group, then `SIGKILL` after 2 s.
- **Exit:** on EOF, collect the exit status and set `Exited`. With `close_pane_on_exit = true` (default) the workspace actor closes the pane, which may close its tab and session. Guard: if a pane exits within 1 s of starting with a non-zero code, keep it open showing the error.

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

## 5. Protocol

Protocol version 1. Every change updates this section in the same commit.

### 5.1 Frames

```
frame = len:u32 | kind:u8 | payload        len = bytes of kind + payload; integers little-endian

0x01 JSON      both ways        UTF-8 JSON object (5.2)
0x10 OUTPUT    daemon → client  pane:u64 seq:u64 bytes...
0x11 SNAPSHOT  daemon → client  pane:u64 seq:u64 cols:u16 rows:u16 bytes...
0x12 RESIZED   daemon → client  pane:u64 seq:u64 cols:u16 rows:u16
0x20 INPUT     client → daemon  pane:u64 bytes...
```

Maximum frame size 64 MiB; a bigger frame or unknown kind closes the connection with a logged protocol error. Binary frames carry the numeric pane id (`3`), JSON the prefixed form (`"p3"`).

### 5.2 JSON messages

JSON-RPC 2.0 shapes: a request has `id`, `method`, `params`; a response has `id` plus `result` or `error {code, message, data?}`; a notification has `method` and `params` but no `id`. The daemon handles one connection's requests in order.

### 5.3 Handshake

The first frame must be `hello`; anything else gets an error and the connection closes. The window sends the allowlisted environment variables from 4.2 in `env`.

```
→ {"jsonrpc":"2.0","id":1,"method":"hello","params":{"protocol":1,"client":"gui","version":"0.1.0","env":{"WAYLAND_DISPLAY":"wayland-1"}}}
← {"jsonrpc":"2.0","id":1,"result":{"protocol":1,"daemon_version":"0.1.0","pid":4312}}
```

### 5.4 Methods

| Method | Params | Result |
| --- | --- | --- |
| `hello` | `protocol`, `client` (`gui` \| `cli`), `version`, `env` | `protocol`, `daemon_version`, `pid` |
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

**Notifications from the daemon**

- `workspace.changed {rev, workspace}`: the whole workspace after any change, at most one per 16 ms per connection.
- `daemon.notice {level, message, key?}`: banner-worthy problems (config error, theme folder not found, a restore that skipped a broken file).
- `pane.event {pane, event}`: reserved for agent panes (M3). Not sent in M1.

**Error codes:** `-32600` invalid request, `-32601` unknown method, `1001` not found, `1002` invalid state, `1003` version mismatch (data: `daemon_protocol`), `1004` bad params, `1005` internal.

### 5.5 Attaching to a pane

1. The client sends `pane.attach`.
2. On the pane's thread, in one step: dump a snapshot with the formatter (screen, scrollback, styles, cursor, modes), note the current sequence number `N`, and register the subscriber for items after `N`.
3. The daemon sends `SNAPSHOT(pane, N, cols, rows, bytes)`, then the JSON result `{seq: N, cols, rows}`, then `OUTPUT` and `RESIZED` frames numbered `N+1`, `N+2`, ...
4. The client builds a fresh mirror terminal at `cols × rows`, feeds it the snapshot, then every item in order. A `RESIZED` frame resizes the mirror at exactly that point.
5. A later `SNAPSHOT` for an attached pane means "replace your mirror". A sequence gap means the mirror is stale: the client sends `pane.attach` again.

**Backpressure:** each subscriber has an 8 MiB queue. On overflow the daemon drops the queue and sends a fresh `SNAPSHOT` at the current sequence number. A slow window never slows the shell or other windows.

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
- **Scrollback:** for a pane whose output changed, after 2 s of quiet, at most once every 30 s; and every pane on shutdown. The pane thread runs the formatter; compressing and writing happen on a blocking worker, never on the workspace actor.
- **What is saved:** the main screen plus scrollback up to `scrollback.lines`, with colors and styles. If a full-screen program is up, only the main screen underneath is saved.
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

- **Mirrors.** Each attached pane has a mirror: a `shoal-term` terminal plus its render state. New bytes are fed in and a frame is requested; nothing redraws while nothing changes.
- **Per frame.** Only dirty rows are rebuilt: merged background rectangles, text shaped per style run, underline and strike lines, selection, cursor. Clean rows reuse cached shaped lines. The renderer produces an internal draw list first, so it is testable without a GPU and portable to the fallback toolkit.
- **Colors.** Default-background cells are not painted, so the pane's translucent fill shows. Explicit backgrounds are painted at `window.opacity`. Dim text is drawn at 60% alpha.
- **Fonts.** `font.family` (default JetBrains Mono), then the bundled Symbols Nerd Font Mono (embedded, MIT), then the system emoji font, then system fallback. Wide characters take two cells, as libghostty reports.
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
- If the daemon's version differs from the window's: a banner with a "Restart daemon" button, which persists, restarts and restores.
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
lines = 10000
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

**Following theme switches:** watch `~/.local/state/omarchy/current/` (Omarchy 4) or `~/.config/omarchy/current/` (3.x) for the theme folder being swapped and `theme.name` being written. Debounce 100 ms, re-read, recolor every window. The daemon watches too, so programs that query terminal colors get the new ones.

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
- [ ] Auto: the frame decoder survives 10 minutes of fuzzing without a panic.
- [ ] Manual: killing the daemon with a window open shows "Reconnecting", restarts it, and restores the panes.

**Performance**

- [ ] Every budget in section 12 is met and recorded in `docs/perf.md`.
