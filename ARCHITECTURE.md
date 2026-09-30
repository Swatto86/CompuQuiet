# CompuQuiet architecture

Tauri 2 desktop app: a Rust engine behind a vanilla TypeScript window. The
page has no filesystem, shell or process permission; everything with an
effect happens in Rust behind a validated command.

```
ui/            vanilla TS + Vite: dashboard, targets editor, settings, about
src-tauri/     the shell: commands, engine, tray, autostart, window lifecycle
crates/
  cq-core/     domain, no OS calls: profile, policy, planner, journal, settings
  cq-platform/ the Platform trait and its adapters: windows, linux, macos, fake
e2e/           WebdriverIO suite driving the real binary (fake platform)
scripts/       verify (full gate), fastcheck (inner loop), driver setup
```

Dependencies point inward: `src-tauri` → `cq-platform` → `cq-core`.

## The flow

1. **Snapshot.** `Platform::snapshot` lists programs (via `sysinfo`; never
   threads), the state of the profile's services, and the active power plan.
2. **Plan.** `cq_core::build_plan` turns profile + snapshot + capabilities into
   ordered `Step`s and a list of skipped targets with reasons. Order: power
   plan, services, processes, memory purge. Critical processes and services
   (`policy.rs`), keep-alive entries (a process is also spared when only its
   own name matches, as a truncated Linux name does) and the app itself are
   never planned.
3. **Execute and journal.** Each step's `DoneStep` is written to
   `journal.json` (atomic write) before the platform carries it out
   (`DoneStep::intended`, `Engine::run_journaled`), corrected afterwards if
   the platform reports something different, and taken back out if the step
   failed. A crash mid-step therefore leaves it on record, and every undo is
   safe for a step that never happened. A step that times out
   (`PlatformError::TimedOut`, code `timed_out`) may still take effect, so
   its entry stays; a program that has already gone is logged as done, not
   failed. Progress lines stream to the window; a failed step is logged and
   the run continues.
4. **Restore.** The journal is replayed newest-first (`restore_steps`),
   relaunching a closed program once per distinct command line, and not at
   all while that command line is already running. A helper a program started
   for itself is journaled with its program's command line
   (`ProcessInfo::program_root`), so the program comes back, not the helper.
   The journal is saved after every settled step (`Journal::without`), so an
   interrupted restore never repeats one. Entries whose undo failed are kept
   for retry; one whose process, program or service no longer exists is done
   with, and a power plan deleted meanwhile is replaced by Balanced
   (`Platform::restore_power` returns the plan now active). An emptied
   journal is deleted, with retries; one left on disk because the file stayed
   locked counts as finished at the next launch. What a restore could not
   undo is listed in `EngineState::unrestored`; only then does Home offer to
   give up on it (`Engine::give_up_restoring`), after a confirmation that
   names each entry, which moves `journal.json` to `journal.json.bad` and ends
   Quiet Mode.
5. **Recovery.** At launch a leftover journal puts the engine straight into
   Quiet Mode marked "recovered", so a crash never strands changes. Steps a
   restart or a new sign-in has already undone are skipped (`Elapsed`,
   `RestoreStep::overtaken`), comparing the journal's `Marker` (uptime and
   sign-in identity from `Platform::marker`, no wall clock) with the current
   one; the power plan is a saved setting and is always put back. A journal
   from an earlier sign-in is finished at launch; an unreadable one blocks a
   new run rather than being overwritten, until Home's action moves it aside
   (`Engine::set_aside_journal`). `go_quiet` also re-reads the file
   (`adopt_journal_on_disk`): entries it finds that this copy has not seen are
   adopted and the run refused, and a file that has become unreadable blocks
   it the same way.
6. **System tools** (`powercfg`, `taskkill`, `schtasks`, `systemctl`,
   `launchctl`, `pkexec`) run through `cq_platform::run_tool`: a 180 s
   deadline, and the C locale on Linux and macOS so their messages can be
   matched. Windows decisions use error codes, never a tool's translated
   text; the forced close is `TerminateProcess`.

Processes are identified by PID plus start time so a reused PID is refused.

## The scan

`cq_core::recommend` turns a snapshot, the live stats, the platform's
`Activity` (which PIDs own a visible window, which is in front) and the
profile into ranked `Recommendation`s: catalogue matches (`catalogue.rs`, per
OS, each with a reason and a `Risk`), running services from the catalogue, a
non-performance power plan, a file cache above 1 GB, and, only when the
platform can report windows, unknown processes over 200 MB or 3% CPU that
own none, except the workloads in `catalogue::WORKLOADS` (interpreters, local
AI servers, the WSL VM) and the family of the program in front. Names on the
keep-alive list, services included, are skipped. Anything the profile already
covers is returned marked `already_targeted` so the page can grey it out.
`recommend::apply` folds accepted finds into a profile without touching
existing entries.

The engine exposes `scan()` (fresh snapshot) and `apply_recommendations()`
(saves the profile). With `Settings::auto_scan` on, `go_quiet` reuses its own
snapshot to compute the report and plans against the profile plus the
low-risk, not-yet-targeted program and service finds for that run only (the
power plan and the purge stay the saved switches); the journal records what
actually happened, so Restore is unchanged. Removing a target on the Targets
tab adds its name to the keep-alive list ("Never touch"), because the scan
would otherwise find a known target again on every run. Window ownership comes from
`EnumWindows` on Windows (a Store app's own process owns only a child window
of its frame, so those are read too); Linux and macOS report
`Activity::known = false` and the scanner then names only recognised software.

## Platform adapters

`cq-platform` is the only crate allowed `unsafe`, and only in its Windows
module: `NtSuspendProcess`/`NtResumeProcess`, the token elevation check, the
standby-list purge, the file-cache figure (`GetPerformanceInfo`), a service's
running dependents, window enumeration, starting a program with the desktop
shell's token (`CreateProcessWithTokenW`, so an elevated CompuQuiet does not
hand its rights on) and the `runas` relaunch. Everything else uses safe crates
(`windows-service`, `sysinfo`, `nix`) or structured subprocess calls with
validated arguments (`powercfg`, `taskkill`, `systemctl`, `powerprofilesctl`,
`launchctl`, `schtasks`).

`Capabilities` reports what this process can do at its privilege level; the
planner skips what it cannot with the reason shown ("needs administrator
rights"), and the window offers the elevated relaunch on Windows.

The `fake` feature provides an in-memory machine for tests and the e2e
suite. It is never a default feature; `scripts/verify.sh` asserts that. It
can be told to fail (`Fake::fail(Call, target, Failure)`, `Fake::heal`): a
refusal, a missing-rights error, or a timeout whose effect still lands. The
e2e build exposes that as the `fake_fail` and `fake_heal` commands, which
exist only with the `fake-platform` feature.

## Shell behaviour

- Window starts hidden with a matching background; the page reveals it after
  its first render (`frontend_ready`), with a 3 s safety net in Rust.
- Close hides to the tray when `close_to_tray` is set; otherwise it emits
  `confirm-quit` and the page decides. The `quit` command is the one exit and
  can restore first.
- Tray: left click shows or hides the window (a blur caused by that click
  still counts as "the window was in front"). Right click opens the menu on
  the event loop, not inside the icon's window procedure, because Windows
  ignores menu clicks opened from that procedure. Linux keeps the indicator
  menu and puts it back after an icon change, which otherwise drops it.
- A window whose WebView2 failed to start is only logged by Tauri; its handle
  stays registered but every query on it fails. `tray::reveal` treats that as
  "no window" and `reopen` restarts the process once with `--reopen` (shown, never
  hidden), waiting for any Quiet Mode run to finish first. A reopen that
  fails too notifies instead of restarting again.
- `logfile` sends every crate's `log` warnings and errors to
  `compuquiet.log` in the data directory; it is the only record of why a
  window failed to load.
- One copy per data directory (`cq_core::instance`, `single.rs`): the first
  copy holds an exclusive lock on `instance.lock` for the life of the process.
  A later launch leaves a request in `wake/` (one command word from a fixed
  list, never arguments) and waits up to 3 s for the running copy, which polls
  twice a second, to take it; the window is then shown (`Command::Show`). The
  files carry the user's access list, so an unelevated launch reaches an
  elevated copy, which window messages and named objects cannot. A lock freed
  during the wait is taken over: a restart or an elevated relaunch starts
  before the old process is gone and both call `single::release` first. A
  copy that does not answer exits with code 1. The elevated-to-unelevated pair
  is a manual check; the suite runs from one unelevated shell.
- Updates (`update.rs`, `update/guard.rs`): only a copy that can replace itself
  checks (Windows with `uninstall.exe` beside it, Linux with `APPIMAGE`, macOS
  inside a `.app`; never a debug or fake build). A resident copy considers a
  check every 30 minutes; one that got an answer is repeated after 6 hours, a
  failed one after 1 hour. The manifest check has a 30 s limit and the download 10
  minutes, so a stalled connection cannot hold the single attempt slot. A
  found update is downloaded at once but installed only when no run is going,
  Quiet Mode is off and the window is hidden (installing ends the process), and
  the restart goes through `single::restart`, as `reopen` does. `update::Status`
  (commands `update_status`, `check_for_updates`; event `update-status`) is
  what the window shows.
- Autostart: `schtasks` logon task on Windows (elevated when created by an
  elevated process), `tauri-plugin-autostart` elsewhere, always guarded
  against registering a temporary or build-directory executable.

## State

`COMPUQUIET_DATA_DIR` overrides the platform config directory. Files are
`settings.json` (versioned) and `journal.json` (versioned). Writes are
temp-file + rename, and the rename and reads are retried briefly on the
Windows errors a scanner holding the file causes. A newer or corrupt
`settings.json` is an error, not a reset: the engine runs on the defaults but
refuses saves and Go Quiet (`EngineState::settings_unreadable`) until the
banner's action moves the file to `settings.json.bad`.

## Verification

- Rust unit tests: policy, planner, journal, settings, store, fake adapter,
  engine round trip and failure paths (`engine/failure_tests.rs`); real suspend/resume/close on a child process and real
  service/power queries on the host OS.
- Frontend tests (`node --test`): formatting and profile editing.
- WebDriver suite (`e2e/`): boot, the full quiet-then-restore workflow
  asserting the journal on disk, persistence across a restart, and a clean
  quit that restores first, and giving up on a restore step that cannot
  succeed. Runs on Windows and Linux in `scripts/verify.sh`.
