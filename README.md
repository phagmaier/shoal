# Shoal

Shoal is a terminal emulator and multiplexer for Linux on Wayland, made for Hyprland and Omarchy.
A per-user daemon owns every shell, tab and split, so your sessions survive closing the window and rebooting.

## Set up a machine

On Arch or Omarchy, from the repository root:

```sh
scripts/setup-machine.sh --check   # report what's missing
scripts/setup-machine.sh           # install it (asks for sudo for pacman)
```

Setup trusts this checkout's `mise.toml`, installs the pinned tools and Rust
components, installs nightly for cargo-fuzz, and fetches read-only reference
sources. Agents run only `--check`; the install command above is for the user.
The Rust pin is 1.98.1 in both mise and rustup so a global mise override cannot
silently select a different workspace compiler.

## Try it

`just run` opens a window from this checkout; `just stop` stops everything.
[docs/using-shoal.md](docs/using-shoal.md) explains what works so far and how
to drive it.

## Development

Milestone 1 is being built task by task; see [the task list](docs/spec/tasks.md) for current status. [AGENTS.md](AGENTS.md) has the canonical check command and links to the agent runbook and known flakes. Use that command for the full check in an agent environment.

```sh
mise exec -- just check        # fmt, clippy -D warnings, tests, dependency gates
mise exec -- just test
mise exec -- just dev-daemon
mise exec -- just dev-gui
```

The development launchers share `target/dev-env/` within this checkout, with
isolated `HOME` and XDG folders. They preserve the real Wayland socket and tool
stores. No production Shoal state or socket is used.

`deny.toml` enforces the M1 crate edges and the exclusive GPUI/Ghostty wrappers.
`scripts/check-boundaries.sh` covers protocol/tokio and testkit dependency-kind
rules. The deny gate checks dependency bans; a full license/advisory/source
policy is not part of this gate.

`mise exec -- just fuzz-proto` runs `cargo +nightly fuzz run frame_decoder`.
Normal workspace checks use stable Rust, not nightly.

## Reference sources

```sh
third_party/fetch.sh           # shallow, sparse fetch at the T00 revisions
third_party/fetch.sh --check   # offline integrity/read-only check
```

`third_party/gpui/` includes GPUI's source and examples;
`third_party/gpui_platform/` holds its platform selector;
`third_party/ghostty/include/ghostty/vt.h` is the pinned terminal API.
They are gitignored links into immutable, revision-named `.sources-*` checkouts.
Repeated fetches reuse intact sources. After a pin change, fetch publishes new
links without overwriting an old cache. Modified/incomplete caches and unmanaged
paths are rejected: move the reported path aside, then rerun fetch.
Cargo uses the pinned git dependencies, not these reference copies.

## Where to read next

- [AGENTS.md](AGENTS.md): architecture, rules and commands for everyone working here.
- [docs/spec/](docs/spec/): the technical decisions, the milestone 1 spec and its task list.

## License

MIT. See [LICENSE](LICENSE).
