/** The banner under the tabs: settings that cannot be read, or administrator rights wanted. */
import {
  api,
  errorMessage,
  type EngineState,
  type Settings,
} from "./bridge.ts";
import { toast } from "./dialog.ts";
import { byId } from "./dom.ts";

export interface BannerHost {
  engine(): EngineState;
  settings(): Settings;
  /** Ask the engine where it stands again and draw everything from that. */
  reload(): Promise<void>;
}

export class Banner {
  constructor(private readonly host: BannerHost) {
    byId("banner-action").addEventListener("click", () => {
      if (host.engine().settings_unreadable !== null)
        void this.setAsideSettings();
      else void this.relaunchElevated();
    });
    byId("banner-dismiss").addEventListener("click", () => {
      const banner = byId("banner");
      banner.dataset["dismissed"] = "1";
      banner.hidden = true;
    });
  }

  /** A run was refused for want of administrator rights: say so even if it was dismissed. */
  reshow(): void {
    delete byId("banner").dataset["dismissed"];
  }

  render(): void {
    const engine = this.host.engine();
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
    if (this.needsElevation()) {
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

  private needsElevation(): boolean {
    const engine = this.host.engine();
    const caps = engine.capabilities;
    if (caps.elevated || !caps.can_elevate) return false;
    // A journal recovered from an elevated session holds stopped services that
    // only an elevated copy can start again.
    if (engine.quiet) return engine.summary.services_stopped > 0;
    const profile = this.host.settings().profile;
    return profile.services.some((s) => s.enabled) || profile.purge_memory;
  }

  private async setAsideSettings(): Promise<void> {
    try {
      const kept = await api.setAsideSettings();
      await this.host.reload();
      toast(
        kept === null
          ? "The settings file was already gone. CompuQuiet is on its built-in settings."
          : `Kept the unreadable settings file as ${kept}. CompuQuiet is on its built-in settings.`,
      );
    } catch (error) {
      toast(errorMessage(error), true);
    }
  }

  private async relaunchElevated(): Promise<void> {
    try {
      await api.relaunchElevated();
    } catch (error) {
      toast(errorMessage(error), true);
    }
  }
}
