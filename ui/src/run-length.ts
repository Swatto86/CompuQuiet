/**
 * Home's "how long": what the next press asks for, and, while Quiet Mode is
 * on, how it ends by itself and the way to change that.
 */
import {
  api,
  errorMessage,
  type EndingState,
  type EngineState,
  type Until,
} from "./bridge.ts";
import { toast } from "./dialog.ts";
import { byId } from "./dom.ts";
import {
  asked,
  endingLine,
  extendedMinutes,
  secondsLeft,
  type Asked,
} from "./ending-text.ts";
import { fillRunning } from "./programs.ts";

export class RunLength {
  private readonly choice = byId<HTMLSelectElement>("until-choice");
  private readonly program = byId<HTMLInputElement>("until-program");
  private ending: EndingState | null = null;
  /** When the engine last reported the ending, on this page's own clock. */
  private reportedAt = 0;
  private engineBusy = false;
  private pending = false;

  constructor(private readonly changed: (state: EngineState) => void) {
    this.choice.addEventListener("change", () => {
      this.showProgram();
      if (this.choice.value === "program")
        void fillRunning(byId<HTMLDataListElement>("until-programs"));
    });
    byId("until-set").addEventListener("click", () => void this.set());
    byId("until-extend").addEventListener("click", () => void this.extend());
    byId("until-clear").addEventListener("click", () => void this.apply(null));
  }

  /** What a press of the big button asks for. */
  request(): Asked {
    return asked(this.choice.value, this.program.value);
  }

  /** Every run is opt-in: the next one starts as "until I put it back". */
  reset(): void {
    this.choice.value = "none";
    this.program.value = "";
    this.showProgram();
  }

  render(state: EngineState): void {
    this.ending = state.ending;
    this.engineBusy = state.busy;
    this.reportedAt = performance.now();
    const ends = state.quiet && state.ending !== null;
    byId("until-choose").hidden = ends;
    byId("until-now").hidden = !ends;
    byId("until-set").hidden = !state.quiet;
    byId("until-label").textContent = state.quiet
      ? "End it, from now"
      : "How long";
    byId("until-extend").hidden = state.ending?.kind !== "timer";
    this.showProgram();
    this.tick();
    this.enable();
  }

  /** Refresh the countdown; the engine counts, this carries it between reports. */
  tick(): void {
    if (this.ending) byId("until-text").textContent = this.line(this.ending);
  }

  private left(ending: EndingState): number | null {
    return secondsLeft(ending, (performance.now() - this.reportedAt) / 1000);
  }

  private line(ending: EndingState): string {
    return endingLine(ending, this.left(ending));
  }

  private showProgram(): void {
    this.program.hidden = this.choice.value !== "program";
  }

  private enable(): void {
    const off = this.engineBusy || this.pending;
    for (const id of [
      "until-choice",
      "until-set",
      "until-extend",
      "until-clear",
    ])
      byId<HTMLButtonElement>(id).disabled = off;
    this.program.disabled = off;
  }

  private async set(): Promise<void> {
    const request = this.request();
    if (!request.ok) {
      toast(request.reason, true);
      return;
    }
    if (await this.apply(request.until)) this.reset();
  }

  private async extend(): Promise<void> {
    const left = this.ending ? this.left(this.ending) : null;
    if (left !== null)
      await this.apply({ kind: "minutes", minutes: extendedMinutes(left) });
  }

  /** Ask the engine to change how the run ends; whether it did. */
  private async apply(until: Until | null): Promise<boolean> {
    this.pending = true;
    this.enable();
    try {
      this.changed(await api.setEnding(until));
      return true;
    } catch (error) {
      toast(errorMessage(error), true);
      return false;
    } finally {
      this.pending = false;
      this.enable();
    }
  }
}
