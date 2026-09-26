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

1. **Snapshot.** `Platform::snapshot` lists processes (via `sysinfo`), the
   state of the profile's services, and the active power plan.
2. **Plan.** `cq_core::build_plan` turns profile + snapshot + capabilities into
   ordered `Step`s and a list of skipped targets with reasons. Order: power
   plan, services, processes, memory purge. Critical processes, keep-alive
   entries and the app itself are never planned.
3. **Execute and journal.** The engine runs each step through the platform,
   appends a `DoneStep` to `journal.json` (atomic write) before the next step,
   and streams a progress line to the window. A failed step is logged and the
   run continues.
4. **Restore.** The journal is replayed newest-first (`restore_steps`),
   relaunching a closed program once per distinct executable. Entries whose
   undo failed are kept for retry; an empty journal is deleted.
5. **Recovery.** At launch a leftover journal puts the engine straight into
   Quiet Mode marked "recovered", so a crash or reboot never strands changes.

Processes are identified by PID plus start time so a reused PID is refused.

## The scan

`cq_core::recommend` turns a snapshot, the live stats, the platform's
`Activity` (which PIDs own a visible window, which is in front) and the
profile into ranked `Recommendation`s: catalogue matches (`catalogue.rs`, per
OS, each with a reason and a `Risk`), running services from the catalogue, a
non-performance power plan, a file cache above 1 GB, and, only when the
platform can report windows, unknown processes over 200 MB or 3% CPU that
own none. Anything the profile already covers is returned marked
`already_targeted` so the page can grey it out. `recommend::apply` folds
accepted finds into a profile without touching existing entries.

The engine exposes `scan()` (fresh snapshot) and `apply_recommendations()`
(saves the profile). With `Settings::auto_scan` on, `go_quiet` reuses its own
snapshot to compute the report and plans against the profile plus the
low-risk, not-yet-targeted finds for that run only; the journal records what
actually happened, so Restore is unchanged. Window ownership comes from
`EnumWindows` on Windows; Linux and macOS report `Activity::known = false`
and the scanner then names only recognised software.

## Platform adapters

`cq-platform` is the only crate allowed `unsafe`, and only in its Windows
module: `NtSuspendProcess`/`NtResumeProcess`, the token elevation check, the
standby-list purge and the `runas` relaunch. Everything else uses safe crates
(`windows-service`, `sysinfo`, `nix`) or structured subprocess calls with
validated arguments (`powercfg`, `taskkill`, `systemctl`, `powerprofilesctl`,
`launchctl`, `schtasks`).

`Capabilities` reports what this process can do at its privilege level; the
planner skips what it cannot with the reason shown ("needs administrator
rights"), and the window offers the elevated relaunch on Windows.

The `fake` feature provides an in-memory machine for tests and the e2e
suite. It is never a default feature; `scripts/verify.sh` asserts that.

## Shell behaviour

- Window starts hidden with a matching background; the page reveals it after
  its first render (`frontend_ready`), with a 3 s safety net in Rust.
- Close hides to the tray when `close_to_tray` is set; otherwise it emits
  `confirm-quit` and the page decides. The `quit` command is the one exit and
  can restore first.
- Tray: left click toggles the window, the menu toggles Quiet Mode, and the
  icon/tooltip reflect state. Tray-driven toggles notify when the window is
  hidden.
- Single instance: a second launch reveals the running window.
- Autostart: `schtasks` logon task on Windows (elevated when created by an
  elevated process), `tauri-plugin-autostart` elsewhere, always guarded
  against registering a temporary or build-directory executable.

## State

`COMPUQUIET_DATA_DIR` overrides the platform config directory. Files are
`settings.json` (versioned; a newer version or corrupt file is an error, not
a reset) and `journal.json` (versioned). Writes are temp-file + rename.

## Verification

- Rust unit tests: policy, planner, journal, settings, store, fake adapter,
  engine round trip; real suspend/resume/close on a child process and real
  service/power queries on the host OS.
- Frontend tests (`node --test`): formatting and profile editing.
- WebDriver suite (`e2e/`): boot, the full quiet-then-restore workflow
  asserting the journal on disk, persistence across a restart, and a clean
  quit that restores first. Runs on Windows and Linux in `scripts/verify.sh`.
