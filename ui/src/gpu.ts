/**
 * The Home screen's graphics-memory gauge: one bar per adapter, or the reason
 * there is none. Read slowly, since a driver's tool answers, and only while
 * the window is showing.
 */
import { api, type GpuReading } from "./bridge.ts";
import { byId } from "./dom.ts";
import { gpuFill } from "./format.ts";

const EVERY_MS = 5000;

interface Bar {
  gauge: HTMLElement;
  fill: HTMLElement;
  value: HTMLElement;
}

export class GpuGauge {
  private readonly list = byId("gpu-list");
  private bars: Bar[] = [];
  /** The adapters the bars were built for, so an update keeps their transition. */
  private built = "";
  private asking = false;

  /** `showing` says whether the window is on screen; a hidden one asks nothing. */
  constructor(private readonly showing: () => Promise<boolean>) {}

  start(): void {
    void this.poll();
    window.setInterval(() => void this.poll(), EVERY_MS);
  }

  private async poll(): Promise<void> {
    // The tool may outlast the interval; never two at once.
    if (this.asking || !(await this.showing())) return;
    this.asking = true;
    try {
      this.render(await api.getGpu());
    } catch {
      // The next poll will report; a missed sample is not worth a toast.
    } finally {
      this.asking = false;
    }
  }

  private render(reading: GpuReading): void {
    if (reading.adapters.length === 0) {
      const note = document.createElement("span");
      note.className = "muted";
      note.textContent = reading.unavailable ?? "Not available";
      this.list.replaceChildren(note);
      this.built = "";
      this.bars = [];
      return;
    }
    const names = JSON.stringify(reading.adapters.map((card) => card.name));
    if (names !== this.built) {
      this.build(reading.adapters.map((card) => card.name));
      this.built = names;
    }
    reading.adapters.forEach((card, index) => {
      const bar = this.bars[index];
      if (!bar) return;
      const { percent, text } = gpuFill(card);
      bar.fill.style.width = `${Math.round(percent)}%`;
      bar.gauge.setAttribute("aria-valuenow", String(Math.round(percent)));
      bar.gauge.setAttribute("aria-valuetext", text);
      bar.value.textContent = text;
    });
  }

  private build(names: string[]): void {
    const rows = names.map((card) => {
      const row = document.createElement("div");
      row.className = "gpu-row";
      const name = document.createElement("span");
      name.className = "gpu-name";
      name.textContent = card;
      name.title = card;
      const gauge = document.createElement("div");
      gauge.className = "gauge";
      gauge.setAttribute("role", "meter");
      gauge.setAttribute("aria-label", `${card} memory`);
      gauge.setAttribute("aria-valuemin", "0");
      gauge.setAttribute("aria-valuemax", "100");
      const fill = document.createElement("div");
      fill.className = "gauge-fill";
      gauge.appendChild(fill);
      const value = document.createElement("span");
      value.className = "stat-value";
      row.append(name, gauge, value);
      return { row, bar: { gauge, fill, value } };
    });
    this.list.replaceChildren(...rows.map(({ row }) => row));
    this.bars = rows.map(({ bar }) => bar);
  }
}
