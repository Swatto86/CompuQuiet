import { strict as assert } from "node:assert";
import { test } from "node:test";

import type { Profile } from "./bridge.ts";
import {
  MOST_PROGRAMS,
  addKeepAlive,
  addProgram,
  addProcess,
  addService,
  handlingOf,
  normalizeName,
  rebase,
  removeKeepAlive,
  removeProcess,
  removeService,
  sameProfile,
  setProcess,
  setService,
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
