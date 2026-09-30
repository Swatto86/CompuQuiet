import { strict as assert } from "node:assert";
import { test } from "node:test";

import type { AutoQuiet, Profile, Settings } from "./bridge.ts";
import {
  MOST_PROFILES,
  MOST_PROGRAMS,
  addKeepAlive,
  addProgram,
  addProcess,
  addService,
  checkProfileName,
  handlingOf,
  normalizeName,
  profileNames,
  rebase,
  removeKeepAlive,
  removeProcess,
  removeService,
  removeTrigger,
  restoreDefaults,
  sameProfile,
  setProcess,
  setService,
  setTriggerProfile,
} from "./profile-edit.ts";

function profile(): Profile {
  return {
    processes: [{ name: "OneDrive", action: "suspend", enabled: true }],
    services: [{ name: "SysMain", enabled: true }],
    power: "performance",
    purge_memory: true,
    keep_awake: false,
    unload_ai_models: false,
    keep_alive: [],
  };
}

test("names compare without case or .exe", () => {
  assert.equal(normalizeName("  OneDrive.EXE "), "onedrive");
  assert.equal(normalizeName("slack"), "slack");
});

test("adding a duplicate or hostile process is refused with a reason", () => {
  const dup = addProcess(profile(), "onedrive.exe", "close");
  assert.equal(dup.ok, false);
  assert.match(!dup.ok ? dup.reason : "", /already/);
  const empty = addProcess(profile(), "   ", "suspend");
  assert.equal(empty.ok, false);
  const control = addProcess(profile(), "bad\u0000name", "suspend");
  assert.equal(control.ok, false);
  const long = addProcess(profile(), "x".repeat(129), "suspend");
  assert.equal(long.ok, false);
});

test("a valid process is appended enabled and the original is untouched", () => {
  const before = profile();
  const result = addProcess(before, " Dropbox ", "close");
  assert.ok(result.ok);
  if (!result.ok) return;
  assert.deepEqual(result.profile.processes[1], {
    name: "Dropbox",
    action: "close",
    enabled: true,
  });
  assert.equal(before.processes.length, 1);
  assert.ok(!sameProfile(before, result.profile));
});

test("services are case-insensitive duplicates and keep-alive removes a parked program", () => {
  assert.equal(addService(profile(), "sysmain").ok, false);
  const added = addService(profile(), "WSearch");
  assert.ok(added.ok && added.profile.services.length === 2);

  const kept = addKeepAlive(profile(), "OneDrive.exe");
  assert.ok(kept.ok);
  if (!kept.ok) return;
  assert.deepEqual(kept.profile.keep_alive, ["OneDrive.exe"]);
  assert.deepEqual(
    kept.profile.processes,
    [],
    "a protected program is no longer parked",
  );
  assert.equal(addProcess(kept.profile, "onedrive", "suspend").ok, false);
});

test("rows can be toggled, retargeted and removed by index", () => {
  let edited = setProcess(profile(), 0, { enabled: false, handling: "close" });
  assert.deepEqual(edited.processes[0], {
    name: "OneDrive",
    action: "close",
    enabled: false,
  });
  edited = setService(edited, 0, false);
  assert.equal(edited.services[0]?.enabled, false);
  edited = removeProcess(edited, 0);
  assert.equal(edited.processes.length, 0);
  assert.equal(
    removeProcess(edited, 5).processes.length,
    0,
    "an out-of-range index is a no-op",
  );
});

test("a slowed program is a suspend with a flag, which an older release reads as a suspend", () => {
  const slowed = setProcess(profile(), 0, { handling: "slow_down" });
  assert.deepEqual(slowed.processes[0], {
    name: "OneDrive",
    action: "suspend",
    enabled: true,
    slow_down: true,
  });
  assert.equal(handlingOf(slowed.processes[0]!), "slow_down");
  // Back to a plain suspend, or on to a close, leaves no flag behind.
  const suspended = setProcess(slowed, 0, { handling: "suspend" });
  assert.equal("slow_down" in suspended.processes[0]!, false);
  const closed = setProcess(slowed, 0, { handling: "close" });
  assert.deepEqual(closed.processes[0], {
    name: "OneDrive",
    action: "close",
    enabled: true,
  });
  assert.equal(handlingOf(closed.processes[0]!), "close");
  // Toggling it keeps how it is handled.
  const off = setProcess(slowed, 0, { enabled: false });
  assert.equal(handlingOf(off.processes[0]!), "slow_down");
  // A new row can be added slowed.
  const added = addProcess(profile(), "Dropbox", "slow_down");
  assert.ok(added.ok);
  assert.equal(handlingOf(added.profile.processes.at(-1)!), "slow_down");
});

test("removing a row records it under Never touch, so a scan does not bring it back", () => {
  const before = profile();
  const noProcess = removeProcess(before, 0);
  assert.deepEqual(noProcess.processes, []);
  assert.deepEqual(noProcess.keep_alive, ["OneDrive"]);
  const noService = removeService(noProcess, 0);
  assert.deepEqual(noService.services, []);
  assert.deepEqual(noService.keep_alive, ["OneDrive", "SysMain"]);
  assert.deepEqual(before.keep_alive, [], "the original is untouched");

  const already = { ...profile(), keep_alive: ["onedrive.exe"] };
  assert.deepEqual(
    removeProcess(already, 0).keep_alive,
    ["onedrive.exe"],
    "a name already protected is not listed twice",
  );
  assert.equal(removeService(before, 9), before, "no such row, no change");
});

test("a protected service leaves the service list and stays out until it is unprotected", () => {
  const kept = addKeepAlive(profile(), "sysmain");
  assert.ok(kept.ok);
  if (!kept.ok) return;
  assert.deepEqual(kept.profile.services, []);
  const refused = addService(kept.profile, "SysMain");
  assert.equal(refused.ok, false);
  assert.match(!refused.ok ? refused.reason : "", /Never touch/);
  const freed = addService(removeKeepAlive(kept.profile, 0), "SysMain");
  assert.ok(freed.ok && freed.profile.services.length === 1);
});

test("restoring the defaults keeps what is under Never touch and leaves it off the lists", () => {
  const current = {
    ...profile(),
    processes: [],
    services: [],
    keep_alive: ["onedrive.exe", "obs64", "SysMain"],
  };
  const defaults: Profile = {
    ...profile(),
    processes: [
      { name: "OneDrive", action: "suspend", enabled: true },
      { name: "Dropbox", action: "suspend", enabled: true },
    ],
    services: [
      { name: "sysmain", enabled: true },
      { name: "Fax", enabled: true },
    ],
  };
  const restored = restoreDefaults(current, defaults);
  assert.deepEqual(restored.keep_alive, ["onedrive.exe", "obs64", "SysMain"]);
  assert.deepEqual(
    restored.processes.map((target) => target.name),
    ["Dropbox"],
  );
  assert.deepEqual(
    restored.services.map((target) => target.name),
    ["Fax"],
  );
  assert.equal(restored.power, defaults.power, "the rest is the defaults'");
  assert.deepEqual(defaults.processes.length, 2, "the defaults are untouched");
});

test("finds saved from Scan join unsaved edits instead of replacing them", () => {
  const saved = profile();
  // Unsaved: OneDrive switched off.
  const working = setProcess(saved, 0, { enabled: false });
  // Saved elsewhere: Scan added Dropbox and turned on the memory purge.
  const after = structuredClone(saved);
  after.processes.push({ name: "Dropbox", action: "close", enabled: true });
  after.purge_memory = !saved.purge_memory;

  const next = rebase(working, saved, after);
  assert.equal(next.processes[0]?.enabled, false, "the edit survives");
  assert.deepEqual(next.processes[1], {
    name: "Dropbox",
    action: "close",
    enabled: true,
  });
  assert.equal(next.purge_memory, after.purge_memory);
  // Already added by hand under another spelling: not added twice.
  const typed = addProcess(working, "dropbox.exe", "close");
  assert.ok(typed.ok);
  if (typed.ok)
    assert.equal(rebase(typed.profile, saved, after).processes.length, 2);
});

test("a program joins the auto-quiet list once, whatever its spelling, up to the limit", () => {
  const added = addProgram([], " Steam.exe ");
  assert.deepEqual(added, { ok: true, programs: ["Steam.exe"] });
  const again = addProgram(["Steam.exe"], "steam");
  assert.equal(again.ok, false);
  assert.equal(again.ok ? "" : again.reason, "steam is already on the list");
  assert.equal(addProgram([], "   ").ok, false);
  assert.equal(addProgram([], "x".repeat(129)).ok, false);
  const full = Array.from({ length: MOST_PROGRAMS }, (_, i) => `game-${i}`);
  const over = addProgram(full, "one-more");
  assert.equal(over.ok, false);
  assert.equal(
    over.ok ? "" : over.reason,
    `The list holds at most ${MOST_PROGRAMS} programs`,
  );
});

test("a name protected from Scan is carried into unsaved edits and takes the program off them", () => {
  const saved = profile();
  // Unsaved: a service switched off, and Dropbox added by hand.
  const typed = addProcess(setService(saved, 0, false), "Dropbox", "suspend");
  assert.ok(typed.ok);
  if (!typed.ok) return;
  // Saved elsewhere: Scan's Never touch on Dropbox and on SysMain.
  const protectedNames = addKeepAlive(saved, "dropbox.exe");
  assert.ok(protectedNames.ok);
  if (!protectedNames.ok) return;
  const after = addKeepAlive(protectedNames.profile, "SysMain");
  assert.ok(after.ok);
  if (!after.ok) return;

  const next = rebase(typed.profile, saved, after.profile);
  assert.deepEqual(next.keep_alive, ["dropbox.exe", "SysMain"]);
  assert.deepEqual(
    next.processes.map((target) => target.name),
    ["OneDrive"],
    "protection wins over the unsaved add",
  );
  assert.deepEqual(next.services, [], "and over the service, edit or not");
  // A name that was already protected when the edits began is not re-added.
  const again = rebase(next, after.profile, after.profile);
  assert.deepEqual(again.keep_alive, next.keep_alive);
});

function settingsWith(active: string, others: string[]): Settings {
  return {
    profile_name: active,
    profile: profile(),
    other_profiles: others.map((name) => ({ name, profile: profile() })),
  } as Settings;
}

test("profiles are listed alphabetically, the one in use among them", () => {
  assert.deepEqual(
    profileNames(settingsWith("Work", ["gaming", "Default", "Local AI"])),
    ["Default", "gaming", "Local AI", "Work"],
  );
  assert.deepEqual(profileNames(settingsWith("Default", [])), ["Default"]);
});

test("a profile name is checked the way the engine checks it", () => {
  const names = ["Default", "Gaming"];
  assert.equal(checkProfileName("Work", names), null);
  assert.equal(checkProfileName("  Work  ", names), null);
  assert.match(checkProfileName("", names) ?? "", /name/);
  assert.match(checkProfileName("   ", names) ?? "", /name/);
  assert.match(checkProfileName("x".repeat(41), names) ?? "", /at most 40/);
  assert.equal(checkProfileName("x".repeat(40), names), null);
  assert.match(checkProfileName("a\u0001b", names) ?? "", /control/);
  // The same name in another case is the same name.
  assert.match(checkProfileName("gaming", names) ?? "", /Gaming already/);
  const many = Array.from({ length: MOST_PROFILES }, (_, n) => `P${n}`);
  assert.match(checkProfileName("One more", many) ?? "", /at most 16/);
});

test("renaming may change a profile's case but not take another's name", () => {
  const names = ["Default", "Gaming"];
  assert.equal(checkProfileName("gaming", names, "Gaming"), null);
  assert.equal(checkProfileName("Games", names, "Gaming"), null);
  assert.match(checkProfileName("default", names, "Gaming") ?? "", /already/);
  // A full list can still rename.
  const many = Array.from({ length: MOST_PROFILES }, (_, n) => `P${n}`);
  assert.equal(checkProfileName("Other", many, "P3"), null);
});

test("a program starts the profile it was set to, and none is the one in use", () => {
  const auto: AutoQuiet = { enabled: true, programs: ["steam", "ollama"] };
  const set = setTriggerProfile(auto, "steam", "Gaming");
  assert.deepEqual(set.profiles, { steam: "Gaming" });
  assert.deepEqual(setTriggerProfile(set, "steam", "").profiles, {});
  assert.deepEqual(auto.profiles, undefined, "the original is not edited");
});

test("removing a program takes the profile it started with it", () => {
  const auto: AutoQuiet = {
    enabled: true,
    programs: ["steam", "ollama"],
    profiles: { steam: "Gaming", ollama: "Local AI" },
  };
  const rest = removeTrigger(auto, "steam");
  assert.deepEqual(rest.programs, ["ollama"]);
  assert.deepEqual(rest.profiles, { ollama: "Local AI" });
  assert.deepEqual(removeTrigger(rest, "nothing").programs, ["ollama"]);
});
