# Process Manager — agent notes

The README is for people using the app. This file is for whoever changes it: how the code is
laid out, the rules the UI follows, how to check a change, and how releases are versioned.

## Layout

| Path | What it holds |
| --- | --- |
| `src/main.rs` | Entry: `--normalize-config`, working directory, single instance, then the UI |
| `src/config.rs` | `processes.json`: load, migrate, save, groups, schedules |
| `src/process_manager.rs` | Starting, stopping and watching processes and containers; logs, activity, resource sampling, managed restart, the scheduler |
| `src/terminal/` | Processes run in a terminal: the VT emulator on `vte` (`term.rs`), the session (the pseudo-terminal's handles, the input writer), and the rules that turn its screen into log lines |
| `src/rest_api.rs` | The optional loopback REST API and the agent skill text |
| `src/platform.rs`, `src/single_instance.rs` | Data directory, Unix process groups, the Windows instance guard |
| `src/ui/mod.rs` | The app's state, navigation (`Nav`, `View`, `ProcessTab`), actions, shortcuts, the frame loop |
| `src/ui/theme.rs` | The Graphite look: colours, fonts, metrics, layout tiers (`Tier`) |
| `src/ui/widgets.rs`, `src/ui/icons.rs` | The kit: buttons, chips, toggles, switches, lights, fields, cards, menus, sparklines; painted icons |
| `src/ui/format.rs` | Wording: status sentences, schedules, sizes, durations, log line classification (pure, tested) |
| `src/ui/header.rs` | Header, footer, the process switcher for narrow windows, sidebar rows |
| `src/ui/sidebar.rs` | Sidebar: groups, rows, drag to reorder, context menus |
| `src/ui/console.rs` | A process's console: head, stats strip, tabs, the log viewer, Activity |
| `src/ui/screen.rs` | The Screen tab: a terminal's cells, and the keys, paste and mouse sent to it |
| `src/ui/editor.rs` | Settings in place (also the New process page), validation, the save bar, path suggestions |
| `src/ui/overview.rs` | The overview table and the stack log |
| `src/ui/palette.rs` | The command palette |
| `src/ui/stack_settings.rs`, `src/ui/dialogs.rs` | Stack settings page; confirmations |
| `scripts/demo/`, `scripts/run-demo.ps1` | A demo stack and the screenshot runner |

## The look

The spec is the "Process Manager Redesign" mockup (Graphite artboard). Keep to it:

- Colours come from `theme.rs` only. One accent (periwinkle `#a3b8ff`) for focus, selection and
  the primary button. Green, amber, red and grey are for status and nothing else.
- Inter Regular and Bold for the interface (`theme::body`, `theme::bold`), Cascadia Mono for
  logs, commands and numbers (`theme::mono`). Both are embedded under the SIL Open Font License.
- Controls are painted by the kit in `widgets.rs` at fixed sizes (30 pt controls, 26 pt small,
  30 pt rows, 6 pt corners, 8 pt cards). Don't use egui's default buttons or checkboxes in the
  main screens; add to the kit instead.
- Text galleys from `widgets::galley` carry no colour of their own; the colour given when painting
  is the one used. Layout jobs with explicit colours (caps labels, log lines) keep theirs.
- Place a widget inside a rect you already allocated with `widgets::put_free`, not `Ui::put`
  (which allocates again and eats the spacing after it).
- Words: say what is true in plain sentences ("Exited with code 1 · restarting", "Skips Start
  all"), never codes like `(M)`. Wording lives in `format.rs` with tests.

### Small windows

`Tier::for_width` decides the layout from the window width: `Wide` ≥ 1100, `Full` ≥ 900,
`Medium` ≥ 760, `Compact` ≥ 520, `Tiny` below. The rules, from wide to narrow:

- The header's search becomes an icon below `Wide`; the stack buttons become icons at `Medium`
  and a menu below it; the API pill drops its address below `Full` and leaves at `Tiny`.
- The sidebar shows from `Medium` up. Below that the header shows a process switcher and the
  console leads with the status (the switcher already names the process).
- The command line and badges fold behind Details when the window is narrow or short (main area
  under 560 pt); the stats strip needs `Medium` and 600 pt of height.
- The log toolbar measures itself: level chips fold into one dropdown, then the toggles into a
  menu, as room runs out. The time column leaves at `Tiny`.
- Overview columns drop by table width (command, behaviour, uptime, memory, CPU), and the stack
  log needs `Medium` and 460 pt.

The log always keeps the room that is left.

## Checking a change

```powershell
cargo fmt --all
cargo test
./scripts/run-demo.ps1 -Screenshot .tmp/shots/console.png -Window 1440x900 -After 12
./scripts/run-demo.ps1 -Screenshot .tmp/shots/small.png -Window 520x640 -Open settings:ingest
```

CI runs `cargo fmt --all -- --check` and `cargo test --locked` on Windows and both Mac
architectures, so format before committing and keep `Cargo.lock` in step.

`run-demo.ps1` copies the build to `.tmp/demo` beside `scripts/demo/processes.json`, so your own
`processes.json` is never touched. The app reads these only for screenshots and checks:

- `PM_SCREENSHOT=<file.png>`: save one frame, then quit.
- `PM_SCREENSHOT_AFTER=<seconds>`: let the stack run this long first (default 3).
- `PM_WINDOW=<width>x<height>`: the window's size at start (and it is not restored from the last run).
- `PM_OPEN=<view>[:<process>]`: start on `overview`, `stack`, `new`, `settings`, `activity`,
  `screen`, `palette` or `delete` (its confirmation), with that process selected.
- `PM_TYPE=<keys>`: 1.5 s before the screenshot, focus the selected process's screen and type
  these keys through the keyboard's path. `{enter}`, `{tab}`, `{esc}`, the arrows (`{up}` …)
  and `{ctrl-<letter>}` are keys; the rest is text.

Unset `NO_COLOR` before screenshots of terminal processes: Rich and most colour libraries obey it
(it is passed on to terminal processes, as a user who sets it would expect).

Look at screenshots at a wide and a narrow size before calling a UI change done. Images in
`images/` are the README's; regenerate them with the commands in the README's Development section
when the look changes.

## Runtime notes

- A process's in-memory log (1,000 lines) is kept across restarts; Clear the log view empties it.
  Line numbers count every line of the session (`ProcessRuntimeInfo::log_total`), so they stay
  stable as old lines fall off.
- Every start and stop records who asked (`start_process_by`, `stop_process_by`): `You`, `Start
  all`, `Auto-start`, `Managed restart #n`, `Schedule`, `Local API`, a group. Keep a cause on new
  call sites; it is what the Activity tab shows.
- Restarts wait for the stop, so the UI runs them on a thread, never on the frame.
- Saving settings updates the running process's config in place. A changed command, folder, kind,
  output (terminal mode) or disk logging applies at the next start; the save bar offers Save and
  restart for that.

### Terminals

A process's `terminal_mode` (`Off`, `Auto`, `Screen`; absent is `Off`, how every process ran
before 2.1) picks pipes or a pseudo-terminal (`portable-pty`: ConPTY on Windows, `openpty` on
macOS). Keep the default `Off` so old configs run unchanged.

- The log comes from the primary screen. A row joins the log as soon as it has text and changes
  in place while the program redraws it (progress bars, live tables); it is written to disk and
  checked for errors once it has been still for a second, or when it scrolls off, the screen is
  cleared, or the process ends. A row erased for good (0.4 s) lets its line go, keeping the text.
  Rows on screen when a full-screen program takes over are sealed: what replaces them later is a
  new line. The alternate screen never reaches the log. Tests in `src/terminal/mod.rs` pin these.
- Lock order is terminal, then processes. Never lock a terminal while holding the manager's lock;
  `ProcessManager::terminal` hands out the `Arc` for that reason.
- Closing a Windows pseudo-console waits for its last output, which the reader thread must be
  free to read: `TerminalSession::detach` hands back the handles to drop outside every lock.
- New terminals start at the size the UI last measured for the log area (at least 100 × 24, so
  output is not laid out for a narrow window and stays broken). The Screen tab resizes only
  full-screen programs and `Screen`-mode processes; `Auto` returns to its starting size when the
  program leaves the alternate screen.
- Terminal lines carry colour spans (`LogLine::spans`); their text is plain, so filters, error
  detection, disk logs and the REST API see plain text.

## Versions and releases

The app follows [Semantic Versioning](https://semver.org). `Cargo.toml` is the only place the
version is written; the app, the packaging scripts and the release workflow read it from there.

- **Major** (`3.0.0`): a change people or their tools must adapt to: `processes.json` that an
  older build cannot read, a REST route or response that changes or goes away, a shortcut or
  default that does something different.
- **Minor** (`2.1.0`): new features that leave existing configs, routes and habits working.
- **Patch** (`2.0.1`): fixes only.

To release:

1. Add the version's entry at the top of `CHANGELOG.md` (what changed, in words a user follows;
   call out anything that behaves differently).
2. Set `version` in `Cargo.toml`, run `cargo build` so `Cargo.lock` follows, and commit both with
   the changelog as `Release <version>`.
3. Tag that commit `v<version>` and push the tag. The workflow refuses a tag that does not match
   `Cargo.toml`, builds every platform, and drafts the GitHub release for review.

Never move or reuse a tag, and never ship a different build under a version already released:
bump the patch number instead. Between releases `Cargo.toml` keeps the last released version.

2.0.0 is the first release under these rules (the redesign); `v1.0.0`–`v1.0.2` were tagged before
them and 1.0.3–1.0.6 were never tagged.
