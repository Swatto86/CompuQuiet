/** Boot, wiring and the flows that span views: toggle, quit, elevation. */
import {
  api,
  errorMessage,
  isAppError,
  onConfirmQuit,
  onNotice,
  onProgress,
  onRunError,
  onState,
  type AppInfo,
  type EngineState,
  type Settings,
} from "./bridge.ts";
import { Dashboard } from "./dashboard.ts";
import { Diagnostics } from "./diagnostics.ts";
import { showDialog, toast } from "./dialog.ts";
import { byId } from "./dom.ts";
import { closeHint, homePlan } from "./format.ts";
import { addKeepAlive } from "./profile-edit.ts";
import { PreviewPanel } from "./preview.ts";
import { Recovery } from "./recovery.ts";
import { RunLength } from "./run-length.ts";
import { Scan } from "./scan.ts";
import { SettingsView } from "./settings-view.ts";
import { wireTabs, type Tabs } from "./tabs.ts";
import { Targets } from "./targets.ts";
import { applyTheme } from "./theme.ts";
import { Updates } from "./updates.ts";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

let engine: EngineState;
let settings: Settings;
let info: AppInfo;
let busy = false;
/** The Park list holds edits that are not saved, so Home does not count them. */
let unsaved = false;
/** A run started outside this window (the tray, or at startup) is going. */
let runElsewhere = false;

const dashboard = new Dashboard(() => void toggle());
const preview = new PreviewPanel();
const runLength = new RunLength((next) => {
  engine = next;
  renderAll();
});
const recovery = new Recovery((next) => {
  engine = next;
  renderAll();
});
let targets: Targets;
let settingsView: SettingsView;
let scanView: Scan;
let tabs: Tabs;

async function boot(): Promise<void> {
  [settings, engine, info] = await Promise.all([
    api.getSettings(),
    api.getState(),
    api.appInfo(),
  ]);
  applyTheme(settings.theme);

  tabs = wireTabs((view) => {
    if (view === "dashboard") preview.refreshIfOpen();
    if (view === "targets") void targets.refreshRunning();
    if (view === "settings") void settingsView.refreshAutostart();
    if (view === "scan") void scanView.refresh();
  });
  targets = new Targets(settings.profile, {
    save: async (profile) => saveSettings({ ...settings, profile }),
    defaults: async () => (await api.defaultSettings()).profile,
    unsaved: (now) => {
      if (now === unsaved) return;
      unsaved = now;
      tabs.setUnsaved("targets", now);
      renderPlan();
    },
  });
  settingsView = new SettingsView({
    current: () => settings,
    save: saveSettings,
    quit: () => void quitFlow(),
  });
  scanView = new Scan({
    onSettings: (next) => {
      settings = next;
      targets.setProfile(next.profile);
      renderAll();
    },
    neverTouch: async (name) => {
      const result = addKeepAlive(settings.profile, name);
      if (!result.ok) throw new Error(result.reason);
      await saveSettings({ ...settings, profile: result.profile });
      targets.setProfile(result.profile);
    },
    goQuiet: async () => {
      if (!engine.quiet) await toggle();
    },
  });

  renderAll();
  wireBanner();
  wireE2eHooks();

  await onProgress((line) => {
    if (!busy && !runElsewhere) {
      runElsewhere = true;
      dashboard.setBusy(true);
    }
    dashboard.appendLog(line);
  });
  await onState((state) => {
    engine = state;
    if (runElsewhere && !busy) {
      runElsewhere = false;
      dashboard.setBusy(false);
    }
    renderAll();
  });
  await onRunError((error) => toast(error.message, true));
  await onNotice((text) => toast(text));
  await onConfirmQuit(() => void quitFlow());
  new Diagnostics();
  new Updates().start().catch((error: unknown) => {
    toast(`Could not read the update status: ${errorMessage(error)}`, true);
  });
  // A run that ended between the first fetch and the listeners above would
  // otherwise leave the page on "Working…" until the next click.
  engine = await api.getState();
  renderAll();

  if (engine.startup_error) toast(engine.startup_error, true);
  else if (engine.recovered)
    toast("Recovered a previous Quiet session. Restore puts everything back.");

  await api.frontendReady();
  void pollStats();
  window.setInterval(() => void pollStats(), 2000);
  window.setInterval(() => {
    dashboard.tick();
    runLength.tick();
  }, 15_000);
}

function renderAll(): void {
  // Also puts the look back when a save that changed it was refused.
  applyTheme(settings.theme);
  scanView.setQuiet(engine.quiet);
  dashboard.render(engine);
  preview.setAvailable(!engine.quiet);
  recovery.render(engine);
  runLength.render(engine);
  targets.describe(engine.capabilities, engine.os);
  settingsView.render(settings, info);
  renderPlan();
  renderAbout();
  renderBanner();
}

function renderPlan(): void {
  byId("hero-plan").textContent = homePlan(engine.quiet, settings.profile, {
    autoScan: settings.auto_scan,
    unsaved,
  });
  byId("close-hint").textContent = closeHint(settings.close_to_tray);
}

function renderAbout(): void {
  byId("about-version").textContent = `v${info.version}`;
  byId("about-os").textContent = {
    windows: "Windows",
    linux: "Linux",
    mac_os: "macOS",
  }[info.os];
  byId("about-build").textContent = info.debug ? "development" : "release";
  byId("about-admin").textContent = engine.capabilities.elevated
    ? "yes"
    : engine.capabilities.can_elevate
      ? "no — services and the memory purge need it"
      : "not required on this platform";
}

function needsElevation(): boolean {
  const caps = engine.capabilities;
  if (caps.elevated || !caps.can_elevate) return false;
  // A journal recovered from an elevated session holds stopped services that
  // only an elevated copy can start again.
  if (engine.quiet) return engine.summary.services_stopped > 0;
  const profile = settings.profile;
  return profile.services.some((s) => s.enabled) || profile.purge_memory;
}

function renderBanner(): void {
  const banner = byId("banner");
  const text = byId("banner-text");
  const action = byId<HTMLButtonElement>("banner-action");
  const dismiss = byId<HTMLButtonElement>("banner-dismiss");
  // Not dismissible: nothing is saved and Quiet Mode is off until it is dealt with.
  dismiss.hidden = engine.settings_unreadable !== null;
  if (engine.settings_unreadable !== null) {
    text.textContent = `CompuQuiet could not read its settings file (${engine.settings_unreadable}), so it saves nothing and will not go quiet. Set the file aside to keep a copy as settings.json.bad and start fresh, or fix it and restart.`;
    action.textContent = "Set the file aside";
    action.hidden = false;
    banner.hidden = false;
    return;
  }
  if (banner.dataset["dismissed"] === "1") return;
  if (needsElevation()) {
    text.textContent = engine.quiet
      ? "Restoring the stopped services needs administrator rights; the elevated copy picks up this session."
      : "Stopping services and purging memory need administrator rights.";
    action.textContent = "Relaunch as administrator";
    action.hidden = false;
    banner.hidden = false;
  } else {
    banner.hidden = true;
  }
}

function wireBanner(): void {
  byId("banner-action").addEventListener("click", () => {
    if (engine.settings_unreadable !== null) void setAsideSettings();
    else void relaunchElevated();
  });
  byId("banner-dismiss").addEventListener("click", () => {
    const banner = byId("banner");
    banner.dataset["dismissed"] = "1";
    banner.hidden = true;
  });
}

/** Acceptance-suite only: same path as the tray Quit menu item. */
function wireE2eHooks(): void {
  const button = document.getElementById("e2e-tray-quit");
  if (!button) return;
  button.addEventListener("click", () => {
    void invoke("simulate_tray_menu", { id: "tray-quit" }).catch(
      (error: unknown) => {
        toast(errorMessage(error), true);
      },
    );
  });
}

async function setAsideSettings(): Promise<void> {
  try {
    const kept = await api.setAsideSettings();
    engine = await api.getState();
    renderAll();
    toast(
      kept === null
        ? "The settings file was already gone. CompuQuiet is on its built-in settings."
        : `Kept the unreadable settings file as ${kept}. CompuQuiet is on its built-in settings.`,
    );
  } catch (error) {
    toast(errorMessage(error), true);
  }
}

async function relaunchElevated(): Promise<void> {
  try {
    await api.relaunchElevated();
  } catch (error) {
    toast(errorMessage(error), true);
  }
}

async function saveSettings(next: Settings): Promise<void> {
  // Applied before the round trip, so a second quick change builds on this
  // one rather than on the settings before it; put back if the save fails.
  const previous = settings;
  settings = next;
  try {
    await api.saveSettings(next);
  } catch (error) {
    if (settings === next) settings = previous;
    renderAll();
    throw error;
  }
  renderBanner();
  renderPlan();
}

/**
 * Whether anyone can see the window. Hiding it to the tray does not make
 * `document.hidden` true in WebView2, so the window is asked as well;
 * without this the app samples the machine every two seconds all day.
 */
async function windowShowing(): Promise<boolean> {
  if (document.hidden) return false;
  try {
    return await getCurrentWindow().isVisible();
  } catch {
    // Cannot ask: carry on sampling, as before.
    return true;
  }
}

async function pollStats(): Promise<void> {
  if (!(await windowShowing())) return;
  try {
    dashboard.updateStats(await api.getStats());
  } catch {
    // The next poll will report; a missed sample is not worth a toast.
  }
}

async function toggle(): Promise<void> {
  if (busy) return;
  const until = engine.quiet ? null : runLength.request();
  if (until && !until.ok) {
    toast(until.reason, true);
    return;
  }
  busy = true;
  dashboard.setBusy(true);
  try {
    engine = engine.quiet
      ? await api.restore()
      : await api.goQuiet(until?.until ?? null);
    // Every timed run is asked for afresh.
    if (engine.quiet) runLength.reset();
    if (engine.quiet && engine.log.some((line) => !line.ok)) {
      toast("Some steps failed; see Activity.", true);
    }
  } catch (error) {
    toast(errorMessage(error), true);
    if (isAppError(error) && error.code === "needs_elevation") {
      const banner = byId("banner");
      delete banner.dataset["dismissed"];
    }
    engine = await api.getState();
  } finally {
    busy = false;
    dashboard.setBusy(false);
    renderAll();
  }
}

async function quitFlow(): Promise<void> {
  if (!engine.quiet) {
    try {
      await api.quit(false);
    } catch (error) {
      toast(errorMessage(error), true);
    }
    return;
  }
  const choice = await showDialog({
    title: "Quiet Mode is still on",
    body: "Restore the services, programs and power plan before quitting? Quitting without restoring leaves them parked; the journal is kept so you can restore next time.",
    buttons: [
      {
        label: "Restore and quit",
        value: "restore",
        primary: settings.restore_on_quit,
      },
      {
        label: "Quit without restoring",
        value: "quit",
        danger: true,
        primary: !settings.restore_on_quit,
      },
      { label: "Cancel", value: "cancel" },
    ],
    cancel: "cancel",
  });
  if (choice === "cancel") return;
  try {
    await api.quit(choice === "restore");
  } catch (error) {
    toast(errorMessage(error), true);
    engine = await api.getState();
    renderAll();
  }
}

void boot().catch((error: unknown) => {
  toast(`CompuQuiet could not start: ${errorMessage(error)}`, true);
  api.frontendReady().catch((reason: unknown) => {
    toast(
      `CompuQuiet could not show its window: ${errorMessage(reason)}`,
      true,
    );
  });
});
