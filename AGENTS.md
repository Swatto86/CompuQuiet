# CompuQuiet — working context

`ARCHITECTURE.md` explains the structure; this file records what the code
does not show.

## Decisions

- **2026-09-30: a preview is a look; battery holds back power.** One planner
  (`Engine::plan_now`) serves both; a press plans afresh. On battery (unknown,
  a desktop or a UPS is mains) the power plan, keep-awake and purge are
  skipped unless `allow_on_battery`.
- **2026-09-30: Quiet Mode can start or end by itself, safely.** Journal
  `ending` and `awake` are optional (1.1.7 ignores them). Auto-quiet is off by
  default, suspends instead of closing, skips the purge, spares its programs
  and never ends or restarts a run the user pressed for. Keep-awake dies with
  the process: no undo entry.
- **2026-09-30: the Windows setup is per-machine (Program Files).** The
  elevated logon task must start a program ordinary processes cannot replace:
  HIGHEST only for an exe under `%ProgramFiles%` made by an elevated process,
  else unelevated with the reason shown. `src-tauri/windows/hooks.nsh` removes
  a 1.1.x per-user copy (data untouched) and re-points the task; only a
  person's uninstall deletes it, not an update or a setup run over a copy. An
  unelevated copy's update raises UAC; an elevated one's restarts elevated,
  from the setup, not its `/R`. Rollback: install 1.1.7; data is kept.
- **2026-09-30: a step that times out stays on record, and a stuck restore
  can be given up.** A timeout does not prove the step failed (a busy service
  stops late), so its entry stays and Restore undoes it harmlessly. What a
  restore could not undo can be given up after a confirmation; the journal
  moves to `journal.json.bad`.
- **2026-09-30: the memory purge is opt-in.** A new profile has no purge; a
  saved setting is kept. Scan may suggest it; auto-scan never switches it on.
- **2026-09-30: an unreadable `settings.json` is never overwritten.** A
  damaged or newer file leaves the engine on the built-in settings, refusing
  every save and Go Quiet; the banner moves it to `settings.json.bad`.
- **2026-09-30: one copy per data directory, by file lock; the command line
  rides on it.** The first copy locks `instance.lock`; a later launch leaves
  a request in `wake/` (`cq_core::instance`: a fixed word, no arguments),
  which crosses elevation as window messages do not. `--quiet`, `--restore`,
  `--toggle` and `--profile NAME` are a public promise: any other argument
  exits 2, and a command acts on saved settings only. Restarts drop them from
  Tauri's `Env` (an update would replay them) and go to the tray.
- **2026-09-30: named profiles, in a file 1.1.7 still reads.** `profile`
  stays the active one, `profile_name` names it, `other_profiles` holds the
  rest; a switch swaps them. Never touch is one list for all. A run may name
  its profile (`--profile`, `auto_quiet.profiles`) without switching; profiles
  change only while Quiet Mode is off. Rollback: 1.1.7 runs the active
  profile and its next save drops the rest; copy `settings.json`.
- **2026-09-29: a restart or new sign-in ends Quiet Mode.** The journal
  records where it began (optional `began`: uptime and, on Windows, the WTS
  logon stamp), never the wall clock, which jumps by hours on this dual-boot
  PC. An earlier sign-in's journal is finished at launch; older journals
  restore everything.
- **2026-09-30: Linux start times are from boot** (a clock step faked a new
  program); 10^9 or more is an older wall-clock value,
  still read.
- **Every step is on record before it happens, and every undo is safe to
  repeat.** A crash mid-step or mid-restore strands and repeats nothing;
  restore saves after each step and never relaunches a running command line.
- **Leaving never cuts a run short.** Quit, elevated relaunch and the
  updater's install claim the engine (`Engine::claim_for_exit`).
- **2026-09-29: a window that never loaded restarts the app once**
  (`--reopen`): an elevated logon launch sometimes gets no WebView2.
- **2026-09-26: renamed to CompuQuiet.** A leftover `ComputeQuiet` settings
  folder, logon task or env override is still recognised.
- **Unelevated by default on Windows**: elevation at launch would block
  autostart and the WebDriver suite. It offers *Relaunch as administrator*.
- **Suspend is the default process action; Close and Slow down are opt-in,
  per target**: closing loses unsaved state; slowing suits a program that
  breaks when frozen. A slowed target is saved as `suspend` plus
  `slow_down: true`, so 1.1.7 loads it as a suspend but cannot read the
  `process_slowed` journal kind: restore before downgrading. A program with
  sound running is left alone (`guard_audio`), which only removes steps.
- **The fake platform is a cargo feature** for the e2e suite; the gate
  asserts it is never a default.
- **2026-09-25: GitHub is the only remote** (Origin has no runners or
  releases); Swatto mirrors it to Origin. Push to `origin` (GitHub) only.
- **2026-09-27: updates install themselves.** `tauri-plugin-updater` reads
  `latest.json` on the GitHub release; bundles are signed with a minisign key
  (public half in `tauri.conf.json`). Only a copy that can replace itself
  checks (`update/guard.rs`; never debug or fake), and installs with the
  window closed to the tray. `auto_update` off only announces a release.
- **2026-09-30: unloading local AI models is opt-in and loopback-only.** A
  single-model `llama-server` cannot unload, so it is a journaled close, left
  alone if busy, secret-bearing or run by a service manager. `lms` runs only
  from `~/.lmstudio/bin`, never elevated.
- **2026-09-30: diagnostics stay local**: home folder as `~`, step names never
  arguments, clipboard only.
- **2026-09-30: a removed target stays removed, and essential services are
  never stopped.** Removal records *Never touch* (scan and planner honour it);
  `is_critical_service` names are skipped, and refused when newly saved.
- **2026-09-19 (1.1.0): the scanner acts on low risk only.** `auto_scan` is
  on by default and parks low-risk finds for that run without editing the
  saved targets; medium-risk finds are shown, never applied unasked. Unknown
  programs are suggested only where the platform can say which own a window
  (Windows, macOS, X11), never a helper of one that does. Catalogue entries
  need a reason and a risk.
- **Linux elevation is per action through polkit** (`systemctl` for system
  units, `pkexec` for the cache drop), never a root relaunch of the GUI.
  macOS reports power and memory actions as unavailable.

## Deliberately not doing

- Nothing the journal cannot undo (registry, `bcdedit` or service start-type
  changes, cleaners), except the documented no-undo steps: purge, model
  unload, keep-awake.
- No repeating memory-trim or purge loops: the one optional purge stays one.
- No real-time priority, affinity pinning, overclocking or registry tweak packs.

## Workflow

- Single branch `main`; commit and push verified units.
- Inner loop: `npx tauri dev`; `scripts/fastcheck.ps1`/`.sh`.
- Full gate: `scripts/verify.ps1` / `.sh` (fmt, clippy, tests, frontend, fake
  build, WebDriver). Windows needs
  `scripts/setup-e2e.ps1` once per WebView2 update.
- Release: bump the version in `Cargo.toml`, `src-tauri/tauri.conf.json` and
  `package.json` (the gate checks agreement), `AGENT_RELEASE=1 npx tauri build`
  for the local install, wait for `verify` to pass on GitHub for that commit,
  then push tag `vX.Y.Z` to publish. That build signs the updater bundles, so
  it needs `TAURI_SIGNING_PRIVATE_KEY` (path in host memory), or
  `--config '{"bundle":{"createUpdaterArtifacts":false}}'` for an unsigned one.

## Known limits

- Elevated on Windows, relaunched programs get the desktop shell's token
  (unelevated); where that cannot be used they get CompuQuiet's rights and a
  warning.
- A relaunched program gets CompuQuiet's environment (minus an AppImage's
  bundle variables). A Flatpak, Snap, AppImage or Store app is suspended
  instead of closed: it cannot be relaunched from here.
- Programs that respawn themselves (updaters) are suspended, not closed.
- The e2e suite does not run on macOS (`tauri-driver` has no macOS backend).
