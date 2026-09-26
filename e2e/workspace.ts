/**
 * Isolated state for one acceptance run, and process lookups that never
 * match anything by executable name.
 *
 * The app reads `COMPUQUIET_DATA_DIR`; the driver launches it with this
 * process's environment, so pointing that at a fresh temp directory keeps the
 * suite away from an installed copy's settings and journal.
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export const DATA_DIR_ENV = "COMPUQUIET_DATA_DIR";

/** The profile the specs assume; matches the fake machine's process table. */
export const SEEDED_SETTINGS = {
  version: 1,
  profile: {
    processes: [
      { name: "OneDrive", action: "suspend", enabled: true },
      { name: "Dropbox", action: "close", enabled: true },
      { name: "Slack", action: "suspend", enabled: true },
      { name: "NotRunningApp", action: "suspend", enabled: true },
    ],
    services: [
      { name: "SysMain", enabled: true },
      { name: "DiagTrack", enabled: true },
      { name: "NoSuchService", enabled: true },
      { name: "Spooler", enabled: false },
    ],
    power: "performance",
    purge_memory: true,
    keep_alive: ["game"],
  },
  start_hidden: false,
  close_to_tray: true,
  theme: "dark",
  notifications: false,
  restore_on_quit: true,
  // Off so the quiet spec pins the profile path; scan.spec switches it on.
  auto_scan: false,
};

export function prepareWorkspace(): string {
  const dir = fs.realpathSync.native(
    fs.mkdtempSync(path.join(os.tmpdir(), "compuquiet-e2e-")),
  );
  process.env[DATA_DIR_ENV] = dir;
  fs.writeFileSync(
    path.join(dir, "settings.json"),
    JSON.stringify(SEEDED_SETTINGS, null, 2),
  );
  return dir;
}

/** PIDs of running copies of exactly this executable. */
export function appPids(application: string): number[] {
  if (process.platform === "win32") {
    const script =
      "$p = $env:CQ_E2E_APPLICATION; Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $p } | ForEach-Object { $_.ProcessId }";
    const result = execFileSync(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-Command", script],
      {
        encoding: "utf8",
        env: { ...process.env, CQ_E2E_APPLICATION: application },
      },
    );
    return result.split(/\s+/).filter(Boolean).map(Number);
  }
  return fs
    .readdirSync("/proc")
    .filter((name) => /^\d+$/.test(name))
    .flatMap((name) => {
      try {
        return fs.readlinkSync(`/proc/${name}/exe`) === application
          ? [Number(name)]
          : [];
      } catch (error) {
        const code = (error as NodeJS.ErrnoException).code ?? "";
        if (["ENOENT", "EACCES", "EPERM"].includes(code)) return [];
        throw error;
      }
    });
}
