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
- Keys so far: Ctrl+Shift+R renames the current tab, Ctrl+Shift+Enter zooms the
  focused pane in and out.
- Shell typing and scrolling come with T42. The tab rename box already takes text.

Until then, drive a pane from any other terminal with the `shoal` command:

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

## A sandbox for experiments

`just dev-daemon` and `just dev-gui` (in two terminals) run debug builds with
their own home folder in `target/dev-env/`, so nothing touches your real
sessions or settings. They don't see your Omarchy theme.
