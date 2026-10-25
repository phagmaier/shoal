# Shoal: technical decisions and milestone plan

As of Oct 24, 2026.

## Summary

Shoal is three Rust binaries built around one rule: the daemon owns everything, and the window only mirrors it.

- **`shoald`** (the daemon) runs every shell in a pseudo-terminal, keeps each pane's screen and scrollback in a libghostty-vt terminal, owns the session › tab › split layout, and saves it all to disk.
- **`shoal-gui`** (the window) is a GPUI app. It shows a pane by replaying the daemon's output bytes into its own copy of that pane's terminal.
- **`shoal`** (the CLI) launches windows, scripts the daemon and draws the native prompt. It is small so the prompt stays fast.

**Choices made with the user:** direct key chords, native prompt on by default for bash and zsh, reboot restore inside milestone 1, smart new windows, and `shoal` as the working name.

**The vision doc's open questions, answered**

| Question | Answer |
| --- | --- |
| Build on, or from scratch? | Build on libghostty-vt, Ghostty's embeddable core. Don't fork WezTerm (decision 1). |
| Do pi and omp speak ACP? | omp has ACP built in. pi works through community adapters (pi-acp, pi-acpinator). Both also run fine as TUIs in a shell pane. |
| How does Omarchy theme sync work? | Omarchy 4 renders the active theme into `~/.local/state/omarchy/current/theme/`, starting from a 24-color `colors.toml`. Shoal reads that file directly and watches it, so no hook is needed (decision 6). |
| Native prompt or my own? | Native by default; `prompt.mode = "shell"` keeps your own. |
| Markdown pane: view or edit? | View-only first. Add light editing only if you miss it after using it. |
| Remote: SSH, Tailscale, or both? | SSH only. Tailscale already works through SSH. If the link drops, the window shows "reconnecting", the remote daemon keeps running, and reattaching replays the screen. |
| Name | `shoal`, as a working name. |

**Pushback that shaped the spec**

- **Blur is Hyprland's job.** Hyprland already blurs whatever sits behind a translucent window. Shoal exposes opacity only.
- **No in-app wallpaper.** The window is translucent, so the real wallpaper shows through and already follows Omarchy theme switches.
- **Running programs can't survive a reboot.** Reboot restore means same layout, same directories, old scrollback, and a fresh shell in each pane. Agents that support resuming are the exception, in a later milestone.
- **Only show buttons that work.** Search, command and notification buttons appear in the milestone that makes them work.

## Key technical decisions

| # | Decision | Recommendation | Confidence |
| --- | --- | --- | --- |
| 1 | Terminal core | libghostty-vt, behind our own wrapper crate | High |
| 2 | Language | Rust (edition 2024) | High |
| 3 | GUI and rendering | GPUI pinned to one Zed commit; fallback winit + wgpu | Medium; the T00 spike kept GPUI |
| 4 | Daemon | One per-user daemon owns panes, layout and persistence; windows mirror it | High |
| 5 | Client-daemon protocol | Unix socket, length-prefixed frames, JSON for control, binary for pane bytes | High |
| 6 | Config and theme | TOML; read Omarchy's `colors.toml` directly and watch it | High |
| 7 | ACP later | Daemon is the ACP client; milestone 1 reserves the slots, writes no ACP code | High |

### 1. Terminal core: libghostty-vt

**Recommendation:** use libghostty-vt through its Rust bindings, and let exactly one crate of ours (`shoal-term`) touch it.

**What it is:** the part of Ghostty that turns a program's output bytes into a grid of colored cells with a cursor and scrollback, and turns keypresses back into bytes. It does no drawing and no windowing. It ships a C API with a render-state API for drawing and a snapshot module to encode and restore terminal state ([vt.h](https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt.h)).

**Why it fits Shoal:**

- It can capture a pane two ways. Its binary snapshot module restores a pane's complete state; live attach uses it, with daemon and window on the identical libghostty build. Its formatter dumps the screen as plain terminal bytes; reboot restore uses that, as zmx does ([zmx](https://erock-git-zmx.pgs.sh)). The T00 spike showed the formatter alone loses state a live mirror needs (`docs/spike-00.md`).
- It already handles Kitty keyboard and graphics protocols, reflow on resize, prompt marks, a key encoder and a mouse encoder.
- Maintained Rust bindings exist: the `libghostty-vt` crate, 0.2.2 as of Sep 28, 2026 ([lib.rs](https://lib.rs/crates/libghostty-vt)).

**What it costs:**

- **The API is not stable.** Ghostty made an ABI-breaking change on Oct 21, 2026 ([changelog](https://github.com/dive/ghostty-tip-changelog/releases/tag/daily-2026-10-21)). Mitigation: pin one Ghostty commit, wrap it in `shoal-term`, treat upgrades as their own task with tests.
- **The snapshot format has no compatibility guarantee.** Mitigation: `hello` carries a core identity built from the full pinned revisions, and a window attaches only to a daemon with the identical one (M1 spec §5.3). A pin upgrade re-runs the snapshot fidelity suite.
- **The build needs Zig.** The bindings compile libghostty-vt from source; the Zig version depends on the pinned commit ([libghostty-rs](https://github.com/uzaaft/libghostty-rs)). Pin it with mise.
- **Not thread-safe.** Each terminal object lives on one thread; the daemon gives every pane its own thread anyway.

**Alternatives considered:**

| Option | Why not |
| --- | --- |
| alacritty_terminal | Pure Rust and solid, but no built-in way to dump its state as replayable terminal bytes, and no Kitty graphics. Kept as the fallback: `shoal-term` makes a swap possible. |
| Fork WezTerm | Last stable release Feb 2024; maintenance only resumed June 2026 ([issue #6341](https://github.com/wezterm/wezterm/issues/6341)). Forking a whole cross-platform terminal is the opposite of small pieces. |
| Write our own | Years of edge cases. Ruled out. |

### 2. Language: Rust

- The pieces we need already speak Rust: the libghostty bindings, GPUI, and the official ACP SDK.
- The compiler is the best guardrail for agent-written code.
- Cost: slower compiles; GPUI is heavy. Acceptable.
- Why not Zig: it still breaks between versions, has no mature GUI toolkit, and agents write it less reliably.

### 3. GUI and rendering: GPUI, proven by a spike

**Recommendation:** GPUI, Zed's GPU-accelerated UI framework, pinned to a single Zed commit as a git dependency. This is the riskiest call, so milestone 1 opens with a one-day spike (task T00) that must pass first.

- Zed's own terminal is drawn with it, so a fast terminal grid in GPUI is a solved problem.
- The agent pane later is a small chat app (Markdown, code blocks, diffs, buttons, a composer). That needs a real UI toolkit.
- It is Wayland-native with a wgpu renderer on Linux, supports transparent windows, and handles input methods through `text-input-v3` ([gpui-ce platform notes](https://docs.rs/crate/gpui-ce/0.3.0/source/docs/platforms.md)).

**What it costs:** no stable release; crates.io `gpui` 0.2.2 is far behind Zed main ([awesome-gpui](https://github.com/edo-zhou/awesome-gpui)). Mitigation: pin one commit and keep a read-only copy of its source in `third_party/`. Thin docs, heavy compile times.

**Fallback if the spike fails:** winit + wgpu with glyphon for text and a small hand-written UI layer. The terminal renderer is written against an internal draw list, so it moves over unchanged.

**Spike result (T00, `docs/spike-00.md`):** GPUI is kept, pinned to Zed v1.25.3 (`8dd255c3eafb5231056fe4e6f601cedd011f1c93`) with libghostty-rs `9ede35b8ea94a55ed3608408d67e102883129f1e` (Ghostty `521cf2c5d30bd48028ca61546c64e7913764073d`, Zig 0.17.0). A full 200 × 60 redraw met the 4 ms target only with cached per-cell glyphs and dirty-row reuse. Per-style-run shaping missed it, so M1 draws without cross-cell ligatures (M1 spec §7.3).

**Rejected:** GTK4 (fighting its theming for the Tern look) and iced (unproven for a fast grid).

### 4. Daemon: one owner, many mirrors

**Recommendation:** one `shoald` per user, run as a systemd user service. It owns the layout, every pane process, every pane's terminal state, and persistence.

```
 shoal-gui (window) ──socket──┐        ┌──────────── shoald ────────────┐
                              ├──────▶ │ Workspace actor (layout)       │
 shoal (CLI) ─────────socket──┘        │        │ spawns, resizes       │
                                       │ Pane threads (PTY + terminal) ─┼──PTY──▶ shells
                                       │        │ snapshots             │
                                       │ Persistence ───────────────────┼──saves──▶ disk
                                       └────────────────────────────────┘
```

Closing a window removes only the left side; everything inside the daemon keeps running.

- **One workspace actor.** A single task owns the session › tab › split tree. Every change goes through it in order, bumps a revision number, and is broadcast.
- **One thread per pane.** It owns its PTY and libghostty terminal, reads output, updates the terminal, and fans bytes out to attached windows. A quiet shell costs no CPU.
- **Windows mirror.** When a window shows a pane, the daemon sends a snapshot, then every new output byte with a sequence number. The window feeds both into its own terminal copy.

**Why mirroring:** bytes are what every terminal speaks, so images, links and modes come along for free; it works over SSH unchanged; scrolling and selection are local. Cost: parsing twice and two copies of visible panes in memory.

**Three rules that keep mirrors honest:**

1. Only the daemon answers a program's queries to the terminal; mirrors discard their replies.
2. The daemon decides each pane's size; the most recently focused window wins, and resizes travel in the same numbered stream as output.
3. A gap in sequence numbers means the mirror is stale; it reattaches for a fresh snapshot.

**Persistence:** layout to `workspace.json`, each pane's scrollback to a compressed file under `~/.local/state/shoal/`, written after output goes quiet and on shutdown. Restore rebuilds the layout, replays old scrollback under a dim "restored" divider, and starts a fresh shell in the saved directory.

### 5. Client-daemon protocol

**Recommendation:** a Unix socket at `$XDG_RUNTIME_DIR/shoal/shoald.sock` with length-prefixed frames: JSON-RPC 2.0 style for control, binary frames for pane bytes.

- JSON keeps it debuggable with `socat` and matches ACP and MCP, which arrive later.
- Binary for pane bytes avoids base64 on the busiest path.
- Full-state updates: any layout change resends the whole workspace (a few KB) with its revision. No delta logic.
- Remote later is the same protocol: `ssh host shoal proxy` pipes frames over SSH.
- A `hello` handshake carries a protocol number and a core identity (the exact libghostty build). On either mismatch the window offers a user-triggered daemon restart that persists first (M1 spec §5.3).

### 6. Config and Omarchy theme

**Recommendation:** one TOML file at `~/.config/shoal/config.toml`, hot-reloaded; theme colors read straight from Omarchy.

- TOML because Omarchy and Alacritty use it. A broken edit keeps the last good config and shows a banner naming the bad line.
- How Omarchy theming works: `omarchy-theme-set` builds the theme in a staging folder, renders templates from `colors.toml`, moves it to `~/.local/state/omarchy/current/theme`, writes `theme.name`, then fires the `theme-set` hook ([Omarchy theming docs](https://github.com/omacom/omarchy/blob/quattro/docs/theming.md)).
- Shoal reads `colors.toml` from that folder (falling back to `~/.config/omarchy/current/theme/` for Omarchy 3.x) and watches it. A `shoal.toml` in the theme folder overrides generated colors.
- The color mapping copies Omarchy's own Alacritty template ([alacritty.toml.tpl](https://github.com/omacom/omarchy/blob/quattro/default/themed/alacritty.toml.tpl)). The focus color is the theme's `accent`.
- No Nerd Font setup: Shoal bundles the symbols-only Nerd Font as a fallback and draws powerline and box-drawing characters itself.

### 7. Leaving room for ACP agent panes

**Recommendation:** the daemon will be the ACP client: it starts each agent as a subprocess and holds the connection, so an agent keeps working with no window open. Use the official `agent-client-protocol` Rust crate (2.2.0) against stable protocol version 1 ([crates.io](https://crates.io/crates/agent-client-protocol), [rust-sdk](https://github.com/agentclientprotocol/rust-sdk)).

What milestone 1 does now, without writing ACP code:

- `PaneKind` is a tagged enum from day one. M1 has only `Shell`.
- Every pane has a `status` field; agents will add `working`, `idle`, `needs_input`.
- Every pane gets its own folder in the state directory for later transcripts and session ids.
- The protocol reserves `pane.event`, and the window draws panes through a per-kind view trait.
- Orchestration later becomes an MCP server inside the daemon, handed to agents through ACP.

## Milestone plan

Each milestone is something you'd use daily before the next starts.

| Milestone | What | Gate |
| --- | --- | --- |
| M1 · Daily driver | One window with tabs and splits, Omarchy theme, native prompt, sessions that survive closing the window and rebooting, minimal CLI | One week as your only terminal |
| M2 · Workspace polish | Command palette, full session switcher, settings screen and theme picker, notifications, scrollback search, layout presets, drag-to-rearrange, full scripting CLI | Every action reachable from the palette |
| M3 · Agent panes via ACP | Agent pane kind: chat view, tool calls and diffs, permission buttons, composer; daemon holds connections and resumes after reboot where supported. Targets: Claude Code, Codex, opencode, omp, pi | A Claude Code session survives closing the window and a reboot |
| M4 · Remote | Headless `shoald` on a Linux host; window attaches via `ssh host shoal proxy`; switcher groups by host; auto-reconnect | A day of remote work over Tailscale feels local |
| M5 · Git where you look | Ahead/behind and dirty state in tabs; palette fetch, pull, push, branch create/switch, browse branches and commits; branch chip on agent panes | Branches from the palette |
| M6 · Markdown pane | Rendered, view-only, live-reloading | A plan file open beside an agent all day |
| M7 · Orchestration (opt-in) | MCP server in the daemon: list, spawn, send, read, wait, behind the vision's guardrails | One agent hands work to another, visibly and stoppably |
| Later | Diff pane, worktree helpers, agent overview | |

## Risks and the spike

| Risk | What could go wrong | Mitigation |
| --- | --- | --- |
| GPUI on Hyprland | Transparency, input methods or frame pacing misbehave; API churn | Spike first; pin one commit; vendor source; winit + wgpu fallback |
| libghostty-vt churn | A pin upgrade breaks our code | Only `shoal-term` touches it; upgrades are their own task with golden tests |
| Snapshot fidelity | The formatter's dump misses state, so a reattached mirror behaves differently | T00 found the formatter loses title, dynamic colors, saved cursor, the screen under an alternate screen, prompt marks and parser state, which no mode list can carry. Live attach uses the exact-build binary snapshot instead (31/31 checked cases). A pin upgrade re-runs the snapshot fidelity suite. |
| Mirror drift after resize | Daemon and window reflow differently | Resizes sequenced in the stream; resync replaces the mirror |
| Zig toolchain | Bindings need a specific Zig version | Pin with mise; clear build error on the wrong one |
| Omarchy changes paths | Theme sync silently stops | Probe both locations, log which is used, banner if neither exists |

**What the spike must show (go/no-go):**

- [x] A transparent GPUI window on Hyprland, blurred by Hyprland, with crisp text at your scale factor. (Confirmed by eye on the laptop at scale 2.)
- [x] A 200 × 60 grid fed by `bash` through libghostty-vt, redrawing under 4 ms per frame. (Laptop: 599 of 600 frames under 4 ms, max 4.11 ms; desktop: all 600.)
- [x] `cat` of a 50 MB log finishes, and idle CPU after it is near zero.
- [ ] A formatter snapshot restored into a second terminal matches the original screen, scrollback and modes. **Failed;** live attach uses the exact-build binary snapshot instead (31/31).
- [ ] Typing through an input method (e.g. fcitx5), if you use one, reaches the terminal. **Skipped:** no input method is used.
- [x] Pinned commits for GPUI and Ghostty written down, with build steps that work from a clean checkout.

**Result (T00, 2026-10-25):** GPUI kept. Numbers, pins, the build recipe and the decisions are in `docs/spike-00.md`.

## Sources

- [Ghostty vt.h](https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt.h)
- [Ghostling: minimal terminal on libghostty](https://github.com/ghostty-org/ghostling)
- [libghostty-vt Rust crate](https://lib.rs/crates/libghostty-vt)
- [libghostty-rs build requirements](https://github.com/uzaaft/libghostty-rs)
- [Ghostty tip changelog, Oct 21 2026](https://github.com/dive/ghostty-tip-changelog/releases/tag/daily-2026-10-21)
- [zmx](https://erock-git-zmx.pgs.sh)
- [WezTerm maintenance status](https://github.com/wezterm/wezterm/issues/6341)
- [Zed GPUI Cargo.toml](https://github.com/zed-industries/zed/blob/main/crates/gpui/Cargo.toml)
- [awesome-gpui](https://github.com/edo-zhou/awesome-gpui)
- [gpui-ce platform notes](https://docs.rs/crate/gpui-ce/0.3.0/source/docs/platforms.md)
- [ACP Rust SDK](https://github.com/agentclientprotocol/rust-sdk)
- [agent-client-protocol crate](https://crates.io/crates/agent-client-protocol)
- [pi-acp](https://github.com/svkozak/pi-acp)
- [omp and ACP](https://innfactory.ai/en/ai-harness/omp/)
- [Omarchy theming docs](https://github.com/omacom/omarchy/blob/quattro/docs/theming.md)
- [Omarchy Alacritty template](https://github.com/omacom/omarchy/blob/quattro/default/themed/alacritty.toml.tpl)
- [Omarchy 4.0 release](https://github.com/basecamp/omarchy/releases/tag/v4.0.0)
