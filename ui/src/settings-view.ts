/** Behaviour preferences and the quit button. */
import {
  api,
  errorMessage,
  type AppInfo,
  type Settings,
  type Theme,
} from "./bridge.ts";
import { toast } from "./dialog.ts";
import { applyTheme } from "./theme.ts";

function byId<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing #${id}`);
  return element as T;
}

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
    byId<HTMLSelectElement>("set-theme").value = settings.theme;
    byId("data-dir").textContent = info.data_dir;
    void this.refreshAutostart();
  }

  async refreshAutostart(): Promise<void> {
    const input = byId<HTMLInputElement>("set-autostart");
    const note = byId("autostart-note");
    try {
      const status = await api.getAutostart();
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
              ? "(starts without administrator rights)"
              : "";
    } catch (error) {
      note.textContent = `(${errorMessage(error)})`;
    }
  }

  private async setAutostart(enabled: boolean): Promise<void> {
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
