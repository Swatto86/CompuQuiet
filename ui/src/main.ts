/** Boot, wiring and the flows that span views: toggle, quit, elevation. */
import {
  api,
  errorMessage,
  isAppError,
  onConfirmQuit,
  onProgress,
  onState,
  type AppInfo,
  type EngineState,
  type Settings,
} from "./bridge.ts";
import { Dashboard } from "./dashboard.ts";
import { showDialog, toast } from "./dialog.ts";
import { homePlan } from "./format.ts";
import { Scan } from "./scan.ts";
import { SettingsView } from "./settings-view.ts";
import { Targets } from "./targets.ts";
import { applyTheme } from "./theme.ts";
import { invoke } from "@tauri-apps/api/core";

function byId<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing #${id}`);
  return element as T;
}

let engine: EngineState;
let settings: Settings;
let info: AppInfo;
let memoryBaseline: number | null = null;
let busy = false;
/** A run started outside this window (the tray, or at startup) is going. */
let runElsewhere = false;

const dashboard = new Dashboard(() => void toggle());
let targets: Targets;
let settingsView: SettingsView;
let scanView: Scan;

async function boot(): Promise<void> {
  [settings, engine, info] = await Promise.all([
    api.getSettings(),
    api.getState(),
    api.appInfo(),
  ]);
  applyTheme(settings.theme);

  targets = new Targets(settings.profile, {
    save: async (profile) => saveSettings({ ...settings, profile }),
    defaults: async () => (await api.defaultSettings()).profile,
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
    goQuiet: async () => {
      if (!engine.quiet) await toggle();
    },
  });

  renderAll();
  wireTabs();
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
  await onConfirmQuit(() => void quitFlow());

  if (engine.startup_error) toast(engine.startup_error, true);
  else if (engine.recovered)
    toast("Recovered a previous Quiet session. Restore puts everything back.");

  await api.frontendReady();
  void pollStats();
  window.setInterval(() => void pollStats(), 2000);
  window.setInterval(() => dashboard.tick(), 15_000);
}

function renderAll(): void {
  dashboard.render(engine);
  targets.describe(engine.capabilities, engine.os);
  settingsView.render(settings, info);
  renderPlan();
  renderAbout();
  renderBanner();
}

function renderPlan(): void {
  byId("hero-plan").textContent = homePlan(engine.quiet, settings.profile);
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

function wireTabs(): void {
  const tabs = [...document.querySelectorAll<HTMLButtonElement>(".tab")];
  const show = (view: string) => {
    for (const tab of tabs)
      tab.setAttribute("aria-selected", String(tab.dataset["view"] === view));
    for (const section of document.querySelectorAll<HTMLElement>(".view")) {
      section.hidden = section.id !== `view-${view}`;
    }
    if (view === "targets") void targets.refreshRunning();
    if (view === "settings") void settingsView.refreshAutostart();
    if (view === "scan") void scanView.refresh();
  };
  for (const tab of tabs) {
    tab.addEventListener("click", () =>
      show(tab.dataset["view"] ?? "dashboard"),
    );
  }
  for (const jump of document.querySelectorAll<HTMLButtonElement>(
    "[data-jump]",
  )) {
    jump.addEventListener("click", () =>
      show(jump.dataset["jump"] ?? "dashboard"),
    );
  }
  document.addEventListener("keydown", (event) => {
    if (!event.ctrlKey || event.key < "1" || event.key > "4") return;
    const tab = tabs[Number(event.key) - 1];
    if (tab) {
      event.preventDefault();
      show(tab.dataset["view"] ?? "dashboard");
      tab.focus();
    }
  });
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

async function pollStats(): Promise<void> {
  if (document.hidden) return;
  try {
    const stats = await api.getStats();
    const freed =
      engine.quiet && memoryBaseline !== null
        ? Math.max(0, memoryBaseline - stats.memory_used)
        : null;
    dashboard.updateStats(stats, freed);
    // Not while a run is freeing memory, or the figure would shrink.
    if (!engine.quiet && !busy && !runElsewhere)
      memoryBaseline = stats.memory_used;
  } catch {
    // The next poll will report; a missed sample is not worth a toast.
  }
}

async function toggle(): Promise<void> {
  if (busy) return;
  busy = true;
  dashboard.setBusy(true);
  try {
    engine = engine.quiet ? await api.restore() : await api.goQuiet();
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
