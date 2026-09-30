/**
 * The About card's "Copy diagnostics". The report is built and redacted in
 * Rust; this only puts it on the clipboard for the person to read and paste.
 * It goes nowhere else.
 */
import { api, errorMessage } from "./bridge.ts";
import { toast } from "./dialog.ts";
import { byId } from "./dom.ts";

export class Diagnostics {
  private readonly button = byId<HTMLButtonElement>("diagnostics-copy");
  /** Shown only when the clipboard refuses, so the person can copy by hand. */
  private readonly fallback = byId<HTMLTextAreaElement>("diagnostics-text");

  constructor() {
    this.button.addEventListener("click", () => void this.copy());
  }

  private async copy(): Promise<void> {
    this.button.disabled = true;
    try {
      const report = await api.diagnostics();
      try {
        await navigator.clipboard.writeText(report);
        this.fallback.hidden = true;
        toast("Diagnostics copied. Read them before you share them.");
      } catch {
        this.fallback.value = report;
        this.fallback.hidden = false;
        this.fallback.focus();
        this.fallback.select();
        toast(
          "The clipboard was not available. The report is shown below to copy by hand.",
          true,
        );
      }
    } catch (error) {
      toast(errorMessage(error), true);
    } finally {
      this.button.disabled = false;
    }
  }
}
