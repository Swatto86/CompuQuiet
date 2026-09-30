/**
 * Text and matching for the Park list, without the DOM: what the filter box
 * keeps, what a service picker's entry says, and what the page tells about
 * this system's abilities.
 */
import type { Capabilities, Os, ServiceRow, ServiceState } from "./bridge.ts";

/**
 * Whether every word typed in the filter is somewhere in the names given
 * (a program's or service's own, and the name the machine gives a service).
 * Nothing typed keeps everything.
 */
export function matchesFilter(
  filter: string,
  ...names: (string | undefined)[]
): boolean {
  const words = filter.toLowerCase().split(/\s+/).filter(Boolean);
  const text = names.join(" ").toLowerCase();
  return words.every((word) => text.includes(word));
}

export function stateText(state: ServiceState): string {
  switch (state) {
    case "running":
      return "running";
    case "stopped":
      return "stopped";
    case "transitioning":
      return "starting or stopping";
    case "not_installed":
      return "not installed";
  }
}

/** What one entry of the service picker says beside its name. */
export function pickerLabel(row: ServiceRow): string {
  return `${row.display_name} · ${stateText(row.state)}`;
}

/** The picker offers what Quiet Mode may stop; the essential is left out. */
export function offered(rows: ServiceRow[]): ServiceRow[] {
  return rows.filter((row) => !row.essential);
}

/** What the Services card says about naming a service on this system. */
export function serviceHint(caps: Capabilities, os: Os): string {
  // Rust refuses these when the list is saved; say so before that.
  const essential = " Essential ones (sound, network, security) are refused.";
  if (os === "linux")
    return (
      "systemd units; prefix user units with user: (e.g. user:tracker-miner-fs-3)." +
      essential
    );
  if (os === "mac_os")
    return "launchd agent labels, e.g. com.microsoft.update.agent." + essential;
  if (!caps.services) return "Stopping services needs administrator rights.";
  return "Windows service names, as shown in services.msc." + essential;
}

/** What the "keep the PC awake" option says about whether this system can. */
export function awakeHint(caps: Capabilities): string {
  return caps.keep_awake
    ? "Stops sleep and the screen turning off until you put everything back. Closing a laptop's lid still sleeps it. On battery it is left out unless you allow that in Settings."
    : "Not available on this system: it has no tool CompuQuiet can ask to hold off sleep (systemd-inhibit on Linux, caffeinate on macOS).";
}
