# CompuQuiet — working context

`ARCHITECTURE.md` explains the structure; this file records the decisions and
constraints that are not visible in the code.

## Decisions

- **2026-09-30: a preview is a look, a run measures itself, battery holds
  back power.** `Engine::plan_now` serves the run and the read-only preview;
  a press always plans afresh. `RunReport` (memory only) is read by the
  engine around a run. On battery (`Platform::on_battery`; unknown, a desktop
  and a UPS count as mains) the power plan and purge are skipped unless
  `allow_on_battery`.
- **2026-09-30: the Windows setup is per-machine (Program Files).** The
  elevated logon task must start a program ordinary processes cannot replace:
  it is HIGHEST only for an exe under `%ProgramFiles%` made by an elevated
  process, else unelevated with the reason shown. `src-tauri/windows/hooks.nsh`
  removes a 1.1.x per-user copy (`%LOCALAPPDATA%\CompuQuiet`; data is
  untouched), re-points the task and deletes it on uninstall;
  an installed copy re-points a stale task at start-up. An unelevated copy's
  update raises UAC, announced. Rollback: uninstall, install a 1.1.7 setup;
  settings and journal are unchanged.
- **2026-09-30: a step that times out stays on record, and a stuck restore
  can be given up.** A timeout (`PlatformError::TimedOut`) does not prove the
  step failed (a busy service stops late), so its entry stays and Restore
  undoes it harmlessly. Only what a restore could not undo
  (`EngineState::unrestored`) is offered to give up, after a confirmation
  naming each entry; `journal.json` moves to `journal.json.bad`, never
  deleted, as an unreadable journal does.
- **2026-09-30: the memory purge is opt-in.** A new profile defaults to no
  purge on every platform; a saved setting is kept as it is. Scan may suggest
  it; auto-scan never switches it on.
- **2026-09-30: an unreadable `settings.json` is never overwritten.** A
  damaged or newer file leaves the engine on the built-in settings, refusing
  every save and Go Quiet; the banner moves it to `settings.json.bad`.
  Restore is unaffected.
- **2026-09-30: one copy per data directory, by file lock.** The first copy
  locks `instance.lock`; a later launch leaves a request in `wake/` (a fixed
  command word, no arguments; `cq_core::instance`) and exits once the running
  copy takes it (window messages cannot cross the elevation boundary).
  `go_quiet` re-reads `journal.json` and adopts it.
- **2026-09-29: a restart or new sign-in ends Quiet Mode.** The journal
  records where Quiet Mode began (optional `began`: uptime and, on Windows,
  the WTS sign-in's logon stamp), never compared with the wall clock, which
  jumps by hours on this dual-boot PC. An earlier sign-in's journal is
  finished at launch: closed programs are not relaunched, services restart
  unless uptime shows a reboot, resumes are always tried and the power plan
  is always restored. Older journals, and Linux/macOS sign-outs without a
  reboot, restore everything.
- **2026-09-30: Linux start times are recorded from boot.** A clock step made
  a resume look like another program. `ProcessInfo.start_time` is seconds
  since boot; 10^9 or more is an older wall-clock value, still accepted.
  Restore before downgrading to 1.1.7, which would not resume those.
- **Every step is on record before it happens, and every undo is safe to
  repeat.** A crash mid-step or mid-restore strands nothing and repeats
  nothing; restore saves after each step and never relaunches a command line
  that is already running.
- **Leaving never cuts a run short.** Quit, the tray's Quit, elevated
  relaunch and the updater's install claim the engine
  (`Engine::claim_for_exit`) and wait or refuse while a run is in progress.
- **2026-09-29: a window that never loaded restarts the app once.** The
  elevated logon launch sometimes gets no WebView2, which Tauri only logs,
  leaving a tray with nothing behind it. Tray actions and a second launch
  restart with `--reopen`.
- **2026-09-30: failures are kept.** `compuquiet.log` holds warnings, errors,
  failed steps (label and code, never program arguments) and panics; a full
  file becomes `.log.1`.
- **2026-09-26: renamed to CompuQuiet.** A leftover `ComputeQuiet` settings
  folder, logon task or env override is still recognised. Tray actions run
  in Rust, so they work with the window hidden.
- **2026-09-19: rewritten as Rust + Tauri 2** (was C#/WPF), cross-platform;
  vanilla TypeScript + Vite frontend; three-crate workspace.
- **Unelevated by default on Windows.** Services and the purge need
  administrator rights, but elevation at launch would block a prompt-free
  autostart and the WebDriver suite. The app plans around its capabilities
  and offers *Relaunch as administrator*.
- **Suspend is the default process action; Close is opt-in.** Suspending
  keeps a program's state and is fully reversible; closing frees its memory
  but loses unsaved state, so it is per target.
- **Fake platform behind a cargo feature** for the e2e suite, which drives the
  real binary with only the OS adapter swapped. `verify.sh` asserts the
  feature is not a default and not in `tauri.conf.json`.
- **2026-09-25: GitHub is the only remote.** Origin has no runners or
  releases, so `Swatto86/CompuQuiet` on GitHub is the source of truth,
  workflows and releases. Swatto mirrors it to Origin; this clone has no
  Origin remote. Push to `origin` (GitHub) only.
- **2026-09-27: updates install themselves.** `tauri-plugin-updater` checks
  `latest.json` on the GitHub release when idle. The release workflow signs
  the NSIS installer, AppImage and macOS `.app.tar.gz` with the minisign key
  in `TAURI_SIGNING_PRIVATE_KEY` (public half in `tauri.conf.json`). Only a
  copy that can replace itself checks (`update/guard.rs`; never a debug or
  fake build), once idle with the window closed to the tray. With
  `auto_update` off a release is only announced, never fetched.
- **2026-09-30: diagnostics stay local.** `diagnostics.rs` hides the home
  folder as `~` in the whole report, names steps but never arguments, and
  only reaches the clipboard.
- **2026-09-19 (1.1.0): the scanner acts on low risk only.** `auto_scan` is
  on by default and parks low-risk finds for that run without editing the
  saved targets; medium-risk finds (browsers, launchers, voice chat, Office)
  are shown on the Scan tab and never applied unasked. Unknown programs are
  suggested only where the platform can prove they own no window (Windows).
  Catalogue: `crates/cq-core/src/catalogue.rs`; an entry needs a reason and a risk.
- **Linux elevation is per action through polkit** (`systemctl` for system
  units, `pkexec` for the cache drop), never a root relaunch of the GUI.
  macOS reports power and memory actions as unavailable rather than
  half-doing them.

## Workflow

- Single branch `main`; commit and push verified units.
- Inner loop: `npx tauri dev`; `scripts/fastcheck.ps1` / `.sh`.
- Full gate: `scripts/verify.ps1` / `.sh` (fmt, clippy, tests, frontend, debug
  build with the fake platform, WebDriver suite). Windows needs
  `scripts/setup-e2e.ps1` once per WebView2 update.
- Release: bump the version in `Cargo.toml`, `src-tauri/tauri.conf.json` and
  `package.json` (the gate checks agreement), `AGENT_RELEASE=1 npx tauri build`
  for the local install, wait for `verify` to pass on GitHub for that commit,
  then push tag `vX.Y.Z` to GitHub to publish the release.

## Known limits

- Elevated on Windows, relaunched programs get the desktop shell's token
  (unelevated); with none to borrow they get CompuQuiet's rights and a warning.
- A relaunched program gets CompuQuiet's environment (minus an AppImage's
  bundle variables), not its original launcher's. A Flatpak, Snap or Store
  app is suspended instead of closed: it cannot be relaunched from here.
- Programs that respawn themselves (updater schedulers) are suspended, not
  closed, by default.
- The Linux process name from the kernel is 15 bytes; matching also uses the
  executable's file stem and a prefix rule.
- The e2e suite does not run on macOS (`tauri-driver` has no macOS backend).
