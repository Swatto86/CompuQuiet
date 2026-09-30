/**
 * The settings the page reads and saves, as `cq-core`'s `Settings` writes them.
 * Re-exported by `bridge.ts`, which is where the rest of the page imports them
 * from.
 */

export type Os = "windows" | "linux" | "mac_os";
export type ProcessAction = "suspend" | "close";
export type PowerPolicy = "leave" | "performance";
export type Theme = "system" | "dark" | "light";

export interface ProcessTarget {
  name: string;
  action: ProcessAction;
  enabled: boolean;
  /**
   * Keep it running at the lowest priority instead of freezing it. Saved on a
   * suspend, so a release that does not know it reads the file as a suspend.
   */
  slow_down?: boolean;
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
  /** Ask Ollama, LM Studio, llama.cpp and llama-swap to unload the models they hold in memory. */
  unload_ai_models: boolean;
  keep_alive: string[];
}

/** Go quiet by itself while one of these programs runs. */
export interface AutoQuiet {
  enabled: boolean;
  programs: string[];
  /** The profile a program starts, by name; one left out starts the profile in use. */
  profiles?: Record<string, string>;
}

/** A saved profile that is not the one in use. */
export interface NamedProfile {
  name: string;
  profile: Profile;
}

export interface Settings {
  version: number;
  /** The profile in use: what a press runs and the Park list edits. */
  profile: Profile;
  profile_name: string;
  /** The others. The window changes them only through the profile commands. */
  other_profiles: NamedProfile[];
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
