# Agent environment runbook

Isolated, disposable environment rules for every agent session. `AGENTS.md` points here; task records in `docs/spec/completed.md` explain *why* these rules exist.

## Sandbox

- Never touch real state: no `~/.local/state/shoal`, no real socket, no real config. Use `shoal_testkit::TempEnv` in tests and disposable `HOME` plus every XDG path per child in manual scripts (`scripts/manual/t45.sh` is the model).
- Fresh sandboxes per task. Do not reuse old saved workspace. Preserve passing guided-result files before a script overwrites them.
- Build before isolating `HOME`: `CARGO_BUILD_JOBS=8 mise exec -- cargo build --locked --release -p shoal-gui -p shoal-cli -p shoal-daemon --bins --example call`. Release assets need `target/share -> ../share`, as the guided scripts arrange.
- Required checks run outside the execution sandbox (Unix sockets and PTYs). Use a runtime tmpfs:
  ```sh
  mkdir -p /run/user/$(id -u)/shoal-check
  ulimit -c 0
  TMPDIR=/run/user/$(id -u)/shoal-check CARGO_BUILD_JOBS=8 mise exec -- just check
  ```
  Do not use `/tmp` (synthetic `/tmp/.git` confuses prompt tests) or btrfs `/var/tmp` (exposes filesystem-sensitive persistence failures). Do not weaken assertions to hide environment failures.
- GUI builds use a lot of memory. If the linker or rustc is killed, retry with `CARGO_BUILD_JOBS=8` (or lower), never change code for it.

## Desktop and input

- An agent shell may lack `WAYLAND_DISPLAY` and `HYPRLAND_INSTANCE_SIGNATURE`. Discover the live socket explicitly (unique Wayland socket under `$XDG_RUNTIME_DIR`, `hyprctl -j instances` or `$XDG_RUNTIME_DIR/hypr/`). A normal interactive shell may already export them.
- Verify the target window's executable/`HOME` before injecting input; refresh geometry for each pointer action.
- `wtype` exercises real shortcuts. Standard physical key codes may be needed for compositor bindings; `wtype`'s remapped codes can reach the app instead.
- Read the Omarchy skill/capture guide before screenshots. `grim` is available.
- Never change desktop preferences or test against real Shoal state. Stop only processes the script started. Never reuse recorded PIDs to signal processes.
- The shell's machine-check report may say "not a Wayland session" when session variables are absent, even though the live compositor/socket works when passed explicitly.

## Daemon and systemd

- Tests never reach the real `systemctl`: every `TempEnv` command finds a logging fake first on `PATH`, which fails unless a test scripts it.
- `shoal daemon stop` stays stopped while windows are open. Reconnecting never auto-starts a daemon: the client retries the socket; systemd's `Restart=on-failure` restarts a crashed managed daemon.
- SIGKILL is only for the owned crash-test process. Never `SIGKILL` as a restart/recovery mechanism (that loses up to 30 s of scrollback).

## Logging

- Never log pane output, typed input, or environment values.
- Window logs go to `target/logs/` at debug level. Daemon logs go to `~/.local/state/shoal/logs/shoald.log` in production, stderr under `just dev-daemon`.
