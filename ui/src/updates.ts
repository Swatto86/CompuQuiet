/** The About card's update line and its manual "Check now". */
import {
  api,
  errorMessage,
  onUpdateStatus,
  type UpdateStatus,
} from "./bridge.ts";
import { toast } from "./dialog.ts";
import { byId } from "./dom.ts";
import { canCheckForUpdates, updateLine } from "./format.ts";

export class Updates {
  private readonly line = byId("about-update");
  private readonly button = byId<HTMLButtonElement>("update-check");

  constructor() {
    this.button.addEventListener("click", () => void this.check());
  }

  async start(): Promise<void> {
    // Listen first, so a change between the read and the listener is not lost.
    await onUpdateStatus((status) => this.render(status));
    this.render(await api.updateStatus());
  }

  private render(status: UpdateStatus): void {
    this.line.textContent = updateLine(status);
    this.button.disabled = !canCheckForUpdates(status);
  }

  private async check(): Promise<void> {
    this.button.disabled = true;
    try {
      this.render(await api.checkForUpdates());
    } catch (error) {
      toast(errorMessage(error), true);
      this.button.disabled = false;
    }
  }
}
