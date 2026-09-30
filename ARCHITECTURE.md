# CompuQuiet architecture

Tauri 2 desktop app: a Rust engine behind a vanilla TypeScript window. The
page has no filesystem, shell or process permission; everything with an
effect happens in Rust behind a validated command.

```
ui/            vanilla TS + Vite: Home, Scan, Park list, Settings (with About)
src-tauri/     the shell: commands, engine, watch, cli, tray, autostart, update,
               diagnostics, log file, single-copy lock, window lifecycle
crates/
  cq-core/     domain, no OS calls: profile, policy, planner, scan, journal,
               settings, endings and the watch, single-copy requests
  cq-platform/ the Platform trait and its adapters: windows, linux, macos, fake
e2e/           WebdriverIO suite driving the real binary (fake platform)
scripts/       verify (full gate), fastcheck (inner loop), driver setup, and the
               release checks (version, tag, update key, manifest, checksums)
```

Dependencies point inward: `src-tauri` → `cq-platform` → `cq-core`.

## The flow

1. **Snapshot.** `Platform::snapshot` lists programs (via `sysinfo`; never
   threads), the state of the profile's services, and the active power plan.
2. **Plan.** `cq_core::build_plan` turns profile + snapshot + capabilities into
   ordered `Step`s and a list of skipped targets with reasons. Order: power
   plan, keep-awake, services, processes, memory purge. Critical processes and services
   (`policy.rs`), keep-alive entries (a process is also spared when only its
   own name matches, as a truncated Linux name does) and the app itself are
   never planned. `cq_core::guard_battery` then drops the performance power
   plan, keep-awake and the memory purge, listing them as left alone, when
   `Platform::on_battery` says `Some(true)` and `Settings::allow_on_battery`
   is off. Windows answers from `GetSystemPowerStatus`, Linux from the
   machine's own `Battery` supplies in `/sys/class/power_supply` (a
   peripheral's, `scope` Device, and a `UPS` never count) and macOS from
   `pmset -g batt` (`UPS Power` is not battery); `None`, a desktop, and a UPS
   on mains all mean mains.
   A target set to `ProcessAction::SlowDown` plans a `Step::SlowProcess` per
   process (none where `Capabilities::slow_down` is false: the program is
   left running as it is, never frozen instead, with the reason shown). Then
   `Engine::spare_sound`, only when the plan parks a program, asks
   `Platform::audio_users` which processes have a stream of sound running and
   `cq_core::guard_audio` drops the suspend, slow and close steps of every
   program that has one, or a helper that does (by `ProcessInfo::program_root`
   and the ancestors of the audible process), listing it as left alone. An
   answer it cannot get is skipped quietly where the platform cannot tell
   (`PlatformError::Unsupported`) and listed as "Sound" otherwise.
   With `Profile::unload_ai_models` on, `cq_core::plan_unloads` then adds a
   `Step::UnloadModel` for each model `Platform::loaded_models` finds, before
   the purge, and says what was left alone (none loaded; LM Studio's tool not
   run). The step has no undo (a model loads again when used) and so no
   `DoneStep`: a journal never names it, and 1.1.7 reads every journal.
   `Engine::plan_now` runs steps 1 and 2 and the guard for both a run and the
   read-only `Engine::preview`, so a preview cannot differ from what a press
   plans; the run always plans afresh and never uses an earlier preview.
3. **Execute and journal.** Each step's `DoneStep` is written to
   `journal.json` (atomic write) before the platform carries it out
   (`DoneStep::intended`, `Engine::run_journaled`), corrected afterwards if
   the platform reports something different, and taken back out if the step
   failed. A crash mid-step therefore leaves it on record, and every undo is
   safe for a step that never happened. A step that times out
   (`PlatformError::TimedOut`, code `timed_out`) may still take effect, so
   its entry stays; a program that has already gone is logged as done, not
   failed. A slowed program's entry (`DoneStep::ProcessSlowed`) is written
   with no `previous` pace and replaced by the one the platform read before
   changing anything; restoring an entry that never got it puts the usual
   pace back. Progress lines stream to the window; a failed step is logged and
   the run continues. A run with steps is measured by the engine
   (`Engine::measure`: two `stats()` readings `Platform::settle` apart, before
   the first step and after the last) into `EngineState::run_report`, in
   memory only, so a recovered run has none. It keeps the memory the suspended
   programs still hold apart from the closed ones' and from the change in
   available memory: freezing frees nothing, and the change is approximate.
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
   matched. The tools that only look (`pactl`, `xprop`, `lsappinfo`,
   `nvidia-smi`, `pmset`, the service listings) use `run_tool_within` with a
   deadline of their own. Windows decisions use error codes, never a tool's
   translated text; the forced close is `TerminateProcess`.
7. **Endings and the watch.** A run may end by itself (`Journal::ending`,
   `cq_core::watch::Ending`): at a deadline counted in the machine's uptime
   (`Platform::marker`, never the wall clock), when a program the user named
   has closed, or, for a run the watch started, when none of the auto-quiet
   programs has run for a while. `src-tauri/src/watch.rs` looks every five
   seconds (`Platform::processes`, only when there is something to look for),
   gives the look to the pure `Watch::poll` (start after 10 s, leave after
   30 s, in uptime) and does what it says through `run_transition`, so the
   busy guard, claim, journal and tray are a press's. It acts only when idle,
   never restores under a game that started meanwhile and never starts twice
   over one game (`disarmed`). A run it starts is `Plan::unattended` (suspend
   instead of close, no purge) and spares the program it started for
   (`Profile::protect` on that run's copy of the profile). `go_quiet` takes an
   `Until` from the page, checked in `Until::ending`; `Engine::set_ending`
   changes a run in progress. `Settings::still_on_hours` sets a reminder,
   said once, for a run nothing will end. Keep-awake (`Profile::keep_awake`,
   `Step::KeepAwake`) is a hold that ends with the process, so it has no undo
   entry: `Journal::awake` records it, `Engine::resume_awake` takes it up
   again for a recovered run, and restore lets go once the run is over.
   Windows holds `SetThreadExecutionState` on a thread of its own; Linux and
   macOS start `systemd-inhibit` or `caffeinate` in their own process group,
   bound to this process's pid (`cq-platform/src/awake.rs`).

8. **Profiles.** `Settings::profile` is the active profile (what a press runs
   and the Park list edits), `profile_name` names it and `other_profiles`
   holds the rest; `Settings::switch_to` swaps them, so 1.1.7, which knows
   only `profile`, still loads the file and runs the active one (and forgets
   the others if it saves). `Settings::normalize` (at load and on a save)
   gives every profile the active one's Never touch list, taking what is on
   it off their lists, and drops an auto-quiet choice whose program or profile
   is gone. A run names its profile to `runnable_settings` (the tray and the
   window: the active one; `--profile`: that one; a run the watch starts: the
   one chosen for its program in `AutoQuiet::profiles`, else the active one),
   which plans from `Settings::for_profile`, a copy; `Journal::profile`
   records it (optional, 1.1.7 ignores it) and `EngineState::run_profile`
   shows it. `engine/profiles.rs` is the only way the set of profiles
   changes (`switch_profile`, `add_profile`, `rename_profile`,
   `delete_profile`), and only with Quiet Mode off and no run going
   (`quiet_on`): the page's `save_settings` keeps the engine's profile name
   and `other_profiles`, and refuses with `profile_changed` when the page's
   profile is no longer the one in use (the tray switched meanwhile).

Processes are identified by PID plus start time so a reused PID is refused.

## The scan

`cq_core::recommend` turns a snapshot, the live stats, the platform's
`Activity` (which PIDs own a visible window, which is in front) and the
profile into ranked `Recommendation`s: catalogue matches (`catalogue.rs`, per
OS, each with a reason and a `Risk`), running services from the catalogue, a
non-performance power plan, a file cache above 1 GB, and, only when the
platform can report windows, unknown processes over 200 MB or 3% CPU that
own none, except the workloads in `catalogue::WORKLOADS` (interpreters, local
AI servers, the WSL VM), the family of the program in front and the helpers of
a program that has a window (a copy of its executable under another name, or a
Mac app's "Name Helper (Renderer)": freezing one freezes the program). Names on the
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
of its frame, so those are read too). Linux reads an X11 session with `xprop`
(`linux/x11.rs`): the window manager's `_NET_CLIENT_LIST`, `_NET_ACTIVE_WINDOW`
and each window's `_NET_WM_PID`, one call per window within a 4 s budget. It
reports `known = false` on Wayland (there is no list of other programs'
windows, and XWayland's holds only the older programs, so a program would read
as windowless), when a window names no process, or when the list cannot be
read whole. A Flatpak app's window names its PID inside the sandbox, so that
program can read as windowless: a Medium, unticked suggestion. macOS asks
`lsappinfo list` and `front` (`macos/apps.rs`), which says how each registered
app shows itself, not whether a window is open, so every `Foreground` or
`UIElement` app counts as having one and only `BackgroundOnly` ones and
programs Launch Services does not list are candidates. Its output is for
people: anything not understood (no entry with both `pid` and `type`) reports
`known = false`. Where `known` is false the scanner names only recognised
software.

## Platform adapters

`cq-platform` is the only crate allowed `unsafe`, and only in its Windows
module: `NtSuspendProcess`/`NtResumeProcess`, a program's priority class and
Efficiency mode (`SetPriorityClass`, `SetProcessInformation` with process
power throttling, the two that Task Manager's Efficiency mode sets), the token
elevation check, the standby-list purge, the file-cache figure (`GetPerformanceInfo`), the list
of services and a service's running dependents, window enumeration, starting a program with the desktop
shell's token (`CreateProcessWithTokenW`, so an elevated CompuQuiet does not
hand its rights on), the audio sessions (COM), holding off sleep
(`SetThreadExecutionState`) and the `runas` relaunch. Everything else uses
safe crates (`windows-service`, `sysinfo`, `nix`) or structured subprocess
calls with validated arguments (`powercfg`, `taskkill`, `systemctl`,
`powerprofilesctl`, `launchctl`, `schtasks`).

`Platform::slow_down` lowers a process to the lowest priority and returns how
it ran before (`Pace`); `speed_up` puts that back, but only what is still as
slow_down set it, so a priority or mode the program or the user has changed
since stays. Windows sets `IDLE_PRIORITY_CLASS` and Efficiency mode (before
1709, priority alone). Lowering a nice value is allowed to anyone and raising
it again is not (`renice` as a normal user answers "Permission denied"), so
Linux and macOS (`unix.rs`: `ps -o ni=`, `renice`) report `slow_down` only as
root. `Platform::audio_users` is the processes with a running stream:
Windows enumerates the audio sessions of every active render and capture
device through the `windows` crate's Core Audio interfaces (a Windows-only
dependency of `cq-platform` with just the audio and COM features), on a thread
of its own so COM is set up the way it needs; Linux asks
`pactl --format=json list sink-inputs` and `source-outputs` (`linux/audio.rs`)
and skips paused streams; macOS cannot tell.

`Platform::list_services` names every service the machine has for the Park
list's picker (`EnumServicesStatusExW`; `systemctl list-units`, system and
user; `launchctl list`). The engine turns it into `ServiceRow`s (`rows.rs`),
marking those `policy::is_critical_service` refuses as `essential`, which the
page keeps out of the picker but still uses to name a listed service. It is
a listing, not a check: on Linux and macOS a stopped or unloaded unit may be
missing from it, so an absent name is never reported as not installed.

`Platform::gpu` reads each graphics adapter's own memory (`gpu.rs`, shared by
the adapters as the trait's default): `nvidia-smi` on Windows and Linux (no
tool means no NVIDIA card, not an error) and, on Linux, the amdgpu driver's
`mem_info_vram_*` files. Any other card, and a Mac, whose graphics share the
system's memory, answers with the reason it cannot be read, and the page shows
that reason instead of a gauge. `get_gpu` returns a `GpuReading` (adapters, or
`unavailable`); one adapter failing leaves the others. Per-adapter, because a
model must fit in one card, and reading only, so nothing here changes the
machine. Which programs hold the memory is not read.

`Platform::loaded_models` and `unload_model` (`ai.rs`, shared as the trait's
defaults) talk to local model servers. Ollama is reached over HTTP/1.0 on
127.0.0.1 alone (only the port of `OLLAMA_HOST` is taken): `GET /api/ps`, then
`POST /api/generate` with `keep_alive` 0, as `ollama stop` does, all within
deadlines. LM Studio's REST API needs a key, so its `lms` tool is run
(`ps --json`, `unload <identifier>`), but only from `~/.lmstudio/bin` (never
found on the search path), only while an LM Studio process is in the snapshot
(the tool can start the app), and never when this process is elevated: the
folder is the user's, so an elevated run would hand its executable those
rights. A model's name is checked before it becomes an argument or JSON. The
fake keeps a model list (`fake_models`, `Call::UnloadModel`).

`Capabilities` reports what this process can do at its privilege level; the
planner skips what it cannot with the reason shown ("needs administrator
rights"), and the window offers the elevated relaunch on Windows.

The `fake` feature provides an in-memory machine for tests and the e2e
suite. It is never a default feature; `scripts/verify.sh` asserts that. It
can be told to fail (`Fake::fail(Call, target, Failure)`, `Fake::heal`): a
refusal, a missing-rights error, or a timeout whose effect still lands. The
e2e build exposes that as the `fake_fail` and `fake_heal` commands, which
exist only with the `fake-platform` feature, as do `simulate_tray_menu` (a
tray click, for the tray specs), `fake_program` (open or close a program),
`fake_advance` (move uptime on) and `fake_awake`, which the
timed and auto-quiet specs drive, and `fake_audio` (which programs have sound
running) and `fake_slowed` (which are slowed), which the slow-sound spec does.
`Call::SlowDown`, `SpeedUp` and `AudioUsers` can be made to fail.

## The window (`ui/`)

No framework: one module per view (`dashboard`, `scan`, `targets`,
`settings-view`, `run-length`, `auto-quiet`, `updates`, `diagnostics`,
`profiles`, `banner`), `main.ts` for boot and the flows that span views,
`tabs.ts` for the tab bar, and pure helpers with `node --test` tests
(`format`, `scan-select`, `profile-edit`, `park-list`, `ending-text`,
`preview-text`, `recovery-text`, `theme`). Home's line saying what a press
will do comes from `homePlan` in `format.ts`, so the page states it before
the user leaves Home. The Park list's rows are built in `park-rows.ts` and
its pickers (running programs, the machine's services) live in `pickers.ts`. Conventions that are not visible in the code:

- A control inside a row (a tick, an Action select) updates the state and the
  save button only; it never rebuilds the rows, which would drop the keyboard
  focus it was just used from. Rows are rebuilt on add, remove and reset, and
  a remove moves focus to the row that took its place.
- Scan ticks are keyed by kind and name, so they survive a rescan (a visit to
  another tab starts one) and rows moving. So do the "Close instead" marks,
  which only apply when a find is added: `Recommendation`s stay as the scan
  made them. Scan's *Never touch* saves through the page's settings like the
  Park list does, and `rebase` carries it into unsaved Park list edits.
- The Park list's filter draws only the rows that match (by name, and for a
  service by the machine's name for it) and clears itself when a row is
  added, so a new row is never hidden; the row's place for focus after a
  remove counts the rows drawn, not the whole list.
- One dialog at a time: a new request replaces the one showing, which ends as
  cancelled. While it shows, the header, banner and main view are `inert` and
  the Ctrl+1..4 shortcuts stand aside.
- A toast is painted for the eyes and spoken through the always-present
  `#announce` live region; error toasts are assertive.
- `document.hidden` stays false when the window is hidden to the tray in
  WebView2, so the stats poll asks the window itself (`isVisible`). The GPU
  gauge (`gpu.ts`) does the same, every five seconds and never two reads at
  once, because `nvidia-smi` takes a tenth of a second or more.
- Text colours come from the `--*-text` tokens, which the light theme darkens
  to 4.5:1; `theme.test.ts` checks both light blocks agree and every token
  passes. The window frame follows the Theme setting through `setTheme`.
- The big button carries its state in `data-quiet`, not `aria-pressed`: its
  label already says what a press does.
- Home's preview (`preview.ts`, text in `preview-text.ts`) is a `<details>`
  fetched when opened, on Look again and when Home is shown while open, only
  while Quiet Mode is off. The freed figure and the run lines come from
  `EngineState::run_report`, not from the page's own polling.
- Home's *How long* (`run-length.ts`, wording in `ending-text.ts`) asks for a
  timed or until-a-program run and goes back to *Until I put it back* after
  every start, so each timed run is asked for afresh. While Quiet Mode is on
  it shows the ending the engine reports, counting a timer down between
  reports, and can add an hour or drop the ending.
- The page's permissions are the three calls it makes itself (`listen`,
  `isVisible`, `setTheme`), listed in `src-tauri/capabilities/main.json`; add
  one only with the `ui/src` call that needs it. The e2e build adds four for
  the suite's own calls (`e2e/tauri.conf.json`), and `permissions.spec.ts`
  checks that the tray, app and other window controls stay refused.

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
  menu and puts it back after an icon change, which otherwise drops it. The
  menu's *Profile* submenu (`tray/profiles.rs`) has one ticked entry per
  profile, its id being `tray-profile:` and the name, so a click names what
  was chosen even after the menu went stale; it is rebuilt only when the set of
  profiles changes, the ticks are set again on every refresh (a click ticks
  an entry whether or not the switch was allowed), and the entries are
  disabled while Quiet Mode is on or a run is going. A switch from the tray
  reaches the window as the `settings-changed` event, and the toggle reads
  "Free up this PC (Gaming)" when there is more than one profile.
- A window whose WebView2 failed to start is only logged by Tauri; its handle
  stays registered but every query on it fails. `tray::reveal` treats that as
  "no window" and `reopen` restarts the process once with `--reopen` (shown, never
  hidden), waiting for any Quiet Mode run to finish first. A reopen that
  fails too notifies instead of restarting again, whatever the notifications
  preference says.
- `logfile` sends every crate's `log` warnings and errors, and any panic (a
  release build aborts on one and has no console), to `compuquiet.log` in the
  data directory; it is the record of why a window failed to load or a step
  failed. The engine logs each failed step by label and error code, never a
  program's arguments, and `run_transition` logs a run that failed outright. At
  launch a file over 512 KiB becomes `compuquiet.log.1` (replacing any older
  one) and a new one starts, so the evidence of the last stretch survives.
- A run started from the tray (`tray::toggle_from_tray`, `quit_from_tray`) that
  fails, or a restore that leaves entries, is reported by `report_failure`
  whatever the notifications preference says: the `quiet-error` event carries
  the `AppError` for the page to toast, and the window comes forward. Only
  success notices depend on the preference and a hidden window. Tray Quit
  treats `busy` and `not_quiet` from its restore as "look again".
- One copy per data directory (`cq_core::instance`, `single.rs`): the first
  copy holds an exclusive lock on `instance.lock` for the life of the process.
  A later launch leaves a request in `wake/` (one command word from a fixed
  list, and for `quiet` and `toggle` `:` and a profile's name, checked like any
  name; never a target or a setting) and waits up to 3 s for the running copy, which polls
  twice a second, to take it; the window is then shown (`Command::Show`), or
  the command line's `--quiet`, `--restore` or `--toggle` is done
  (`Command::{Quiet, Restore, Toggle}`, `cli.rs`). The files carry the user's
  access list, so an unelevated launch reaches an
  elevated copy, which window messages and named objects cannot. A lock freed
  during the wait is taken over: a restart or an elevated relaunch starts
  before the old process is gone and both call `single::release` first. A
  copy that does not answer exits with code 1. The elevated-to-unelevated pair
  is a manual check; the suite runs from one unelevated shell.
- The command line (`cli.rs`): `parse` runs first and refuses any argument it
  does not know (exit 2, before the data directory is touched); a command names
  no target, so the running copy does it from its saved settings through
  `run_transition`. `--profile NAME` (or `=NAME`) goes with `--quiet` or
  `--toggle` only, and a name that is not among the saved profiles (read from
  `settings.json`, `check_profile`) is refused at once with 2; the running copy
  looks the name up again. Commands wait in a queue, are done in the order sent and
  each waits (up to 3 minutes) for a run that is going; one is skipped when the
  machine is already as asked. It reports through `announce` and `alert`
  (`watch.rs`), never bringing the window forward. A launch with a command that finds no copy becomes the running
  copy, hidden, and does it after the recovery of an earlier sign-in
  (`cli::start`). The updater relaunches Windows with Tauri's `Env`
  arguments, so `launch_env` is managed first and lacks the command; a
  warning in the log says if Tauri stopped keeping it.
- Updates (`update.rs`, `update/guard.rs`): only a copy that can replace itself
  checks (Windows with `uninstall.exe` beside it, Linux with `APPIMAGE`, macOS
  inside a `.app`; never a debug or fake build). A resident copy considers a
  check every 30 minutes; one that got an answer is repeated after 6 hours, a
  failed one after 1 hour. The manifest check has a 30 s limit and the download 10
  minutes, so a stalled connection cannot hold the single attempt slot. A
  found update is downloaded at once but installed only when no run is going,
  Quiet Mode is off and the window is hidden (installing ends the process), and
  the restart goes through `single::restart`, as `reopen` does. With
  `Settings::auto_update` off (default on; saving it on again checks at once)
  the checks go on but a found release is only announced
  (`Status::Available`): nothing is downloaded, and a download already waiting
  is dropped. `update::Status` (in `update/progress.rs`; commands
  `update_status`, `check_for_updates`; event `update-status`) is what the
  window shows. The Windows setup is per-machine, so an unelevated
  copy's install raises the permission prompt (`ShellExecute` "open" on an
  administrator-manifest installer): `Status::Ready.asks_permission` and the
  "Updating to ..." notification say so beforehand, and a refusal is a `Failed`
  status with the ordinary 6-hour look next, not the hourly retry. An
  elevated copy installs silently.
- Diagnostics (`diagnostics.rs`, `ui/src/diagnostics.ts`): the `diagnostics`
  command takes no argument and returns one text, which the page writes to the
  clipboard (and, if the webview refuses that, shows in a box to copy by hand).
  It is built in one place: `machine` (version, system, capabilities, data
  folder, park list counts, update status), `quiet_mode` (when it began, the
  journal's steps from `Engine::steps_on_record` / `DoneStep::describe`, which
  name a step and never carry a command line, folder or path; what a restore
  left; the last run; what was left alone) and the last 16 KiB of
  `compuquiet.log` from a line start (`logfile::tail`). Every list is cut at
  40 items. `redact` then shows the home folder as `~` in the whole text, in
  any spelling of the path, so no part can be forgotten. It is never sent
  anywhere. An 8.3 short spelling of the home folder (`NAME~1`) is not
  recognised.
- Autostart: `schtasks` logon task on Windows (elevated only when created by
  an elevated process *and* the exe is under Program Files, where only
  administrators can replace it; otherwise unelevated, and
  `AutostartStatus.limited_because` says why), `tauri-plugin-autostart`
  elsewhere, always guarded
  against registering a temporary or build-directory executable (on Linux also
  an AppImage path a login entry would split or expand: spaces, `%`, quotes).
  An entry made with administrator rights is `locked` for an unelevated copy,
  which cannot delete or replace it: `set` refuses (`autostart_locked`) and the
  switch is disabled with the reason. The task always passes `--hidden`, so a
  sign-in starts in the tray whatever "Start hidden" says. At start-up an
  installed copy (`uninstall.exe` beside it) points a task that names another
  exe at itself (`autostart::reconcile`, `windows.rs`): the entry keeps its
  rights, an unelevated one is never upgraded, and an elevated one that an
  unelevated copy cannot change is only logged.
- Windows setup (`bundle.windows.nsis`): NSIS with `installMode: perMachine`
  (Program Files, one permission prompt) and `src-tauri/windows/hooks.nsh`.
  Before installing, the hook uninstalls a per-user copy found under `HKCU`
  (release 1.1.7 and earlier) silently and without touching the data folder,
  then recreates its Start menu and desktop shortcuts for all users and runs
  `schtasks /Change` on the sign-in task, which the elevated setup can do to an
  elevated task and the unelevated app it starts afterwards cannot.
  Uninstalling deletes the task (not on an update). The setup itself is not
  exercised by the gate; it is checked by hand at the local handoff.

## State

`COMPUQUIET_DATA_DIR` overrides the platform config directory (the old
`COMPUTEQUIET_DATA_DIR` still works, and a `ComputeQuiet` folder is adopted
when `CompuQuiet` does not exist yet). Files are `settings.json` (versioned;
named profiles are fields added to it, not a new version) and `journal.json`
(versioned), beside `instance.lock`, the `wake/` requests, `compuquiet.log`
and the `.bad` and `.1` copies named below. Writes are
temp-file + rename, and the rename and reads are retried briefly on the
Windows errors a scanner holding the file causes. A newer or corrupt
`settings.json` is an error, not a reset: the engine runs on the defaults but
refuses saves and Go Quiet (`EngineState::settings_unreadable`) until the
banner's action moves the file to `settings.json.bad`.

## Verification

- Rust unit tests: policy, planner, scan, journal, settings, store, fake
  adapter, engine round trip and failure paths (`engine/failure_tests.rs`);
  real suspend/resume/close on a child process and real service/power queries
  on the host OS.
- Frontend tests (`node --test`, `npm test`): the pure helpers listed under
  the window, and the release scripts (`latest-json`, `check-update-key`).
- WebDriver suite (`e2e/`, the order in `wdio.conf.ts`, one session): `boot`,
  `quiet` (the full quiet-then-restore workflow asserting the journal on
  disk), the run's preview, report and options (`preview`, `run-report`,
  `ai-models`, `slow-sound`, `gpu`), `scan` and `scan-actions`, `park-list`,
  `ui` and `keyboard` (focus, dialogs, theme, polling while hidden,
  screen-reader names), `persist`, the failure paths (`settings-guard`,
  `stuck-restore`, `tray-failure`, `diagnostics`), the automatic ones
  (`timed`, `auto-quiet`, `profiles`, `update`), `permissions` (what the page
  may ask of the shell), and the launches (`cli`, `second-launch`, `exit`: a
  clean quit that restores first). Runs on Windows and Linux in
  `scripts/verify.sh`. A spec that leaves unsaved page state must reload the
  session. WebView2 posts IPC as host messages and
  `__TAURI_INTERNALS__.invoke` is locked, so `watchInvokes` (in `support.ts`)
  stands in front of the transport to count calls or make one fail.
- The suite refuses a binary that lacks the fake platform (it looks for the
  fake machine's `C:/fake/` paths in the executable), so a real-platform build
  at `target/debug` is never driven; `tsc -p e2e` type-checks the specs in the
  gate. A failed test leaves its own folder (screenshot, page source and the
  data directory's log, journal and settings) in `$RUNNER_TEMP/compuquiet-failure`,
  which both workflows upload.
- CI (`.github/workflows/`): `verify` runs the gate on all three systems for
  every commit on main (only pull requests cancel superseded runs); `release`
  repeats it on the tag (`scripts/check-release-tag.sh`: the tag names the
  manifests' version), builds the installers and portable copies with the
  updater bundles signed by `TAURI_SIGNING_PRIVATE_KEY`, checks that the
  `.sig` files were made by the key in `plugins.updater.pubkey`
  (`scripts/check-update-key.mjs`, key IDs only), adds per-platform SHA-256
  sums and the `latest.json` the updater reads (`scripts/latest-json.mjs`:
  one signed Windows, Linux and macOS bundle each, never the portable copy),
  and only then publishes; `audit` runs `cargo audit` and
  `npm audit --omit=dev` weekly and only reports. `verify` and `release`
  install exactly the compiler `rust-toolchain.toml` pins. Nothing is
  code-signed with a publisher certificate or notarised: the bundles carry
  only the updater's minisign signature, which the app itself checks.
