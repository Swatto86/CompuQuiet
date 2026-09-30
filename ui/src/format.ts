/** Pure presentation helpers, tested without a DOM. */
import type { Profile, RunReport, Summary, UpdateStatus } from "./bridge.ts";

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit === 0 ? 0 : value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)} ${UNITS[unit]}`;
}

export function formatPercent(value: number): string {
  if (!Number.isFinite(value)) return "—";
  return `${Math.max(0, Math.min(100, Math.round(value)))}%`;
}

/**
 * A program's CPU as a share of one core, summed over its processes, so it
 * can pass 100: eight helpers at a third of a core each read "267%".
 */
export function formatCoreShare(value: number): string {
  if (!Number.isFinite(value)) return "—";
  return `${Math.max(0, Math.round(value))}%`;
}

/** "3 s", "12 min", "1 h 5 min", "2 d 3 h". */
export function formatSince(startedAt: number, now: number): string {
  const seconds = Math.max(0, Math.floor(now - startedAt));
  if (seconds < 60) return `${seconds} s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    const rest = minutes % 60;
    return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`;
  }
  const days = Math.floor(hours / 24);
  const restHours = hours % 24;
  return restHours === 0 ? `${days} d` : `${days} d ${restHours} h`;
}

export function plural(count: number, one: string, many: string): string {
  return `${count} ${count === 1 ? one : many}`;
}

/** Human lines for the "what Quiet Mode did" card; empty when nothing happened. */
export function summaryLines(summary: Summary): string[] {
  const lines: string[] = [];
  if (summary.services_stopped > 0)
    lines.push(
      `${plural(summary.services_stopped, "service", "services")} stopped`,
    );
  if (summary.processes_suspended > 0)
    lines.push(
      `${plural(summary.processes_suspended, "process", "processes")} suspended`,
    );
  if (summary.processes_closed > 0)
    lines.push(
      `${plural(summary.processes_closed, "process", "processes")} closed (relaunched on restore)`,
    );
  if (summary.power_changed) lines.push("Performance power plan active");
  if (summary.memory_purged) lines.push("Cached memory purged");
  return lines;
}

/**
 * What Quiet Mode gave back, as the engine measured it: the rise in the memory
 * available, never below zero. Null when there is no measurement.
 */
export function memoryFreed(report: RunReport | null): number | null {
  return report === null
    ? null
    : Math.max(0, report.available_after - report.available_before);
}

/**
 * What the run measurably did, for the card beside the counts. Parked and
 * freed stay apart: a suspended program still holds its memory, so only the
 * change in what is available says what came back, and that is approximate.
 */
export function reportLines(report: RunReport | null): string[] {
  if (report === null) return [];
  const lines = [
    `Memory available ${formatBytes(report.available_before)} to ${formatBytes(report.available_after)} (approximate)`,
    `CPU ${formatPercent(report.cpu_before)} to ${formatPercent(report.cpu_after)}`,
  ];
  if (report.suspended_bytes > 0)
    lines.push(
      `Suspended programs still hold ${formatBytes(report.suspended_bytes)}: only closing one frees its memory`,
    );
  if (report.closed_bytes > 0)
    lines.push(`Closed programs held ${formatBytes(report.closed_bytes)}`);
  return lines;
}

function joinAnd(parts: string[]): string {
  if (parts.length <= 1) return parts[0] ?? "";
  if (parts.length === 2) return `${parts[0]} and ${parts[1]}`;
  return `${parts.slice(0, -1).join(", ")}, and ${parts[parts.length - 1]}`;
}

export interface PlanContext {
  /** Quiet Mode also parks the low-risk finds of a quick scan. */
  autoScan: boolean;
  /** The Park list has edits that are not saved, so the button ignores them. */
  unsaved: boolean;
}

/**
 * One sentence for the home screen: what the big button will do, counted
 * from the saved list and the scan setting so the user does not have to open
 * another tab to find out.
 */
export function homePlan(
  quiet: boolean,
  profile: Profile,
  context: PlanContext = { autoScan: false, unsaved: false },
): string {
  const unsaved = context.unsaved
    ? " Unsaved Park list changes are not used yet."
    : "";
  if (quiet) {
    return `Those changes are still in place. Press the button again, or choose Put everything back in the tray menu, to undo them.${unsaved}`;
  }
  const programs = profile.processes.filter((item) => item.enabled).length;
  const services = profile.services.filter((item) => item.enabled).length;
  const actions: string[] = [];
  if (programs > 0)
    actions.push(`park ${plural(programs, "program", "programs")}`);
  if (services > 0)
    actions.push(`stop ${plural(services, "service", "services")}`);
  if (profile.power === "performance")
    actions.push("switch to the performance power plan");
  if (profile.purge_memory) actions.push("purge cached memory");
  const found = "low-risk programs and services a quick scan finds";
  if (actions.length === 0) {
    return context.autoScan
      ? `Your park list is empty, so one press will park only the ${found}. Open Park list to choose your own.${unsaved}`
      : `Nothing is selected yet. Open Park list and tick what this button should touch.${unsaved}`;
  }
  const extra = context.autoScan ? `, plus any ${found}` : "";
  return `One press will ${joinAnd(actions)}${extra}. Press again to undo it.${unsaved}`;
}

/** What closing the window does, which follows the tray setting. */
export function closeHint(closeToTray: boolean): string {
  return closeToTray
    ? "Closing this window leaves CompuQuiet in the tray. Right-click that icon for the same button, to open this window, or to quit."
    : "Closing this window quits CompuQuiet. Turn on Keep running in the tray in Settings to leave it running instead.";
}

/** Where a copy that cannot update itself, or has it switched off, gets the release. */
export const RELEASES_PAGE = "github.com/Swatto86/CompuQuiet/releases";

/** Where self-updating stands, in a sentence for the About card. */
export function updateLine(status: UpdateStatus): string {
  switch (status.kind) {
    case "unavailable":
      return `${status.reason} Newer releases are at ${RELEASES_PAGE}.`;
    case "idle":
      return "Looks for a newer release now and then while CompuQuiet runs.";
    case "checking":
      return "Looking for a newer release…";
    case "up_to_date":
      return "This is the latest release.";
    case "available":
      return `${status.version} is available. Turn on Install updates automatically to get it, or download it from ${RELEASES_PAGE}.`;
    case "downloading":
      return `Downloading ${status.version}…`;
    case "ready":
      return `${status.version} is downloaded. It installs when Quiet Mode is off and you close the window to the tray.${
        status.asks_permission
          ? " Windows will ask for permission when it installs."
          : ""
      }`;
    case "failed":
      return `The last check failed (${status.error}). It tries again later.`;
  }
}

/** Whether a manual check would do anything now. */
export function canCheckForUpdates(status: UpdateStatus): boolean {
  return (
    status.kind === "idle" ||
    status.kind === "up_to_date" ||
    status.kind === "available" ||
    status.kind === "failed"
  );
}
