/**
 * The Home preview: what one press would do on this PC right now. It is a
 * look, fetched when opened and on request, and never kept for the run: a
 * press looks at the machine again.
 */
import { api, errorMessage, type Preview } from "./bridge.ts";
import { byId } from "./dom.ts";
import { fromScan, itemLine, leftAlone } from "./preview-text.ts";

function listItem(text: string, muted = false): HTMLLIElement {
  const item = document.createElement("li");
  item.textContent = text;
  if (muted) item.className = "muted";
  return item;
}

export class PreviewPanel {
  private readonly panel = byId<HTMLDetailsElement>("preview");
  private readonly status = byId("preview-status");
  private readonly steps = byId<HTMLUListElement>("preview-steps");
  private readonly left = byId<HTMLUListElement>("preview-left");
  private readonly scan = byId("preview-scan");
  /** Only the newest look is shown: an older one can finish after it. */
  private ticket = 0;

  constructor() {
    this.panel.addEventListener("toggle", () => {
      if (this.panel.open) void this.refresh();
    });
    byId("preview-refresh").addEventListener(
      "click",
      () => void this.refresh(),
    );
  }

  /** Shown only while a press could start a run. */
  setAvailable(available: boolean): void {
    this.panel.hidden = !available;
    if (!available) this.panel.open = false;
  }

  /** After something the preview reads may have changed. */
  refreshIfOpen(): void {
    if (this.panel.open && !this.panel.hidden) void this.refresh();
  }

  private async refresh(): Promise<void> {
    const ticket = ++this.ticket;
    this.status.textContent = "Looking at this PC…";
    try {
      const preview = await api.previewPlan();
      if (ticket === this.ticket) this.show(preview);
    } catch (error) {
      if (ticket !== this.ticket) return;
      this.steps.replaceChildren();
      this.left.replaceChildren();
      this.scan.hidden = true;
      this.status.textContent = errorMessage(error);
    }
  }

  private show(preview: Preview): void {
    const time = new Date(preview.taken_at * 1000).toLocaleTimeString();
    this.status.textContent = `As of ${time}. Looking changes nothing, and pressing the button looks again first.`;
    this.steps.replaceChildren(
      ...(preview.items.length === 0
        ? [listItem("Nothing would be parked right now.", true)]
        : preview.items.map((item) => {
            const line = itemLine(item);
            const element = listItem(line.text);
            if (line.command !== null) {
              const command = document.createElement("code");
              command.textContent = line.command;
              element.append(" ", command);
            }
            return element;
          })),
    );
    const left = leftAlone(preview.skipped);
    this.left.replaceChildren(
      ...(left.length === 0
        ? [listItem("Nothing on your lists is being left alone.", true)]
        : left.map((line) => listItem(line))),
    );
    const found = fromScan(preview);
    this.scan.hidden = found === null;
    this.scan.textContent = found ?? "";
  }
}
