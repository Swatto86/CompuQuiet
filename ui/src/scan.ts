/** The Scan view: what could be parked right now, and one click to do it. */
import {
  api,
  errorMessage,
  type Recommendation,
  type ScanReport,
  type Settings,
} from "./bridge.ts";
import { showDialog, toast } from "./dialog.ts";
import { formatBytes, formatCoreShare } from "./format.ts";
import {
  carryClosing,
  carrySelection,
  chosenKind,
  kindLabel,
  riskyCloses,
  rowKey,
  selectedItems,
  summarize,
  tickLabel,
  withChoices,
} from "./scan-select.ts";

function byId<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing #${id}`);
  return element as T;
}

export interface ScanHost {
  /** The new settings after finds were added to the park list. */
  onSettings(settings: Settings): void;
  /** Save a program or service under Never touch, as the Park list does. */
  neverTouch(name: string): Promise<void>;
  /** Switch Quiet Mode on now (the dashboard's toggle). */
  goQuiet(): Promise<void>;
}

export class Scan {
  private report: ScanReport | null = null;
  private selected = new Set<string>();
  /** Programs the user marked to be closed rather than suspended. */
  private closing = new Set<string>();
  private busy = false;
  private quiet = false;

  constructor(private readonly host: ScanHost) {
    byId("scan-run").addEventListener("click", () => void this.refresh());
    byId("scan-apply").addEventListener("click", () => void this.apply(false));
    byId("scan-apply-quiet").addEventListener(
      "click",
      () => void this.apply(true),
    );
    this.render();
  }

  /**
   * While Quiet Mode is on a find added now is only parked from the next run,
   * so the button that also goes quiet has nothing left to do.
   */
  setQuiet(quiet: boolean): void {
    this.quiet = quiet;
    this.updateActions();
  }

  async refresh(): Promise<void> {
    if (this.busy) return;
    this.setBusy(true, "Scanning…");
    try {
      await this.rescan();
    } catch (error) {
      toast(`Scan failed: ${errorMessage(error)}`, true);
    } finally {
      this.setBusy(false, "");
      this.render();
    }
  }

  /** A fresh report; the user's ticks stay with the finds they were made on. */
  private async rescan(): Promise<void> {
    const report = await api.scan();
    this.selected = carrySelection(
      this.report?.recommendations ?? [],
      this.selected,
      report.recommendations,
    );
    this.closing = carryClosing(this.closing, report.recommendations);
    this.report = report;
  }

  private async apply(thenQuiet: boolean): Promise<void> {
    if (!this.report || this.busy) return;
    const accepted = withChoices(
      selectedItems(this.report.recommendations, this.selected),
      this.closing,
    );
    if (accepted.length === 0) return;
    const risky = riskyCloses(accepted);
    if (risky.length > 0 && !(await this.confirmClose(risky))) return;
    this.setBusy(true, "Adding…");
    let added = false;
    try {
      const settings = await api.applyRecommendations(accepted);
      added = true;
      this.host.onSettings(settings);
      toast(
        this.quiet
          ? `${accepted.length} added to the park list. Quiet Mode is on, so they are parked from the next run.`
          : `${accepted.length} added to the park list`,
      );
      await this.rescan();
    } catch (error) {
      toast(errorMessage(error), true);
    } finally {
      this.setBusy(false, "");
      this.render();
    }
    // Only once the finds are saved: after a refusal the button has not done
    // what it promised, and going quiet on the old list would park programs
    // the user did not choose. A failed re-scan does not undo the add.
    if (thenQuiet && added) await this.host.goQuiet();
  }

  /** Closing a browser, a launcher or Office can lose what is open in it. */
  private async confirmClose(items: Recommendation[]): Promise<boolean> {
    const many = items.length > 1;
    const choice = await showDialog({
      title: "Close instead of suspend?",
      body: `${items.map((item) => item.name).join(", ")} will be closed when Quiet Mode runs, and opened again when you restore. Anything unsaved in ${many ? "them" : "it"} is lost; suspending would have kept it.`,
      buttons: [
        { label: "Add and close", value: "yes", primary: true },
        { label: "Cancel", value: "no" },
      ],
    });
    return choice === "yes";
  }

  /** Keep a find off every list for good; it then no longer turns up here. */
  private async neverTouch(item: Recommendation): Promise<void> {
    if (this.busy) return;
    this.setBusy(true, "Saving…");
    let saved = false;
    try {
      await this.host.neverTouch(item.name);
      saved = true;
      toast(
        this.quiet
          ? `${item.name} will never be touched. Quiet Mode is on, so that starts from the next run.`
          : `${item.name} will never be touched`,
      );
      await this.rescan();
    } catch (error) {
      toast(
        saved ? `Scan failed: ${errorMessage(error)}` : errorMessage(error),
        true,
      );
    } finally {
      this.setBusy(false, "");
      this.render();
    }
  }

  private setBusy(busy: boolean, status: string): void {
    this.busy = busy;
    byId<HTMLButtonElement>("scan-run").disabled = busy;
    byId("scan-status").textContent = status;
    this.updateActions();
  }

  /** The two Add buttons follow the ticks; a tick does not rebuild the rows. */
  private updateActions(): void {
    const items = this.report?.recommendations ?? [];
    const none = selectedItems(items, this.selected).length === 0 || this.busy;
    byId<HTMLButtonElement>("scan-apply").disabled = none;
    const andQuiet = byId<HTMLButtonElement>("scan-apply-quiet");
    andQuiet.disabled = none;
    andQuiet.hidden = this.quiet;
  }

  private render(): void {
    const rows = byId<HTMLTableSectionElement>("scan-rows");
    const summary = byId("scan-summary");
    const items = this.report?.recommendations ?? [];
    this.updateActions();

    if (!this.report) {
      summary.textContent = "Press Scan to see what could be parked right now.";
      rows.replaceChildren(this.emptyRow("No scan yet."));
      return;
    }
    const counts = summarize(items);
    const parts = [
      `${counts.selectable} new find${counts.selectable === 1 ? "" : "s"}`,
      `${counts.low} low risk`,
      `${counts.medium} medium risk`,
      `${counts.targeted} already on the park list`,
      `${formatBytes(counts.memoryBytes)} in programs not yet parked`,
    ];
    if (this.report.cached_bytes > 0)
      parts.push(`${formatBytes(this.report.cached_bytes)} cached`);
    if (!this.report.activity_known)
      parts.push("unknown programs are not guessed on this platform");
    summary.textContent = parts.join(" · ");

    if (items.length === 0) {
      rows.replaceChildren(
        this.emptyRow(
          "Nothing to add: your park list already covers what is running.",
        ),
      );
      return;
    }
    rows.replaceChildren(...items.map((item) => this.row(item)));
  }

  private row(item: Recommendation): HTMLTableRowElement {
    const key = rowKey(item);
    const row = document.createElement("tr");
    if (item.already_targeted) row.className = "targeted";
    const tick = document.createElement("td");
    const input = document.createElement("input");
    input.type = "checkbox";
    input.checked = this.selected.has(key) && !item.already_targeted;
    input.disabled = item.already_targeted;
    input.setAttribute("aria-label", tickLabel(item));
    input.addEventListener("change", () => {
      if (input.checked) this.selected.add(key);
      else this.selected.delete(key);
      this.updateActions();
    });
    tick.appendChild(input);

    const name = document.createElement("td");
    name.className = "name";
    name.textContent =
      item.instances > 1 ? `${item.name} ×${item.instances}` : item.name;
    const action = document.createElement("td");
    const shown = (): string =>
      item.already_targeted
        ? "already on the list"
        : kindLabel(chosenKind(item, this.closing));
    action.textContent = shown();
    const why = document.createElement("td");
    why.className = "why";
    why.textContent = item.reason;
    const risk = document.createElement("td");
    const chip = document.createElement("span");
    chip.className = `risk risk-${item.risk}`;
    chip.textContent = item.risk === "low" ? "low" : "medium";
    risk.appendChild(chip);
    const memory = document.createElement("td");
    memory.className = "num";
    memory.textContent =
      item.memory_bytes > 0 ? formatBytes(item.memory_bytes) : "—";
    const cpu = document.createElement("td");
    cpu.className = "num";
    cpu.textContent =
      item.kind.kind === "process" ? formatCoreShare(item.cpu_percent) : "—";
    const more = document.createElement("td");
    more.className = "row-actions";
    if (!item.already_targeted && item.kind.kind === "process") {
      const close = this.rowButton("Close instead", () => {
        if (this.closing.has(key)) this.closing.delete(key);
        else this.closing.add(key);
        const closes = this.closing.has(key);
        close.textContent = closes ? "Suspend instead" : "Close instead";
        close.setAttribute(
          "aria-label",
          `${closes ? "Suspend" : "Close"} ${item.name} instead`,
        );
        action.textContent = shown();
        input.setAttribute(
          "aria-label",
          tickLabel({ ...item, kind: chosenKind(item, this.closing) }),
        );
      });
      close.setAttribute("aria-label", `Close ${item.name} instead`);
      more.appendChild(close);
    }
    if (
      !item.already_targeted &&
      (item.kind.kind === "process" || item.kind.kind === "service")
    ) {
      const never = this.rowButton(
        "Never touch",
        () => void this.neverTouch(item),
      );
      never.setAttribute("aria-label", `Never touch ${item.name}`);
      more.appendChild(never);
    }
    row.append(tick, name, action, why, risk, memory, cpu, more);
    return row;
  }

  private rowButton(label: string, onClick: () => void): HTMLButtonElement {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "small ghost";
    button.textContent = label;
    button.addEventListener("click", onClick);
    return button;
  }

  private emptyRow(text: string): HTMLTableRowElement {
    const row = document.createElement("tr");
    const cell = document.createElement("td");
    cell.colSpan = 8;
    cell.className = "muted";
    cell.textContent = text;
    row.appendChild(cell);
    return row;
  }
}
