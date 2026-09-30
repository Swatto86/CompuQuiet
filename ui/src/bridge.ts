/**
 * The typed edge of the IPC boundary. Every shape here mirrors a Rust struct
 * in `src-tauri` or `cq-core`; the acceptance suite drives the same commands
 * through the real webview, so a drift shows up there.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Os = "windows" | "linux" | "mac_os";
export type ProcessAction = "suspend" | "close";
export type PowerPolicy = "leave" | "performance";
export type Theme = "system" | "dark" | "light";

export interface ProcessTarget {
  name: string;
  action: ProcessAction;
  enabled: boolean;
}

export interface ServiceTarget {
  name: string;
  enabled: boolean;
}

export interface Profile {
  processes: ProcessTarget[];
  services: ServiceTarget[];
  power: PowerPolicy;
  purge_memory: boolean;
  keep_alive: string[];
}

export interface Settings {
  version: number;
  profile: Profile;
  start_hidden: boolean;
  close_to_tray: boolean;
  theme: Theme;
  notifications: boolean;
  restore_on_quit: boolean;
  auto_scan: boolean;
}

export type Risk = "low" | "medium";

export type RecommendationKind =
  | { kind: "process"; action: ProcessAction }
  | { kind: "service" }
  | { kind: "power_plan" }
  | { kind: "memory_purge" };

export interface Recommendation {
  kind: RecommendationKind;
  name: string;
  reason: string;
  risk: Risk;
  memory_bytes: number;
  cpu_percent: number;
  instances: number;
  already_targeted: boolean;
}

export interface ScanReport {
  recommendations: Recommendation[];
  scanned_at: number;
  activity_known: boolean;
  cached_bytes: number;
}

export interface Capabilities {
  services: boolean;
  power: boolean;
  memory_purge: boolean;
  elevated: boolean;
  can_elevate: boolean;
}

export interface Summary {
  services_stopped: number;
  processes_suspended: number;
  processes_closed: number;
  power_changed: boolean;
  memory_purged: boolean;
}

export interface Skipped {
  name: string;
  reason: string;
}

export interface LogLine {
  label: string;
  ok: boolean;
  detail: string | null;
}

/** A step the last restore could not undo. */
export interface Unrestored {
  label: string;
  /** What stays as it is if it is given up. */
  consequence: string;
  /** Why the last attempt failed. */
  error: string | null;
}

export interface EngineState {
  quiet: boolean;
  busy: boolean;
  started_at: number | null;
  summary: Summary;
  skipped: Skipped[];
  log: LogLine[];
  capabilities: Capabilities;
  data_dir: string;
  os: Os;
  recovered: boolean;
  /** Why the journal cannot be read, while it is still on disk unread. */
  startup_error: string | null;
  /** Why settings.json cannot be read, until it is fixed or set aside. */
  settings_unreadable: string | null;
  /** What the last restore could not put back; empty once one succeeds. */
  unrestored: Unrestored[];
}

export interface SystemStats {
  cpu_percent: number;
  memory_total: number;
  memory_used: number;
  memory_available: number;
  memory_free: number;
  process_count: number;
}

export interface ProcessRow {
  name: string;
  instances: number;
  memory_bytes: number;
  cpu_percent: number;
  exe: string | null;
}

export interface AutostartStatus {
  enabled: boolean;
  elevated: boolean;
  allowed: boolean;
  reason: string | null;
  /** Why this copy cannot change the entry at all, when it cannot. */
  locked: string | null;
}

/** Where self-updating stands; mirrors `update::Status` in src-tauri. */
export type UpdateStatus =
  | { kind: "unavailable"; reason: string }
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "up_to_date" }
  | { kind: "downloading"; version: string }
  | { kind: "ready"; version: string }
  | { kind: "failed"; error: string };

export interface AppInfo {
  version: string;
  os: Os;
  data_dir: string;
  debug: boolean;
}

export interface AppError {
  code: string;
  message: string;
}

export function isAppError(value: unknown): value is AppError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as AppError).code === "string" &&
    typeof (value as AppError).message === "string"
  );
}

export function errorMessage(error: unknown): string {
  if (isAppError(error)) return error.message;
  if (error instanceof Error) return error.message;
  return String(error);
}

export const api = {
  getState: () => invoke<EngineState>("get_state"),
  appInfo: () => invoke<AppInfo>("app_info"),
  getStats: () => invoke<SystemStats>("get_stats"),
  listProcesses: () => invoke<ProcessRow[]>("list_processes"),
  getSettings: () => invoke<Settings>("get_settings"),
  defaultSettings: () => invoke<Settings>("default_settings"),
  scan: () => invoke<ScanReport>("scan"),
  applyRecommendations: (accepted: Recommendation[]) =>
    invoke<Settings>("apply_recommendations", { accepted }),
  saveSettings: (settings: Settings) =>
    invoke<void>("save_settings", { settings }),
  setAsideSettings: () => invoke<string | null>("set_aside_settings"),
  giveUpRestoring: () => invoke<EngineState>("give_up_restoring"),
  setAsideJournal: () => invoke<string | null>("set_aside_journal"),
  goQuiet: () => invoke<EngineState>("go_quiet"),
  restore: () => invoke<EngineState>("restore"),
  frontendReady: () => invoke<void>("frontend_ready"),
  getAutostart: () => invoke<AutostartStatus>("get_autostart"),
  setAutostart: (enabled: boolean) =>
    invoke<AutostartStatus>("set_autostart", { enabled }),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  checkForUpdates: () => invoke<UpdateStatus>("check_for_updates"),
  relaunchElevated: () => invoke<void>("relaunch_elevated"),
  quit: (restoreFirst: boolean) => invoke<void>("quit", { restoreFirst }),
};

export function onProgress(
  handler: (line: LogLine) => void,
): Promise<UnlistenFn> {
  return listen<LogLine>("quiet-progress", (event) => handler(event.payload));
}

export function onState(
  handler: (state: EngineState) => void,
): Promise<UnlistenFn> {
  return listen<EngineState>("quiet-state", (event) => handler(event.payload));
}

/** A run the page did not start (the tray's) failed. */
export function onRunError(
  handler: (error: AppError) => void,
): Promise<UnlistenFn> {
  return listen<AppError>("quiet-error", (event) => handler(event.payload));
}

export function onUpdateStatus(
  handler: (status: UpdateStatus) => void,
): Promise<UnlistenFn> {
  return listen<UpdateStatus>("update-status", (event) =>
    handler(event.payload),
  );
}

export function onConfirmQuit(handler: () => void): Promise<UnlistenFn> {
  return listen("confirm-quit", () => handler());
}
