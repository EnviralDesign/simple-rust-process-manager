# Changelog

Versions follow [Semantic Versioning](https://semver.org); see `AGENTS.md` for the rules.

## Unreleased

Planned as 2.1.0: new features, and existing `processes.json` files run exactly as before.

### New

- Terminals. A process's new **Output** setting runs it in a terminal (ConPTY on Windows, a
  pseudo-terminal on macOS) instead of pipes, so tools that check for one behave as they do in a
  shell: Rich, coloured loggers and test runners keep their colours, progress bars from pip,
  cargo, npm or Rich update one line in place, and Python flushes its output as it goes.
  - **Terminal**: the log keeps colour and redraws; when the program goes full screen (a Textual
    app, htop, a TUI), the console switches to its **Screen** and back when it leaves.
  - **Terminal, opens on its screen**: for interactive tools such as REPLs and prompts.
- The Screen tab: the program's screen as it draws it, with typing, paste, the mouse and Ctrl+C
  going to it while it has focus (the window's shortcuts wait meanwhile). Copy screen copies its
  text. A program that has ended leaves its last screen.
- The command palette shows a terminal process's screen and sends it Ctrl+C.
- The console's badges say when a process runs in a terminal.
- `PM_OPEN=screen` and `PM_TYPE` (keys typed into a screen) for screenshots and checks.
- The demo stack's nightly backfill runs in a terminal.

### Changed

- Nothing for existing configs: a process without `terminal_mode` (every config before this)
  runs through pipes as it always did, and `Off` is never written back.

## 2.0.0

The redesign. Process Manager gets a new look and a new way around, and the runtime keeps more of
what happens.

### New

- A new look (Graphite): Inter for the interface, Cascadia Mono for logs, one accent colour, and
  status colours only where they mean status.
- A console per process: its status in plain words, the command and folder, badges for what it is
  set to do, and live CPU and memory sparklines, uptime, PID and restarts.
- A better log: line numbers and times, stderr tagged instead of prefixed, chips for errors,
  warnings and Process Manager's own events with counts, a filter that highlights matches, Follow
  that lets go when you scroll up with a "new lines" button back, and Wrap.
- Settings in place: a process's settings are a tab beside its log, saved from a bar at the bottom;
  leaving with unsaved changes asks first. Save and restart applies a new command at once. The
  active-hours grid takes drags and has presets.
- Activity: each process's starts, stops, exits and failures this session, with what caused them
  (you, auto-start, Start all, managed restart, the schedule, the local API, a group).
- Overview: every process in one table under its group, with a filter, state chips, group
  controls, checkboxes for bulk start, stop, restart and making groups, and the stack log, every
  process's output merged by time.
- A command palette (Ctrl/Command+K) for every action and process.
- Stack settings as a page: stack name, log folder, error flash, local API, the config file, About.
- Small windows: the layout steps down as the window narrows, down to 420 points wide. The sidebar
  becomes a switcher in the header and secondary detail folds away; the log keeps the room.
- `scripts/run-demo.ps1` runs a demo stack and takes screenshots of any screen at any size.

### Changed

- Start all, Stop all and Restart all moved to Ctrl/Command+Shift+S, X and R. Ctrl/Command+R now
  restarts only the selected process, and Ctrl/Command+Enter starts or stops it. The old
  Ctrl+X (Stop all) is gone so a stray Cut can no longer stop the stack.
- A process's log is no longer cleared when it starts again: earlier output stays (up to 1,000
  lines), so a crash's traceback survives the managed restart that follows. The REST API's
  `GET /processes/{id}/logs` returns these lines too. Clear the log view from the console's menu.
- Saving a running process's settings no longer stops it: the new settings apply at once, a new
  command or folder at its next start (or now, with Save and restart).
- Restarts and reloads of processes.json run in the background instead of freezing the window
  while processes stop.
- Changing the kind (process or Docker container) of a running entry stops it first.
- Window shortcuts wait while a text field has focus or a dialog is open.
- The interface font is Inter instead of Selawik.

## 1.0.6 and earlier

Before 2.0.0 versions were not tracked here; see the commit history and the `v1.0.0`–`v1.0.2` tags.
