/** Pure helpers for the Scan view: what is ticked by default and what a row says. */
import type { Recommendation, RecommendationKind } from "./bridge.ts";

/** The same find on the next scan has the same key, whatever row it lands on. */
export function rowKey(item: Recommendation): string {
  return `${item.kind.kind}:${item.name}`;
}

/** Low-risk finds that are not already targets start ticked; the rest do not. */
export function defaultSelection(items: Recommendation[]): Set<string> {
  const selected = new Set<string>();
  for (const item of items) {
    if (item.risk === "low" && !item.already_targeted)
      selected.add(rowKey(item));
  }
  return selected;
}

/**
 * The ticks for a fresh scan: what the user chose for a find that was already
 * on offer stays as chosen, and anything new starts as it would by default.
 */
export function carrySelection(
  before: Recommendation[],
  chosen: Set<string>,
  after: Recommendation[],
): Set<string> {
  const offered = new Set(
    before.filter((item) => !item.already_targeted).map(rowKey),
  );
  const next = defaultSelection(after);
  for (const item of after) {
    const key = rowKey(item);
    if (item.already_targeted || !offered.has(key)) continue;
    if (chosen.has(key)) next.add(key);
    else next.delete(key);
  }
  return next;
}

/**
 * What a row will do if it is added: a program marked "close instead" is
 * closed (and opened again when Quiet Mode ends), every other find is as the
 * scan suggested it.
 */
export function chosenKind(
  item: Recommendation,
  closing: Set<string>,
): RecommendationKind {
  return item.kind.kind === "process" && closing.has(rowKey(item))
    ? { kind: "process", action: "close" }
    : item.kind;
}

export function withChoices(
  items: Recommendation[],
  closing: Set<string>,
): Recommendation[] {
  return items.map((item) => ({ ...item, kind: chosenKind(item, closing) }));
}

/** The marks that still apply on a fresh scan: a find that is gone or already on the list takes its mark with it. */
export function carryClosing(
  closing: Set<string>,
  after: Recommendation[],
): Set<string> {
  const open = new Set(
    after
      .filter((item) => item.kind.kind === "process" && !item.already_targeted)
      .map(rowKey),
  );
  return new Set([...closing].filter((key) => open.has(key)));
}

/**
 * Closes that could lose work. A medium-risk find is a browser, a launcher,
 * voice chat or Office: the programs that hold something unsaved.
 */
export function riskyCloses(items: Recommendation[]): Recommendation[] {
  return items.filter(
    (item) =>
      item.risk === "medium" &&
      item.kind.kind === "process" &&
      item.kind.action === "close",
  );
}

export function selectedItems(
  items: Recommendation[],
  selected: Set<string>,
): Recommendation[] {
  return items.filter(
    (item) => selected.has(rowKey(item)) && !item.already_targeted,
  );
}

export function kindLabel(kind: RecommendationKind): string {
  switch (kind.kind) {
    case "process":
      return kind.action === "close" ? "Close & relaunch" : "Suspend";
    case "service":
      return "Stop service";
    case "power_plan":
      return "Power plan";
    case "memory_purge":
      return "Purge cache";
  }
}

/** What a screen reader says for a row's tick box: the action and its object. */
export function tickLabel(item: Recommendation): string {
  switch (item.kind.kind) {
    case "process":
      return `${kindLabel(item.kind)} ${item.name}`;
    case "service":
      return `Stop service ${item.name}`;
    default:
      return `Add ${item.name}`;
  }
}

export interface ScanSummary {
  total: number;
  selectable: number;
  low: number;
  medium: number;
  targeted: number;
  memoryBytes: number;
}

/** Headline figures for a report; memory counts only what can still be added. */
export function summarize(items: Recommendation[]): ScanSummary {
  const summary: ScanSummary = {
    total: items.length,
    selectable: 0,
    low: 0,
    medium: 0,
    targeted: 0,
    memoryBytes: 0,
  };
  for (const item of items) {
    if (item.already_targeted) {
      summary.targeted += 1;
      continue;
    }
    summary.selectable += 1;
    if (item.risk === "low") summary.low += 1;
    else summary.medium += 1;
    if (item.kind.kind === "process") summary.memoryBytes += item.memory_bytes;
  }
  return summary;
}
