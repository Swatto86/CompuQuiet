/** The preview as text, tested without a DOM. */
import type { Preview, PreviewItem, Skipped } from "./bridge.ts";
import { formatBytes, plural } from "./format.ts";

export interface PreviewLine {
  text: string;
  /** The command line a closed program reopens with, shown as code. */
  command: string | null;
}

/** One planned step, in words. */
export function itemLine(item: PreviewItem): PreviewLine {
  const size =
    item.processes > 0
      ? ` (${plural(item.processes, "process", "processes")}, ${formatBytes(item.memory_bytes)})`
      : "";
  switch (item.action) {
    case "power":
      return { text: "Switch to the performance power plan", command: null };
    case "keep_awake":
      return { text: "Keep the PC awake", command: null };
    case "purge":
      return { text: "Purge cached memory", command: null };
    case "unload_model":
      return {
        text: `Unload ${item.name}${item.memory_bytes > 0 ? `, ${formatBytes(item.memory_bytes)}` : ""}. It loads again when it is next used`,
        command: null,
      };
    case "close_server":
      return {
        text: `llama.cpp server ${item.name}${size}: closed now, started again with the same settings when Quiet Mode ends, with`,
        command: item.relaunch,
      };
    case "stop_service":
      return { text: `Stop service ${item.name}`, command: null };
    case "suspend":
      return { text: `Suspend ${item.name}${size}`, command: null };
    case "slow_down":
      return {
        text: `Slow down ${item.name}${size}. It keeps running at the lowest priority`,
        command: null,
      };
    case "close":
      return item.relaunch === null
        ? {
            text: `Close ${item.name}${size}. Its path could not be read, so it could not be opened again`,
            command: null,
          }
        : {
            text: `Close ${item.name}${size}. Restore opens it again with`,
            command: item.relaunch,
          };
  }
}

/**
 * What is left alone, one line per reason so a long list of programs that are
 * not running stays short: "A, B, C — not running".
 */
export function leftAlone(skipped: Skipped[]): string[] {
  const byReason = new Map<string, string[]>();
  for (const entry of skipped) {
    const names = byReason.get(entry.reason) ?? [];
    names.push(entry.name);
    byReason.set(entry.reason, names);
  }
  return [...byReason].map(
    ([reason, names]) => `${names.join(", ")} — ${reason}`,
  );
}

/** The finds a quick scan adds for this press only, or null when there are none. */
export function fromScan(preview: Preview): string | null {
  return preview.from_scan.length === 0
    ? null
    : `A quick scan also adds ${preview.from_scan.join(", ")} for this press. Your saved list is not changed.`;
}
