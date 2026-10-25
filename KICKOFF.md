You are starting a new project called Shoal in this folder: a terminal emulator and multiplexer for Linux, built almost entirely by coding agents. The plan is already written. Your job in this session is to set up the repository and run the first task, T00 (a spike). Task T01 (the scaffold) runs in a separate session afterwards.

Before doing anything, read these files fully:
- AGENTS.md (rules for every agent working here)
- docs/spec/decisions.md (key technical decisions, milestone plan, the spike's go/no-go checklist)
- docs/spec/m1-spec.md (milestone 1 spec)
- docs/spec/tasks.md (every milestone 1 task)

Note: AGENTS.md describes the repository as it will be after task T01. The crates, `justfile`, `third_party/` and the test kit do not exist yet. This session adds `mise.toml`; the rest is T01's job.

Work through the parts below in order. Where it says STOP, end your turn and wait for my reply.

## Part 1: Check this machine

Run `scripts/setup-machine.sh --check`. It installs nothing and reports what's missing. Then check what it doesn't cover:
- This is a Wayland session under Hyprland. `$WAYLAND_DISPLAY` should be set. Your shell may lack `$HYPRLAND_INSTANCE_SIGNATURE` even under Hyprland, so a running `Hyprland` process or a `$XDG_RUNTIME_DIR/hypr/` folder also counts.
- Say which Omarchy theme palette exists: `~/.local/state/omarchy/current/theme/colors.toml` (Omarchy 4) or `~/.config/omarchy/current/theme/colors.toml` (3.x).

If anything required is missing, tell me to run `scripts/setup-machine.sh` (or give me the exact `sudo pacman -S --needed ...` command if the script doesn't cover it), then STOP. Never run sudo yourself. Optional tools (hyperfine, socat, an input method such as fcitx5) don't block.

## Part 2: Set up the repository

- `git init` with branch `main` if this isn't a repo yet. Add a Rust `.gitignore` (target/, fetched third_party contents, *.tmp) that also ignores `KICKOFF.md`; this prompt is not part of the repository.
- Add a LICENSE file: MIT, copyright 2026 Parker Hagmaier.
- Add a short README.md: what Shoal is in two sentences, how to set up a machine (`scripts/setup-machine.sh`), and pointers to AGENTS.md and docs/spec/.
- Commit everything as "Add spec, agent guide, setup script and license".

## Part 3: Task T00, the spike

Do T00 exactly as docs/spec/tasks.md describes, on a branch `spike/t00`.

- Research before writing code. Find the current `libghostty-vt` Rust crate (libghostty-rs) and the Zig version it needs, and pick a recent Zed commit for GPUI and how to depend on it from git. Read the real source of both; never guess an API.
- Pin that Zig version with mise in `mise.toml` and run `mise install`. Installing pinned mise tools, Rust toolchains through rustup, and cargo tools is allowed without asking (see AGENTS.md).
- Build the spike and the snapshot harness. Measure everything you can yourself: frame time at 200 × 60, `cat` of a 50 MB file, idle CPU afterwards (`pidstat`), snapshot fidelity mode by mode.
- Write `docs/spike-00.md` with the numbers, the pinned commits, the Zig version, and build steps that work from a clean checkout.
- If any check you can measure fails, record why in `docs/spike-00.md`, follow T00's failure rule in tasks.md, and STOP for my decision. Don't work around a failure on your own.

Then STOP and ask me to check what only a person can see. Give me the exact command to launch the spike window and these questions:
1. Is the window translucent, with the wallpaper blurred behind it?
2. Is the text crisp at my monitor's scale?
3. Do typing and scrolling feel smooth?
4. If I use an input method, does text typed through it arrive? (I may answer "no input method"; record that as skipped.)

## Part 4: After I reply

- Record my answers in `docs/spike-00.md`.
- If any answer is a failure: write the fallback as a proposed spec change (winit + wgpu for GPUI problems), show it to me, and STOP until I approve.
- If everything passed: update decision 3 in `docs/spec/decisions.md` and the "Looking things up" section of AGENTS.md with the pinned commits and Zig version.
- Add anything the spike needed to install to `scripts/setup-machine.sh`.
- Commit `docs/spike-00.md`, `mise.toml`, the setup script and the spec updates to `main`. Keep the spike code on its branch only.

Finish with a short summary: what was done, anything in the spec that turned out wrong, and that the next step is T01 in a fresh session.

## Rules for the whole session

- Follow AGENTS.md.
- Do not start T01 or any later task.
- If the spec is ambiguous or wrong, ask me; don't invent behaviour.
- Never run sudo. Ask before installing anything outside this folder beyond what AGENTS.md allows.
