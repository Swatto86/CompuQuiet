# CompuQuiet

Frees the machine for games and local AI, then puts everything back.

One switch parks the background work that competes for CPU, memory and disk:
telemetry and indexing services, sync clients, updaters, chat apps. It moves
Windows to its performance power plan and, if you turn that on, purges cached
memory once the rest is out of the way. Switching it off restores every service, resumes or
relaunches every program and returns the power plan, in reverse order. Each
step is written to an undo journal before the next one runs, so a crash or a
reboot cannot lose the list of what to put back.

![CompuQuiet in Quiet Mode](docs/quiet.png)

## What it does

| Action | Windows | Linux | macOS |
| --- | --- | --- | --- |
| Suspend a program (frozen in place, resumed on restore) | `NtSuspendProcess` | `SIGSTOP` | `SIGSTOP` |
| Slow a program down (lowest priority, Efficiency mode; put back on restore) | `SetPriorityClass`, `SetProcessInformation` | `renice`, as root only | `renice`, as root only |
| Close a program and relaunch it on restore | `taskkill` (a program with a window), then `TerminateProcess` | `SIGTERM`, then `SIGKILL` if it stays | `SIGTERM`, then `SIGKILL` if it stays |
| Stop a service and start it again | Service Control Manager (administrator) | `systemctl` (polkit for system units, `user:` prefix for user units) | `launchctl` user agents |
| Performance power plan | `powercfg` (Ultimate or High performance) | `powerprofilesctl` | not available |
| Purge cached memory (opt-in) | standby list (administrator) | `drop_caches` via polkit | not available |
| Keep the PC awake while quiet (opt-in) | `SetThreadExecutionState` | `systemd-inhibit` | `caffeinate` |
| Unload local AI models (opt-in) | Ollama's local API; LM Studio's `lms` tool | same | same |

The desktop shell, compositor, input, audio, security software, the terminal
you are typing in and CompuQuiet itself are always protected and cannot be
added as targets. So are the essential services (sound, the network, the
firewall, sign-in and the desktop session): saving one is refused, and one
that an older version saved is skipped with the reason shown. A service that
a running service needs is left running too, and named. Your own "never
touch" list sits on top of that.

A program with sound running, a call or a song, is left alone and listed as
such with the reason, in the preview and in the run: a helper that plays the
sound counts for the program around it. Windows reads the audio sessions of
every sound device, Linux asks `pactl` (PulseAudio and PipeWire), and macOS
cannot tell, so nothing is spared there. A run that could not tell says so.

*Slow down* is for a program that misbehaves when frozen (a sync client, a
job that must keep running): it keeps running at the lowest priority, in
Efficiency mode where Windows has it, and Restore puts back the priority and
mode it had. It changes nothing that the program or you set differently in the
meantime. Lowering a priority cannot be undone without administrator rights
on Linux and macOS, so a program set to it there is left running as it is
unless CompuQuiet runs as root.

## Using it

1. Open CompuQuiet. Home shows live CPU, memory and process figures, the
   graphics card's memory (NVIDIA on Windows and Linux, AMD on Linux; it says
   so where it cannot be read), and what one press will do. *Preview what one press would do right now* looks at
   the running machine and lists each program and service it would park, the
   command line a closed program reopens with, and what it would leave alone
   and why. It only looks: a press plans again from the machine as it is.
2. Open **Scan** to see what is using resources right now: recognised
   background software that is not yet on the park list, large programs with no
   window (where the system can say: Windows, macOS and X11 Linux with `xprop`,
   not Wayland), stoppable services that are running, a non-performance power plan
   and (not on Linux, which gives its cache up on demand) a large file cache. Each row shows its cost, why it is safe and a risk
   level. Low risk starts ticked; medium risk stays off until you tick it.
   *Add and free up this PC* adds the ticked finds and parks them. On a
   row, *Never touch* keeps that program or service off every list for good,
   and *Close instead* adds a program to be closed rather than suspended
   (it asks first for a browser, launcher or Office, which can hold unsaved
   work).
3. Review **Park list**: the built-in list of background hogs for your platform,
   with a running/not-running indicator. Add any running program by name,
   choose Suspend, Slow down or Close & relaunch, add services (typed, or picked from
   the services the machine has, minus the essential ones), find a row with
   the filter box, and save. Removing a row
   adds it to *Never touch*, so a scan does not bring it back; take it off
   that list to allow that.
4. Press the big button. With *Also park low-risk finds from a quick scan* on
   (the default), the run also parks those finds without changing your saved
   park list. The activity log shows every step and anything left alone (with
   the reason). The tray icon turns amber while Quiet Mode is on. Home then
   shows what CompuQuiet measured around the run: memory available and CPU
   before and after, and how much the suspended programs still hold. Freezing
   a program frees nothing; only closing one gives its memory back, and the
   change is approximate. On battery the performance power plan and the memory
   purge are skipped and listed as left alone, unless Settings allows them.
5. Press it again, or right-click the tray icon, to restore. Quitting while
   quiet can put everything back first.
6. **How long** on Home, beside the button, lets one press end by itself:
   after 1, 2 or 4 hours, or once a running program you pick has closed. Time
   is counted in the machine's uptime, so a clock change does not shift it. While
   quiet, Home shows the countdown and can add an hour or leave it on.
   Settings > *Ending Quiet Mode by itself* can also say so, once, when Quiet
   Mode has been on for a chosen number of hours with nothing to end it.
7. **Go quiet when one of these programs is running** (Settings > *Ending
   Quiet Mode by itself*, off until you turn it on) starts Quiet Mode ten
   seconds after one of a list of programs (a game, a local AI server) starts
   and puts everything back thirty seconds after the last has closed. Because
   you did not press the button it suspends instead of closing programs,
   leaves cached memory alone, never parks the programs on the list, and never
   ends or restarts a Quiet Mode you pressed for.
8. **Keep the PC awake** (Park list > System, off by default) holds off sleep
   and the screen turning off while Quiet Mode is on and lets go when it ends
   or CompuQuiet exits. A closed laptop lid still sleeps it, and on battery it
   is skipped unless Settings allows it.
9. **Unload local AI models** (Park list > System, off by default) asks Ollama
   and LM Studio to unload the models they hold in memory, which frees graphics
   memory for a game. A model loads again the next time something uses it, so
   there is nothing to put back, but a reply being written stops. The preview
   lists each model. Ollama is asked over its own local API, never over the
   network. LM Studio's `lms` tool is run only from `~/.lmstudio/bin`, only
   while LM Studio is running, and not when CompuQuiet has administrator rights
   (the tool sits in your user folder), which the preview says. A run that
   starts by itself unloads them too.
10. **Profiles** (top of the Park list) keep a separate park list and options
    for each use: one for a game, one for local AI, one for work. *Add* makes
    one from a copy of the profile in use or from the built-in list, and starts
    using it. Once there are two, Home has a *Profile* choice and the tray
    menu a *Profile* submenu. The profile in use is the one the big button
    runs and the Park list edits; it can change only while Quiet Mode is off.
    *Never touch* is shared by every profile, so removing a row from one list
    takes that program out of all of them: untick it to stop parking it in one
    profile only. In Settings each program that starts Quiet Mode by itself
    can start its own profile. The profiles are kept in `settings.json`, where
    1.1.7 reads only the one in use: going back to it forgets the others, so
    copy the file first.

![The Scan tab](docs/scan.png)

**Windows and administrator rights.** Stopping services and purging memory
need an elevated process. CompuQuiet starts unelevated so it can run at
logon without a prompt; when a target needs elevation the dashboard offers
*Relaunch as administrator*. "Start when I sign in" registers a logon task.
Created from an elevated CompuQuiet that runs from Program Files (where the
installer puts it), the task starts it elevated without a prompt. From any
other folder, such as a portable copy, the task is made without
administrator rights, because a program running as you could otherwise
replace that copy and be started with administrator rights at every sign-in;
the setting says so.

**Start when I sign in** is available on all three platforms (logon task on
Windows, LaunchAgent on macOS, XDG autostart for the Linux AppImage). It
refuses to register a copy running from Downloads, a temporary folder or a
build directory. The entry always starts CompuQuiet in the tray, whatever
*Start hidden in the tray* says. If the entry was made with administrator
rights and this copy has none, Settings says so and leaves the switch alone.

**From the command line.** `CompuQuiet --quiet` switches Quiet Mode on,
`--restore` puts everything back and `--toggle` does whichever the machine is
not in now. Point a game launcher's before-launch command, a script or a
hotkey tool at them. `--profile NAME` beside `--quiet` or `--toggle` runs that
saved profile this once, and leaves the one in use as it is (`--profile=NAME`
also works); a name that is not saved exits with 2. Each does what one press would (your saved park list, the
same checks), shows no window, and is skipped when the machine is already as
asked. Commands are done in the order they arrive, each after any run that is
going has finished. What happened is said in the window if it is open, and in
a notification if that is on; a failure is always notified. With CompuQuiet
running, the command is handed to it and the program exits at once, before the run has finished (exit code 0; 1 if the running copy did
not take it within three seconds). With none running it starts one in the tray
and does it there. Anything else on the command line (`--hidden` aside, which
starts in the tray, and `--profile`) exits with 2 and does nothing, so a misspelt flag is not
taken for a command that ran. The command names no program or setting, so it
cannot do more than the window can. The Windows release has no console, so
nothing is printed; use `start /wait "" "C:\Program Files\CompuQuiet\CompuQuiet.exe" --quiet`
and read `%ERRORLEVEL%`, or `Start-Process -Wait -PassThru`, to see the code.

## Install

Download from the [GitHub Releases page](https://github.com/Swatto86/CompuQuiet/releases).
Every release ships an installer and a portable build per platform, built by
the `release` workflow from the tagged commit after the full gate passes, with
a `SHA256SUMS-*.txt` file per platform to check a download against:

| Platform | Installer | Portable |
| --- | --- | --- |
| Windows 10/11 x64 | `CompuQuiet_<version>_x64-setup.exe` (NSIS, all users, into Program Files) | `CompuQuiet-portable-windows-x64.exe` |
| Linux x64 | `CompuQuiet_<version>_amd64.deb` | `CompuQuiet-portable-linux-x64` / `.AppImage` |
| macOS (Apple silicon) | `CompuQuiet_<version>_aarch64.dmg` | `CompuQuiet-portable-macos-arm64.app.tar.gz` |

The installers and portable builds are not code-signed with a publisher
certificate, and the macOS app is not notarised, so expect a SmartScreen or
Gatekeeper warning the first time. What is signed is the update: the app
installs a release only if it was signed with CompuQuiet's own update key.

Portable builds keep their settings and undo journal in the normal per-user
configuration folder unless `COMPUQUIET_DATA_DIR` points somewhere else,
for example a folder beside the executable on a USB stick.

The Windows installer asks for administrator permission once, because it
installs for all users. Releases up to 1.1.7 installed per user, into
`%LOCALAPPDATA%\CompuQuiet`; the new installer removes that copy and its
shortcuts, keeps your settings and undo journal (they live elsewhere), and
points an existing sign-in task at the new copy. Uninstalling removes the
sign-in task too. A portable copy is untouched: switch "Start when I sign
in" off before deleting one.

The Windows installer, the Linux AppImage and the macOS app check GitHub for
a newer signed release when Quiet Mode is off (at launch, then every few
hours while CompuQuiet stays running), download it, and install it and
restart once nothing is running and the window is closed to the tray. On
Windows an unelevated copy in Program Files shows the permission prompt for
that install (a notification says so first); a copy running as administrator
installs without one. The `.deb` and the portable copies do not update
themselves; get those from the releases page. Settings > About shows where
that stands, and *Check now* asks again. *Install updates automatically* in
Settings can be turned off: CompuQuiet then still looks now and then and says
under About that a release is out, but downloads and installs nothing until
you turn it back on. A copy built before
this check existed has to be replaced by hand once; after that, later
releases install on their own.

Required runtimes (shared platform components, not bundled):

- **Windows:** the [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/), present on Windows 11 and updated Windows 10.
- **Linux:** `libwebkit2gtk-4.1` and GTK 3 (the `.deb` declares them; the AppImage expects them installed). `powerprofilesctl` (with the power-profiles daemon running and offering a performance profile) and `pkexec` are optional and enable the power and memory actions; `xprop` (x11-utils) lets an X11 desktop tell which programs have a window.
- **macOS:** nothing beyond the system itself. The release is built on
  GitHub's current macOS runner for Apple silicon; no minimum version is set
  or tested.

State lives in `%APPDATA%\CompuQuiet` (Windows), `~/.config/CompuQuiet`
(Linux) or `~/Library/Application Support/CompuQuiet` (macOS):
`settings.json` and, while Quiet Mode is on, `journal.json`. An
`instance.lock` file in the same folder keeps a second copy from running
beside the first, and `compuquiet.log` records warnings, errors and failed
steps (a full one becomes `compuquiet.log.1`). A file CompuQuiet could not read
is moved aside to the same name with `.bad` added, never deleted or overwritten.

**Copy diagnostics** (Settings > About) puts a report on the clipboard for a
bug report: the version, the system, whether it runs as administrator, what
Quiet Mode has parked (by name, never a command line), what could not be put
back, and the last lines of the log. Your home folder is shown as `~`. It goes
to the clipboard for you to read before you paste it, and nowhere else.

## Building from source

Prerequisites: Rust (pinned in `rust-toolchain.toml`), Node 22+, and the
Tauri platform prerequisites for your OS (MSVC Build Tools on Windows;
`libwebkit2gtk-4.1-dev` and friends on Linux; Xcode command line tools on macOS).

```bash
npm ci
npx tauri dev                 # run with hot reload
pwsh scripts/fastcheck.ps1    # or scripts/fastcheck.sh: fmt, clippy, tsc
pwsh scripts/verify.ps1       # or scripts/verify.sh: the full gate
```

The full gate runs formatting, clippy, Rust and frontend tests, a debug build
with an in-memory fake platform, and a WebdriverIO suite that drives the real
binary through its real webview (Windows and Linux; `scripts/setup-e2e.ps1`
fetches the matching Edge WebDriver on Windows, and `scripts/setup-tauri-driver.sh`
the pinned `tauri-driver`; Linux also needs `webkit2gtk-driver`, and `xvfb`
without a display).
Packaged installers are built by `npx tauri build`, which is the release step
rather than the inner loop; it signs the update bundles, so it needs the
`TAURI_SIGNING_PRIVATE_KEY` environment variable (or
`--config '{"bundle":{"createUpdaterArtifacts":false}}'` for an unsigned
local build).

## Licence

MIT. See `LICENSE`.
