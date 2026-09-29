# CompuQuiet — working context

Every agent loads this file itself. `ARCHITECTURE.md` explains the structure; this file records
the decisions and constraints that are not visible in the code.

## Decisions

- **2026-09-29: a restart or new sign-in ends Quiet Mode.** A journal older
  than the sign-in is finished at launch: parked processes are gone and
  self-starting programs are back, so nothing is resumed or relaunched with
  stale arguments; services restart only if the OS did not reboot (Fast
  Startup and sign-out keep them stopped); the power plan is always restored.
  Windows reads the sign-in time from WTS; Linux and macOS use the boot time,
  so a sign-out there without a reboot still relaunches closed programs.

- **2026-09-29: a window that never loaded restarts the app once.** The
  elevated logon launch sometimes gets no WebView2; Tauri only logs that, so
  the tray ran with no window behind it and every item looked dead. Tray
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
  of its own (CI there needs Depot or Buildkite, Linux-only or self-hosted), so
  `Swatto86/CompuQuiet` on GitHub is the source of truth, runs the workflows
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
  suggested only where the platform can prove they own no window (Windows),
  so a Linux or macOS user's IDE is never guessed at. The catalogue lives in
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

- A program closed and relaunched inherits CompuQuiet's elevation if it was
  relaunched from an elevated instance.
- Programs that respawn themselves (updater schedulers) are suspended, not
  closed, by default for that reason.
- The Linux process name from the kernel is 15 bytes; matching also uses the
  executable's file stem and a prefix rule.
- The e2e suite does not run on macOS (`tauri-driver` has no macOS backend).
