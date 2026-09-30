/** How a run ends by itself, in words, and what the page may ask for. Pure. */
import type { EndingState, Until } from "./bridge.ts";
import { formatSince } from "./format.ts";

/** The longest timed run the engine accepts, in minutes. */
export const MAX_MINUTES = 24 * 60;

/** What the page asks for, or the reason it cannot be asked yet. */
export type Asked =
  { ok: true; until: Until | null } | { ok: false; reason: string };

/**
 * The request the choice and the program box make. `none` is a run that ends
 * only when the user says so; a number is minutes; `program` needs a name.
 */
export function asked(choice: string, program: string): Asked {
  if (choice === "none") return { ok: true, until: null };
  if (choice === "program") {
    const name = program.trim();
    return name === ""
      ? { ok: false, reason: "Choose a program to wait for." }
      : { ok: true, until: { kind: "program_exits", name } };
  }
  const minutes = Number(choice);
  return Number.isInteger(minutes) && minutes >= 1 && minutes <= MAX_MINUTES
    ? { ok: true, until: { kind: "minutes", minutes } }
    : { ok: false, reason: "Choose how long Quiet Mode should last." };
}

/**
 * Seconds a timer has left, counting on from when the engine last said. The
 * engine counts by the machine's uptime, so this only carries that figure
 * across the seconds between two of its reports.
 */
export function secondsLeft(
  ending: EndingState,
  waited: number,
): number | null {
  if (ending.kind !== "timer" || ending.seconds_left === null) return null;
  return Math.max(0, ending.seconds_left - Math.max(0, waited));
}

/** The sentence under the button while a run is going to end by itself. */
export function endingLine(ending: EndingState, left: number | null): string {
  switch (ending.kind) {
    case "timer":
      return left === null || left <= 0
        ? "Ending now."
        : `Ends by itself in ${formatSince(0, left)}.`;
    case "program":
      return `Ends by itself once ${ending.program ?? "the program"} has closed.`;
    case "trigger":
      return `Started because ${ending.program ?? "a program"} is running. Ends by itself once none of your auto-quiet programs is.`;
  }
}

/** Minutes from now that put a timer `extra` minutes later, within the limit. */
export function extendedMinutes(left: number, extra = 60): number {
  return Math.min(MAX_MINUTES, Math.ceil(left / 60) + extra);
}
