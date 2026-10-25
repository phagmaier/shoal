# Shoal

Shoal is a terminal emulator and multiplexer for Linux on Wayland, made for Hyprland and Omarchy.
A per-user daemon owns every shell, tab and split, so your sessions survive closing the window and rebooting.

## Set up a machine

On Arch or Omarchy, from the repository root:

```sh
scripts/setup-machine.sh --check   # report what's missing
scripts/setup-machine.sh           # install it (asks for sudo for pacman)
```

## Where to read next

- [AGENTS.md](AGENTS.md): architecture, rules and commands for everyone working here.
- [docs/spec/](docs/spec/): the technical decisions, the milestone 1 spec and its task list.

## License

MIT. See [LICENSE](LICENSE).
