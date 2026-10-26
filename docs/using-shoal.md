# Using Shoal

How to run Shoal from this checkout while it is being built. Run everything
from the repository root. Shoal is not packaged yet (that is T50), so these
commands use the release build in `target/release/`.

## Start and stop

```sh
just run      # build if needed, start the daemon if needed, open a window
just stop     # close the windows this checkout opened and stop the daemon
```

`just run` again opens another window. A window shows one session; the first is
`main`, and a second window gets a new session.

The daemon, `shoald`, owns every shell. Closing a window closes nothing: the
shells keep running, and the next window shows them again. `just stop` stops the
daemon, which saves the workspace first; the next start restores it.

## What works today

Milestone 1 is being built task by task (`docs/spec/tasks.md`). Right now:

- The window shows the session's tabs in the top bar and its panes in their
  boxes, with the terminal content drawn live (T40, T41).
- Panes follow the window's size, including fullscreen (T43).
- With the mouse (T43): click a tab to switch to it, middle-click to close it,
  double-click to rename it (Enter keeps the name, Escape cancels, an empty name
  goes back to the automatic title). `+` opens a tab; the two buttons at the top
  right split the focused pane right or down. Click a pane to focus it; drag the
  gap between panes to resize them.
- Type directly into panes, including Unicode and committed input-method text.
  Terminal keys follow the program's modes, including application arrows and
  Kitty key events. The tab rename field also accepts input-method composition.
- Keyboard tabs: Ctrl+Shift+T creates, Ctrl+Alt+W closes; Ctrl+Tab / Ctrl+Shift+Tab
  (or Ctrl+PageDown / Ctrl+PageUp) cycle. Alt+1…8 selects by position; Alt+9 selects
  the last. Ctrl+Shift+PageUp / PageDown moves a tab. Ctrl+Shift+R renames it.
- Keyboard panes: Ctrl+Shift+O / E splits right / down; Ctrl+Shift+W closes.
  Ctrl+Alt+Arrow moves focus; add Shift to resize by two cells.
  Ctrl+Shift+Enter toggles zoom. Close confirmations come with T47.
- Mouse events go to programs that request them. Hold Shift for local selection.
  Otherwise drag selects, double-click selects a word and triple-click a line.
  Selecting sets primary selection; middle-click pastes it. Ctrl+Shift+C or
  Ctrl+Insert copies; Ctrl+Shift+V or Shift+Insert pastes. Pastes use the program's
  bracketed-paste mode. Selections follow their text through output and resize;
  soft-wrapped lines copy without added newlines. `selection.copy_on_select`
  also writes the clipboard and reloads live. Focus leaving the pane, typing,
  history erasure or eviction of an endpoint clears the selection.
- Scroll locally with the wheel (three rows per notch) or touchpad (by pixel).
  Hold Shift to scroll locally when a program captures the mouse. Shift+PageUp /
  PageDown pages; Shift+Home / End goes to the oldest output / live bottom.
  New output preserves a scrolled-up view and shows a **new output** pill.
  Click it, type or paste to return to the bottom. Ctrl+Shift+Up / Down jumps
  between shell prompts. Recently viewed tabs retain their scroll position.
- Hold Ctrl over a row to underline its links; Ctrl+click opens one with
  `xdg-open`. OSC 8 targets and explicit `http://` / `https://` URLs work,
  including soft wraps. Balanced brackets in URLs are kept; surrounding
  punctuation is trimmed.
- Sessions: click the session chip or Ctrl+Shift+S. Type a fuzzy filter, use
  Up/Down and Enter to switch, or choose New session. F2 renames the selected
  session; Escape cancels. Ctrl+Backspace asks before closing it. Closing the
  session a window shows closes that window. Other sessions keep running.
  Counts show each session's tabs; On screen means another window shows it.
- Ctrl+Alt+PageDown / PageUp cycles sessions in creation order, wrapping.
  Ctrl+Shift+N opens a window on the most recently active off-screen session,
  or creates one when all sessions are on screen.
- Rebind actions in `[keybinds]` in `config.toml`; bindings reload without a restart.
  `"none"` removes a binding. Ctrl+Shift+, opens the file in `$EDITOR` in a new tab
  (`vi` if unset). Font zoom is T48.
- A lost daemon connection blanks the panes and shows **Reconnecting to shoald…**.
  When the daemon returns, the window rebuilds panes from fresh snapshots.
  Reconnecting never starts the daemon: systemd restarts a crashed managed
  daemon; after `shoal daemon stop`, use `shoal daemon start` to resume.
- Config errors and daemon notices appear above the panes with **Dismiss**.
  An invalid config names the file, line and reason and keeps the last good
  settings. A different daemon build offers **Restart daemon**, explaining
  that running programs end and layout/scrollback return. Protocol/core
  mismatches leave panes blank; a version-only mismatch still shows panes.
  Restart happens only when clicked. If stopping takes over five seconds,
  the window reports the failure and leaves the daemon alone.

Omarchy's universal copy/paste needs its `terminal` tag to match `dev.shoal.Shoal`;
packaging and desktop integration are T50. Custom desktop bindings still take
precedence over those defaults.
Shoal itself never binds Super.

You can also drive a pane from another terminal with the `shoal` command:

```sh
scripts/shoal.sh ls                       # sessions, tabs and panes, with ids
scripts/shoal.sh send main 'btop\n'       # type into main's focused pane; \n is Enter
scripts/shoal.sh send main 'q'            # quit btop
scripts/shoal.sh capture main             # print what the pane shows
scripts/shoal.sh new -n work              # a new session called work
scripts/shoal.sh kill work                # close it
scripts/shoal.sh daemon status
```

`scripts/shoal.sh <command>` runs any `shoal` command from this checkout.
To send Escape, use a shell string: `scripts/shoal.sh send main $'\e:q!\n'`
quits nvim.

## Logs

```sh
scripts/shoal.sh log                      # follow the newest window's log
tail -f ~/.local/state/shoal/logs/shoald.log   # the daemon's log
```

Window logs are in `target/logs/`, at debug level. Each frame that redraws a
whole pane logs `full redraw` with its size and time in milliseconds.

## Manual checks

Tasks that need your eyes come with a guided script:

```sh
just manual t41
```

It opens what it needs, asks yes/no questions, measures what it can, and saves
the answers to `target/manual/t41.txt`. When it finishes, tell the agent; it
reads that file. `just manual t43` checks layout interactions. It closes this
checkout's previous windows and stops its daemon first, then prints the rebuilt
GUI's timestamp. Failed steps are recorded and later checks continue.

`just manual t42` checks keys, live rebinding, text, clipboard, mouse and focus
reports in its own disposable HOME/XDG environment. Its bounded input probe saves
raw fixture input there for inspection. It stops only the processes it started;
results go to `target/manual/t42.txt`. Skip input-method checks if none is used.

`just manual t44` checks session filtering, switching, creation, rename,
confirmation, next/previous and smart new windows in its own disposable HOME/XDG
sandbox. Results go to `target/manual/t44.txt`; it also cleans up the extra
windows created during the check.

`just manual t45` checks smooth scrolling, anchored selections, the new-output
pill, clipboard/primary paste, prompt jumps and links in an isolated HOME/XDG
environment. Its substitute `xdg-open` records the exact URI instead of opening
a browser. Results go to `target/manual/t45.txt`.

`just manual t46` checks recovery after a crash, intentional stops, config
errors, notice dismissal, build mismatches and the restart timeout. It uses
disposable HOME/XDG folders and a fake systemctl, and restarts the crashed
test daemon itself to stand in for systemd. Results go to
`target/manual/t46.txt`.

## A sandbox for experiments

`just dev-daemon` and `just dev-gui` (in two terminals) run debug builds with
their own home folder in `target/dev-env/`, so nothing touches your real
sessions or settings. They don't see your Omarchy theme.
