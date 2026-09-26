/** Pure presentation helpers, tested without a DOM. */
import type { Profile, Summary } from "./bridge.ts";

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

function plural(count: number, one: string, many: string): string {
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

function joinAnd(parts: string[]): string {
  if (parts.length <= 1) return parts[0] ?? "";
  if (parts.length === 2) return `${parts[0]} and ${parts[1]}`;
  return `${parts.slice(0, -1).join(", ")}, and ${parts[parts.length - 1]}`;
}

/**
 * One sentence for the home screen: what the big button will do, counted
 * from the saved list so the user does not have to open another tab to find out.
 */
export function homePlan(quiet: boolean, profile: Profile): string {
  if (quiet) {
    return "Those changes are still in place. Press the button again, or choose Put everything back in the tray menu, to undo them.";
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
  if (actions.length === 0) {
    return "Nothing is selected yet. Open Park list and tick what this button should touch.";
  }
  return `One press will ${joinAnd(actions)}. Press again to undo it.`;
}
