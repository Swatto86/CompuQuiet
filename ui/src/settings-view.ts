/** Behaviour preferences and the quit button. */
import {
  api,
  errorMessage,
  type AppInfo,
  type Settings,
  type Theme,
} from "./bridge.ts";
import { toast } from "./dialog.ts";
import { byId } from "./dom.ts";
import { applyTheme } from "./theme.ts";

export interface SettingsHost {
  current(): Settings;
  save(settings: Settings): Promise<void>;
  quit(): void;
}

export class SettingsView {
  constructor(private readonly host: SettingsHost) {
    this.bind("set-hidden", (s, on) => ({ ...s, start_hidden: on }));
    this.bind("set-close-tray", (s, on) => ({ ...s, close_to_tray: on }));
    this.bind("set-notify", (s, on) => ({ ...s, notifications: on }));
    this.bind("set-restore-quit", (s, on) => ({ ...s, restore_on_quit: on }));
    this.bind("set-auto-scan", (s, on) => ({ ...s, auto_scan: on }));
    this.bind("set-battery", (s, on) => ({ ...s, allow_on_battery: on }));
    this.bind("set-auto-update", (s, on) => ({ ...s, auto_update: on }));
    byId<HTMLSelectElement>("set-theme").addEventListener("change", (event) => {
      const theme = (event.target as HTMLSelectElement).value as Theme;
      applyTheme(theme);
      void this.persist({ ...this.host.current(), theme });
    });
    byId<HTMLInputElement>("set-autostart").addEventListener(
      "change",
      (event) => {
        const input = event.target as HTMLInputElement;
        void this.setAutostart(input.checked);
      },
    );
    byId("quit").addEventListener("click", () => this.host.quit());
  }

  render(settings: Settings, info: AppInfo): void {
    byId<HTMLInputElement>("set-hidden").checked = settings.start_hidden;
    byId<HTMLInputElement>("set-close-tray").checked = settings.close_to_tray;
    byId<HTMLInputElement>("set-notify").checked = settings.notifications;
    byId<HTMLInputElement>("set-restore-quit").checked =
      settings.restore_on_quit;
    byId<HTMLInputElement>("set-auto-scan").checked = settings.auto_scan;
    byId<HTMLInputElement>("set-battery").checked = settings.allow_on_battery;
    byId<HTMLInputElement>("set-auto-update").checked = settings.auto_update;
    byId<HTMLSelectElement>("set-theme").value = settings.theme;
    byId("data-dir").textContent = info.data_dir;
  }

  /**
   * Asks the operating system, which can take a second, so it runs when the
   * Settings tab opens and after a change, not on every state event.
   */
  async refreshAutostart(): Promise<void> {
    const input = byId<HTMLInputElement>("set-autostart");
    const note = byId("autostart-note");
    try {
      const status = await api.getAutostart();
      const because = status.limited_because
        ? `: ${status.limited_because}`
        : "";
      input.checked = status.enabled;
      // Refused for this copy: it may still be switched off. Locked: not at all.
      input.disabled =
        status.locked !== null || (!status.allowed && !status.enabled);
      note.textContent = status.locked
        ? `(${status.locked})`
        : status.reason
          ? `(unavailable: ${status.reason})`
          : status.enabled && status.elevated
            ? "(starts with administrator rights)"
            : status.enabled
              ? `(starts without administrator rights${because})`
              : status.limited_because
                ? `(will start without administrator rights${because})`
                : "";
    } catch (error) {
      note.textContent = `(${errorMessage(error)})`;
      input.disabled = false;
    }
  }

  private async setAutostart(enabled: boolean): Promise<void> {
    // Until the answer is back, a second click would race the first.
    byId<HTMLInputElement>("set-autostart").disabled = true;
    try {
      await api.setAutostart(enabled);
      toast(
        enabled ? "CompuQuiet will start with the system" : "Autostart removed",
      );
    } catch (error) {
      toast(errorMessage(error), true);
    }
    await this.refreshAutostart();
  }

  private bind(
    id: string,
    change: (settings: Settings, on: boolean) => Settings,
  ): void {
    byId<HTMLInputElement>(id).addEventListener("change", (event) => {
      const on = (event.target as HTMLInputElement).checked;
      void this.persist(change(this.host.current(), on));
    });
  }

  private async persist(settings: Settings): Promise<void> {
    try {
      await this.host.save(settings);
    } catch (error) {
      toast(errorMessage(error), true);
    }
  }
}
