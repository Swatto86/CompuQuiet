# CompuQuiet — working context

Every agent loads this file itself. `ARCHITECTURE.md` explains the structure; this file records
the decisions and constraints that are not visible in the code.

## Decisions

- **2026-09-30: a step that times out stays on record, and a stuck restore
  can be given up.** A timeout (`PlatformError::TimedOut`) does not prove the
  step failed (a busy service stops late), so its journal entry stays and
  Restore undoes it harmlessly; only definite failures take the entry out.
  What a restore could not undo (`EngineState::unrestored`) is the only thing
  Home offers to give up, after a confirmation naming each entry;
  `journal.json` then moves to `journal.json.bad`, never deleted. An unreadable
  journal is set aside the same way.
- **2026-09-30: the memory purge is opt-in.** A new profile defaults to no
  purge on every platform; a saved setting is kept as it is. The Scan tab may
  still suggest it, but auto-scan never switches it on
  (`low_risk_additions` skips it).
- **2026-09-30: an unreadable `settings.json` is never overwritten.** A
  damaged or newer file leaves the engine on the built-in settings, refusing
  every save and Go Quiet. The banner's action moves the file to
  `settings.json.bad` (then `.bad-2`, never replacing a copy); a file that no
  longer exists is simply resolved.
  Restore is unaffected. `store.rs` retries the rename and the read for the
  Windows access-denied and sharing errors an antivirus scan causes.
- **2026-09-30: one copy per data directory, by file lock.** The first copy
  locks `instance.lock`; a later launch leaves a request in `wake/` (a fixed
  command word, no arguments; `cq_core::instance`) and exits once the running
  copy takes it. Windows blocks window messages from an unelevated launch to
  an elevated copy, which the old single-instance plugin used. `go_quiet`
  re-reads `journal.json` and adopts it.
- **2026-09-29: a restart or new sign-in ends Quiet Mode.** The journal
  records where Quiet Mode began (optional `began`: uptime and, on Windows,
  the WTS sign-in's logon stamp), never compared with the wall clock, which
  jumps by hours on this dual-boot PC. A journal from an earlier sign-in is
  finished at launch: closed programs are not relaunched with stale
  arguments, services restart unless uptime shows a reboot, resumes are always tried (the PID check
  refuses anything else) and the power plan is always restored. Older
  journals, and Linux/macOS sign-outs without a reboot, restore everything.
- **2026-09-30: Linux start times are recorded from boot; only programs are
  listed.** A clock step between runs (this PC's jumps by hours) made a
  resume look like "a different program" and dropped it, leaving it stopped. `ProcessInfo.start_time` is now
  seconds since boot on Linux; a recorded value of 10^9 or more is an older
  wall-clock one, still accepted; restore before downgrading to 1.1.7, which
  would not resume those. Threads are never listed.
- **Every step is on record before it happens, and every undo is safe to
  repeat.** A crash mid-step or mid-restore strands nothing and repeats
  nothing; restore saves after each step and never relaunches a command line
  that is already running.
- **Leaving never cuts a run short.** Quit, the tray's Quit, elevated
  relaunch and the updater's install claim the engine
  (`Engine::claim_for_exit`) and wait or refuse while a run is in progress.

- **2026-09-29: a window that never loaded restarts the app once.** The
  elevated logon launch sometimes gets no WebView2; Tauri only logs that, so
  the tray ran with no window behind it. Tray
  actions and a second launch now restart with `--reopen`, and warnings and
  errors go to `compuquiet.log` in the data directory so the cause is kept.

- **2026-09-26: renamed to CompuQuiet.** Product, crate (`compuquiet`),
  identifier (`co.swatto.compuquiet`), data dir and env vars follow the new
  name; a leftover `ComputeQuiet` settings folder / logon task / env override
  is still recognised so upgrades keep state. Tray menu actions run entirely
  in Rust so Quit / Open / toggle work even when the window is hidden.
- **2026-09-19: rewritten as Rust + Tauri 2, cross-platform.** The previous
  C#/WPF Windows-only app in this repository's history was replaced in full.
  Vanilla TypeScript + Vite frontend, no framework; three-crate workspace.
- **Unelevated by default on Windows.** Services and the memory purge need
  administrator rights, but requiring elevation at launch would block a
  prompt-free autostart and make the WebDriver suite un-runnable from an
  unelevated shell. The app runs unelevated, plans around its capabilities,
  and offers *Relaunch as administrator*; the logon task is created elevated
  only when the creating process is elevated.
- **Suspend is the default process action; Close is opt-in.** Suspending
  keeps a program's state and is fully reversible; closing frees its memory
  but loses unsaved state, so it is chosen per target.
- **Fake platform behind a cargo feature** for the e2e suite. The suite drives
  the real binary; only the OS adapter is swapped. `verify.sh` asserts the
  feature is not a default and not in `tauri.conf.json`.
- **2026-09-25: GitHub is the only remote.** Origin has no runners or releases
  of its own, so `Swatto86/CompuQuiet` on GitHub is the source of truth, runs the workflows
  and hosts the releases. Swatto mirrors it to Origin himself; this clone has
  no Origin remote. Push to GitHub (`origin`) only.
- **2026-09-27: updates install themselves.** `tauri-plugin-updater` checks
  `latest.json` on the GitHub release when the app is idle (not in Quiet
  Mode, not mid-transition). The release workflow signs the NSIS installer,
  the AppImage and the macOS `.app.tar.gz` with the minisign private key in
  `TAURI_SIGNING_PRIVATE_KEY`; the public half is in `tauri.conf.json`.
  Debug and fake-platform builds never check. The `.deb` and portable
  binaries are not updated in place.
- **2026-09-19 (1.1.0): the scanner acts on low risk only.** `auto_scan` is
  on by default and parks low-risk finds for that run without editing the
  saved targets; medium-risk finds (browsers, launchers, voice chat, Office)
  are shown on the Scan tab and never applied unasked. Unknown programs are
  suggested only where the platform can prove they own no window (Windows).
  The catalogue lives in
  `crates/cq-core/src/catalogue.rs`; adding an entry needs a reason and a risk.
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
  closed, by default for that reason.
- The Linux process name from the kernel is 15 bytes; matching also uses the
  executable's file stem and a prefix rule.
- The e2e suite does not run on macOS (`tauri-driver` has no macOS backend).
