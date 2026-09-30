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
  /** Hold off sleep and screen-off while Quiet Mode is on. */
  keep_awake: boolean;
  keep_alive: string[];
}

/** Go quiet by itself while one of these programs runs. */
export interface AutoQuiet {
  enabled: boolean;
  programs: string[];
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
  /** Run the performance plan and the memory purge on battery too. */
  allow_on_battery: boolean;
  /** Download and install a newer release without being asked. */
  auto_update: boolean;
  auto_quiet: AutoQuiet;
  /** Hours of Quiet Mode nothing will end before it is mentioned; 0 never. */
  still_on_hours: number;
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
  keep_awake: boolean;
  elevated: boolean;
  can_elevate: boolean;
}

export interface Summary {
  services_stopped: number;
  processes_suspended: number;
  processes_closed: number;
  power_changed: boolean;
  memory_purged: boolean;
  kept_awake: boolean;
}

export interface Skipped {
  name: string;
  reason: string;
}

/**
 * What the run that began Quiet Mode measurably did, read by the engine. The
 * memory a suspended program holds is kept apart from the change in what is
 * available: freezing a program frees nothing, and the change is approximate.
 */
export interface RunReport {
  /** Memory the suspended programs still hold. */
  suspended_bytes: number;
  /** Memory the closed programs held, given back when they ended. */
  closed_bytes: number;
  /** Memory a new program could use, just before the first step and after the last. */
  available_before: number;
  available_after: number;
  /** CPU load over the same two moments, in percent of the whole machine. */
  cpu_before: number;
  cpu_after: number;
}

export type PreviewAction =
  "power" | "keep_awake" | "stop_service" | "suspend" | "close" | "purge";

/** One line of the preview: a service, a program (all its processes), the plan or the purge. */
export interface PreviewItem {
  action: PreviewAction;
  /** Empty for the power plan and the purge. */
  name: string;
  processes: number;
  memory_bytes: number;
  /** For a closed program: the command line it reopens with. Null: its path was unreadable. */
  relaunch: string | null;
}

/** What one press would do at the moment it was looked at. */
export interface Preview {
  items: PreviewItem[];
  skipped: Skipped[];
  /** Low-risk finds of a quick scan this press would add, by name. */
  from_scan: string[];
  /** Seconds since the epoch. */
  taken_at: number;
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

/** What the page may ask for as the end of a run. */
export type Until =
  | { kind: "minutes"; minutes: number }
  | { kind: "program_exits"; name: string };

/** How the run in progress ends by itself. */
export interface EndingState {
  /** At a time, when a program has closed, or when what started it has. */
  kind: "timer" | "program" | "trigger";
  program: string | null;
  /** By the machine's uptime, 0 once past; only for a timer. */
  seconds_left: number | null;
}

export interface EngineState {
  quiet: boolean;
  busy: boolean;
  started_at: number | null;
  summary: Summary;
  /** Measured by the engine; null when Quiet Mode is off or was recovered. */
  run_report: RunReport | null;
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
  ending: EndingState | null;
}

export interface SystemStats {
  cpu_percent: number;
  memory_total: number;
  memory_used: number;
  memory_available: number;
  memory_free: number;
  process_count: number;
}

/** One graphics adapter's own memory, in bytes. */
export interface GpuAdapter {
  name: string;
  used: number;
  total: number;
}

/**
 * The machine's graphics memory. Either adapters, or `unavailable` saying why
 * there are none: not being able to read it is a fact about the machine.
 */
export interface GpuReading {
  adapters: GpuAdapter[];
  unavailable: string | null;
}

export interface ProcessRow {
  name: string;
  instances: number;
  memory_bytes: number;
  cpu_percent: number;
  exe: string | null;
}

export type ServiceState =
  "running" | "stopped" | "transitioning" | "not_installed";

/** A service the machine has, for the Park list's picker. */
export interface ServiceRow {
  name: string;
  display_name: string;
  state: ServiceState;
  /** Quiet Mode never stops it, so the picker does not offer it. */
  essential: boolean;
}

export interface AutostartStatus {
  enabled: boolean;
  elevated: boolean;
  allowed: boolean;
  reason: string | null;
  /** Why this copy cannot change the entry at all, when it cannot. */
  locked: string | null;
  /**
   * Why an entry made from this copy starts without administrator rights
   * although this copy has them (it is not in Program Files).
   */
  limited_because: string | null;
}

/** Where self-updating stands; mirrors `update::Status` in src-tauri. */
export type UpdateStatus =
  | { kind: "unavailable"; reason: string }
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "up_to_date" }
  | { kind: "available"; version: string }
  | { kind: "downloading"; version: string }
  | { kind: "ready"; version: string; asks_permission: boolean }
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
  getGpu: () => invoke<GpuReading>("get_gpu"),
  listProcesses: () => invoke<ProcessRow[]>("list_processes"),
  listServices: () => invoke<ServiceRow[]>("list_services"),
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
  previewPlan: () => invoke<Preview>("preview_plan"),
  goQuiet: (until: Until | null = null) =>
    invoke<EngineState>("go_quiet", { until }),
  /** Change how the run in progress ends; null leaves it to the user. */
  setEnding: (until: Until | null) =>
    invoke<EngineState>("set_ending", { until }),
  restore: () => invoke<EngineState>("restore"),
  frontendReady: () => invoke<void>("frontend_ready"),
  getAutostart: () => invoke<AutostartStatus>("get_autostart"),
  setAutostart: (enabled: boolean) =>
    invoke<AutostartStatus>("set_autostart", { enabled }),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  checkForUpdates: () => invoke<UpdateStatus>("check_for_updates"),
  /** A report for the clipboard, built and redacted in Rust. */
  diagnostics: () => invoke<string>("diagnostics"),
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

/** Something the app did by itself (a timer, an auto-quiet start, a reminder). */
export function onNotice(handler: (text: string) => void): Promise<UnlistenFn> {
  return listen<string>("quiet-notice", (event) => handler(event.payload));
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
