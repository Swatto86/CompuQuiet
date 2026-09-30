/**
 * Editing a profile without touching the DOM: add, remove and rename rules
 * that the Targets view applies and the tests exercise directly. The Rust
 * side validates again on save; this layer exists so the page can explain a
 * refusal before the round trip.
 */
import type {
  AutoQuiet,
  ProcessAction,
  ProcessTarget,
  Profile,
  Settings,
} from "./bridge.ts";

export type EditResult =
  { ok: true; profile: Profile } | { ok: false; reason: string };

/**
 * What the action menu offers for a program. A slowed one is saved as a
 * suspend with `slow_down` set (see `ProcessTarget`), so the menu and the
 * file differ by one flag, and only this file knows it.
 */
export type Handling = ProcessAction | "slow_down";

export function handlingOf(target: ProcessTarget): Handling {
  return target.action === "suspend" && target.slow_down === true
    ? "slow_down"
    : target.action;
}

function handled(target: ProcessTarget, handling: Handling): ProcessTarget {
  const next: ProcessTarget = {
    ...target,
    action: handling === "close" ? "close" : "suspend",
  };
  if (handling === "slow_down") next.slow_down = true;
  else delete next.slow_down;
  return next;
}

export function normalizeName(name: string): string {
  const trimmed = name.trim();
  const stem = /\.exe$/i.test(trimmed) ? trimmed.slice(0, -4) : trimmed;
  return stem.toLowerCase();
}

function checkName(name: string, kind: string): string | null {
  const trimmed = name.trim();
  if (trimmed === "") return `Enter a ${kind} name`;
  if (trimmed.length > 128)
    return `${kind} names are limited to 128 characters`;
  // Code points rather than a regex literal: a formatter once rewrote the
  // escaped character class into raw control bytes no reviewer can see.
  const hasControl = [...trimmed].some((char) => {
    const code = char.codePointAt(0) ?? 0;
    return code < 0x20 || code === 0x7f;
  });
  if (hasControl) return `${kind} names cannot contain control characters`;
  return null;
}

function hasName(list: string[], name: string): boolean {
  const key = normalizeName(name);
  return list.some((kept) => normalizeName(kept) === key);
}

/** `list` plus `name`, unless a name that compares equal is already in it. */
function withName(list: string[], name: string): string[] {
  return hasName(list, name) ? list : [...list, name];
}

export function addProcess(
  profile: Profile,
  name: string,
  handling: Handling,
): EditResult {
  const problem = checkName(name, "program");
  if (problem) return { ok: false, reason: problem };
  const key = normalizeName(name);
  if (profile.processes.some((target) => normalizeName(target.name) === key)) {
    return { ok: false, reason: `${name.trim()} is already in the list` };
  }
  if (profile.keep_alive.some((kept) => normalizeName(kept) === key)) {
    return {
      ok: false,
      reason: `${name.trim()} is on the Never touch list; remove it there first`,
    };
  }
  return {
    ok: true,
    profile: {
      ...profile,
      processes: [
        ...profile.processes,
        handled(
          { name: name.trim(), action: "suspend", enabled: true },
          handling,
        ),
      ],
    },
  };
}

export function addService(profile: Profile, name: string): EditResult {
  const problem = checkName(name, "service");
  if (problem) return { ok: false, reason: problem };
  const key = name.trim().toLowerCase();
  if (profile.services.some((target) => target.name.toLowerCase() === key)) {
    return { ok: false, reason: `${name.trim()} is already in the list` };
  }
  if (
    profile.keep_alive.some(
      (kept) => normalizeName(kept) === normalizeName(name),
    )
  ) {
    return {
      ok: false,
      reason: `${name.trim()} is on the Never touch list; remove it there first`,
    };
  }
  return {
    ok: true,
    profile: {
      ...profile,
      services: [...profile.services, { name: name.trim(), enabled: true }],
    },
  };
}

export function addKeepAlive(profile: Profile, name: string): EditResult {
  const problem = checkName(name, "program or service");
  if (problem) return { ok: false, reason: problem };
  const key = normalizeName(name);
  if (profile.keep_alive.some((kept) => normalizeName(kept) === key)) {
    return { ok: false, reason: `${name.trim()} is already protected` };
  }
  // Nothing can be both parked and protected; protection wins.
  return {
    ok: true,
    profile: {
      ...profile,
      keep_alive: [...profile.keep_alive, name.trim()],
      processes: profile.processes.filter(
        (target) => normalizeName(target.name) !== key,
      ),
      services: profile.services.filter(
        (target) => normalizeName(target.name) !== key,
      ),
    },
  };
}

/** Programs the auto-quiet list may hold, as the engine allows. */
export const MOST_PROGRAMS = 32;

export type ProgramsResult =
  { ok: true; programs: string[] } | { ok: false; reason: string };

/** The auto-quiet list plus `name`, or why it cannot be added. */
export function addProgram(programs: string[], name: string): ProgramsResult {
  const problem = checkName(name, "program");
  if (problem) return { ok: false, reason: problem };
  const key = normalizeName(name);
  if (programs.some((listed) => normalizeName(listed) === key))
    return { ok: false, reason: `${name.trim()} is already on the list` };
  if (programs.length >= MOST_PROGRAMS)
    return {
      ok: false,
      reason: `The list holds at most ${MOST_PROGRAMS} programs`,
    };
  return { ok: true, programs: [...programs, name.trim()] };
}

/** Profiles one settings file holds, as the engine allows. */
export const MOST_PROFILES = 16;
const MOST_NAME = 40;

/** Every profile's name, alphabetically, the one in use among them. */
export function profileNames(settings: Settings): string[] {
  const key = (name: string): string => name.toLowerCase();
  return [
    settings.profile_name,
    ...settings.other_profiles.map((other) => other.name),
  ].sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : 0));
}

/**
 * Why `name` cannot be a profile's name, or null. `names` are the profiles
 * there are; `renaming` is the one being given a new name, which may keep its
 * own name in another case.
 */
export function checkProfileName(
  name: string,
  names: string[],
  renaming: string | null = null,
): string | null {
  const trimmed = name.trim();
  if (trimmed === "") return "Give the profile a name";
  if ([...trimmed].length > MOST_NAME)
    return `A profile name is at most ${MOST_NAME} characters`;
  if ([...trimmed].some((char) => (char.codePointAt(0) ?? 0) < 0x20))
    return "A profile name cannot contain control characters";
  const key = trimmed.toLowerCase();
  const taken = names.find(
    (other) =>
      other.toLowerCase() === key &&
      (renaming === null || other.toLowerCase() !== renaming.toLowerCase()),
  );
  if (taken !== undefined) return `There is a profile called ${taken} already`;
  if (renaming === null && names.length >= MOST_PROFILES)
    return `CompuQuiet keeps at most ${MOST_PROFILES} profiles`;
  return null;
}

/** The auto-quiet list with `program` starting `profile`; empty: the profile in use. */
export function setTriggerProfile(
  auto: AutoQuiet,
  program: string,
  profile: string,
): AutoQuiet {
  const profiles = { ...auto.profiles };
  if (profile === "") delete profiles[program];
  else profiles[program] = profile;
  return { ...auto, profiles };
}

/** The auto-quiet list without `program`, and without the profile it was set to start. */
export function removeTrigger(auto: AutoQuiet, program: string): AutoQuiet {
  const profiles = { ...auto.profiles };
  delete profiles[program];
  return {
    ...auto,
    programs: auto.programs.filter((listed) => listed !== program),
    profiles,
  };
}

/**
 * Removing a row is a decision, not just a deletion: a scan finds a known
 * target that is missing from the list and parks it again on every run. So
 * the name goes under Never touch, where it shows and can be taken back.
 */
export function removeProcess(profile: Profile, index: number): Profile {
  const removed = profile.processes[index];
  if (!removed) return profile;
  return {
    ...profile,
    processes: profile.processes.filter((_, i) => i !== index),
    keep_alive: withName(profile.keep_alive, removed.name),
  };
}

export function removeService(profile: Profile, index: number): Profile {
  const removed = profile.services[index];
  if (!removed) return profile;
  return {
    ...profile,
    services: profile.services.filter((_, i) => i !== index),
    keep_alive: withName(profile.keep_alive, removed.name),
  };
}

export function removeKeepAlive(profile: Profile, index: number): Profile {
  return {
    ...profile,
    keep_alive: profile.keep_alive.filter((_, i) => i !== index),
  };
}

export function setProcess(
  profile: Profile,
  index: number,
  change: { enabled?: boolean; handling?: Handling },
): Profile {
  return {
    ...profile,
    processes: profile.processes.map((target, i) => {
      if (i !== index) return target;
      const next = { ...target, enabled: change.enabled ?? target.enabled };
      return change.handling ? handled(next, change.handling) : next;
    }),
  };
}

export function setService(
  profile: Profile,
  index: number,
  enabled: boolean,
): Profile {
  return {
    ...profile,
    services: profile.services.map((target, i) =>
      i === index ? { ...target, enabled } : target,
    ),
  };
}

/**
 * Carry a change saved elsewhere (Scan adding its finds) from `before` to
 * `after` into a working copy with unsaved edits, instead of discarding the
 * edits. Scan adds targets, switches options on, and puts names under Never
 * touch (which, as when typed in, takes them off the lists).
 */
export function rebase(
  working: Profile,
  before: Profile,
  after: Profile,
): Profile {
  const next = structuredClone(working);
  const has = (list: { name: string }[], name: string): boolean =>
    list.some((target) => normalizeName(target.name) === normalizeName(name));
  for (const target of after.processes) {
    if (
      !has(before.processes, target.name) &&
      !has(next.processes, target.name)
    )
      next.processes.push(structuredClone(target));
  }
  for (const target of after.services) {
    if (!has(before.services, target.name) && !has(next.services, target.name))
      next.services.push(structuredClone(target));
  }
  for (const name of after.keep_alive) {
    if (hasName(before.keep_alive, name)) continue;
    next.keep_alive = withName(next.keep_alive, name);
    const key = normalizeName(name);
    next.processes = next.processes.filter(
      (t) => normalizeName(t.name) !== key,
    );
    next.services = next.services.filter((t) => normalizeName(t.name) !== key);
  }
  if (after.power !== before.power) next.power = after.power;
  if (after.purge_memory !== before.purge_memory)
    next.purge_memory = after.purge_memory;
  return next;
}

/** Deep equality for the "unsaved changes" indicator. */
export function sameProfile(a: Profile, b: Profile): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
