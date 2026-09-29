/**
 * Editing a profile without touching the DOM: add, remove and rename rules
 * that the Targets view applies and the tests exercise directly. The Rust
 * side validates again on save; this layer exists so the page can explain a
 * refusal before the round trip.
 */
import type { ProcessAction, Profile } from "./bridge.ts";

export type EditResult =
  { ok: true; profile: Profile } | { ok: false; reason: string };

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

export function addProcess(
  profile: Profile,
  name: string,
  action: ProcessAction,
): EditResult {
  const problem = checkName(name, "program");
  if (problem) return { ok: false, reason: problem };
  const key = normalizeName(name);
  if (profile.processes.some((target) => normalizeName(target.name) === key)) {
    return { ok: false, reason: `${name.trim()} is already in the list` };
  }
  if (profile.keep_alive.some((kept) => normalizeName(kept) === key)) {
    return { ok: false, reason: `${name.trim()} is on the keep-alive list` };
  }
  return {
    ok: true,
    profile: {
      ...profile,
      processes: [
        ...profile.processes,
        { name: name.trim(), action, enabled: true },
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
  return {
    ok: true,
    profile: {
      ...profile,
      services: [...profile.services, { name: name.trim(), enabled: true }],
    },
  };
}

export function addKeepAlive(profile: Profile, name: string): EditResult {
  const problem = checkName(name, "program");
  if (problem) return { ok: false, reason: problem };
  const key = normalizeName(name);
  if (profile.keep_alive.some((kept) => normalizeName(kept) === key)) {
    return { ok: false, reason: `${name.trim()} is already protected` };
  }
  // A program cannot be both parked and protected; protection wins.
  return {
    ok: true,
    profile: {
      ...profile,
      keep_alive: [...profile.keep_alive, name.trim()],
      processes: profile.processes.filter(
        (target) => normalizeName(target.name) !== key,
      ),
    },
  };
}

export function removeProcess(profile: Profile, index: number): Profile {
  return {
    ...profile,
    processes: profile.processes.filter((_, i) => i !== index),
  };
}

export function removeService(profile: Profile, index: number): Profile {
  return {
    ...profile,
    services: profile.services.filter((_, i) => i !== index),
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
  change: { enabled?: boolean; action?: ProcessAction },
): Profile {
  return {
    ...profile,
    processes: profile.processes.map((target, i) =>
      i === index ? { ...target, ...change } : target,
    ),
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
 * edits. Scan only adds targets and switches options on.
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
  if (after.power !== before.power) next.power = after.power;
  if (after.purge_memory !== before.purge_memory)
    next.purge_memory = after.purge_memory;
  return next;
}

/** Deep equality for the "unsaved changes" indicator. */
export function sameProfile(a: Profile, b: Profile): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
