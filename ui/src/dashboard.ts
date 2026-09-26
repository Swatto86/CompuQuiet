/** The dashboard: the toggle, live gauges, and what the last run did. */
import type { EngineState, LogLine, SystemStats } from "./bridge.ts";
import {
  formatBytes,
  formatPercent,
  formatSince,
  summaryLines,
} from "./format.ts";

function byId<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing #${id}`);
  return element as T;
}

export class Dashboard {
  private readonly toggle = byId<HTMLButtonElement>("toggle");
  private readonly label = byId("power-label");
  private readonly title = byId("hero-title");
  private readonly sub = byId("hero-sub");
  private readonly since = byId("hero-since");
  private readonly pill = byId("status-pill");
  private readonly log = byId<HTMLOListElement>("log");
  private readonly summary = byId<HTMLUListElement>("summary");
  private readonly skipped = byId<HTMLUListElement>("skipped");
  private startedAt: number | null = null;

  constructor(onToggle: () => void) {
    this.toggle.addEventListener("click", onToggle);
  }

  render(state: EngineState): void {
    this.startedAt = state.started_at;
    this.toggle.setAttribute("aria-pressed", String(state.quiet));
    this.toggle.classList.toggle("busy", state.busy);
    this.label.textContent = state.busy
      ? "Working…"
      : state.quiet
        ? "Put everything back"
        : "Free up this PC";
    this.pill.textContent = state.busy
      ? "Working"
      : state.quiet
        ? "Quiet"
        : "Ready";
    this.pill.className = `pill ${state.busy ? "pill-busy" : state.quiet ? "pill-quiet" : "pill-idle"}`;

    if (state.quiet) {
      this.title.textContent = state.recovered
        ? "Quiet Mode (recovered)"
        : "Quiet Mode is on";
      this.sub.textContent = state.recovered
        ? "A previous session left changes in place. Restore puts everything back."
        : "Background work is parked. Press again when you are done.";
    } else {
      this.title.textContent = "Ready for a game or local AI";
      this.sub.textContent =
        "One press parks background apps and services. Press again to put everything back.";
    }
    this.tick();

    const lines = summaryLines(state.summary);
    this.fill(
      this.summary,
      lines.length ? lines : ["Nothing yet."],
      lines.length === 0,
    );
    const skipped = state.skipped.map(
      (entry) => `${entry.name} — ${entry.reason}`,
    );
    this.fill(
      this.skipped,
      skipped.length ? skipped : ["—"],
      skipped.length === 0,
    );

    this.log.replaceChildren();
    if (state.log.length === 0) {
      const empty = document.createElement("li");
      empty.className = "muted";
      empty.textContent = "No activity yet.";
      this.log.appendChild(empty);
    } else {
      for (const line of state.log) this.appendLog(line);
    }
  }

  setBusy(busy: boolean): void {
    this.toggle.classList.toggle("busy", busy);
    this.toggle.disabled = busy;
    if (busy) {
      this.label.textContent = "Working…";
      this.pill.textContent = "Working";
      this.pill.className = "pill pill-busy";
      this.log.replaceChildren();
    }
  }

  appendLog(line: LogLine): void {
    const first = this.log.firstElementChild;
    if (first?.classList.contains("muted")) first.remove();
    const item = document.createElement("li");
    item.classList.toggle("failed", !line.ok);
    const label = document.createElement("span");
    label.textContent = line.label;
    item.appendChild(label);
    if (line.detail) {
      const detail = document.createElement("span");
      detail.className = "detail";
      detail.textContent = `— ${line.detail}`;
      item.appendChild(detail);
    }
    this.log.appendChild(item);
    this.log.scrollTop = this.log.scrollHeight;
  }

  updateStats(stats: SystemStats, freedBytes: number | null): void {
    const memPercent =
      stats.memory_total > 0
        ? (stats.memory_used / stats.memory_total) * 100
        : 0;
    byId("cpu-fill").style.width = formatPercent(stats.cpu_percent);
    byId("cpu-value").textContent = formatPercent(stats.cpu_percent);
    byId("cpu-gauge").setAttribute(
      "aria-valuenow",
      String(Math.round(stats.cpu_percent)),
    );
    byId("mem-fill").style.width = formatPercent(memPercent);
    byId("mem-value").textContent =
      `${formatBytes(stats.memory_used)} of ${formatBytes(stats.memory_total)}`;
    byId("mem-gauge").setAttribute(
      "aria-valuenow",
      String(Math.round(memPercent)),
    );
    byId("proc-value").textContent = String(stats.process_count);
    byId("freed-value").textContent =
      freedBytes === null ? "—" : formatBytes(freedBytes);
    this.tick();
  }

  /** Refresh the "on for 12 min" line. */
  tick(): void {
    if (this.startedAt === null) {
      this.since.hidden = true;
      return;
    }
    this.since.hidden = false;
    this.since.textContent = `Quiet for ${formatSince(this.startedAt, Date.now() / 1000)}`;
  }

  private fill(list: HTMLUListElement, lines: string[], muted: boolean): void {
    list.replaceChildren(
      ...lines.map((text) => {
        const item = document.createElement("li");
        item.textContent = text;
        if (muted) item.className = "muted";
        return item;
      }),
    );
  }
}
