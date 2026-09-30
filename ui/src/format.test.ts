import { strict as assert } from "node:assert";
import { test } from "node:test";

import {
  canCheckForUpdates,
  closeHint,
  formatBytes,
  formatCoreShare,
  formatPercent,
  formatSince,
  homePlan,
  memoryFreed,
  reportLines,
  summaryLines,
  updateLine,
} from "./format.ts";
import type { Profile, RunReport, UpdateStatus } from "./bridge.ts";

test("bytes scale with one decimal below 100 and none above", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(1536), "1.5 KB");
  assert.equal(formatBytes(1024 * 1024 * 210), "210 MB");
  assert.equal(formatBytes(1024 ** 3 * 1.25), "1.3 GB");
  assert.equal(formatBytes(-1), "—");
  assert.equal(formatBytes(Number.NaN), "—");
});

test("percentages are clamped and rounded", () => {
  assert.equal(formatPercent(23.4), "23%");
  assert.equal(formatPercent(140), "100%");
  assert.equal(formatPercent(-3), "0%");
  assert.equal(formatPercent(Number.NaN), "—");
});

test("elapsed time reads naturally at every scale", () => {
  assert.equal(formatSince(100, 103), "3 s");
  assert.equal(formatSince(0, 12 * 60), "12 min");
  assert.equal(formatSince(0, 65 * 60), "1 h 5 min");
  assert.equal(formatSince(0, 2 * 3600), "2 h");
  assert.equal(formatSince(0, 27 * 3600), "1 d 3 h");
  assert.equal(
    formatSince(500, 100),
    "0 s",
    "a clock that went backwards is not negative",
  );
});

test("summary lines only mention what happened", () => {
  assert.deepEqual(
    summaryLines({
      services_stopped: 0,
      processes_suspended: 0,
      processes_closed: 0,
      power_changed: false,
      memory_purged: false,
    }),
    [],
  );
  assert.deepEqual(
    summaryLines({
      services_stopped: 1,
      processes_suspended: 3,
      processes_closed: 1,
      power_changed: true,
      memory_purged: true,
    }),
    [
      "1 service stopped",
      "3 processes suspended",
      "1 process closed (relaunched on restore)",
      "Performance power plan active",
      "Cached memory purged",
    ],
  );
});

function profile(partial: Partial<Profile> = {}): Profile {
  return {
    processes: [],
    services: [],
    power: "leave",
    purge_memory: false,
    keep_alive: [],
    ...partial,
  };
}

test("a program's CPU is a share of one core and may pass 100", () => {
  assert.equal(formatCoreShare(0), "0%");
  assert.equal(formatCoreShare(46.6), "47%");
  assert.equal(
    formatCoreShare(263.4),
    "263%",
    "eight helpers on a third of a core each are not one full core",
  );
  assert.equal(formatCoreShare(-3), "0%");
  assert.equal(formatCoreShare(Number.NaN), "—");
});

test("the home plan counts only what the button will actually touch", () => {
  assert.equal(
    homePlan(false, profile()),
    "Nothing is selected yet. Open Park list and tick what this button should touch.",
  );
  assert.equal(
    homePlan(
      false,
      profile({
        processes: [
          { name: "OneDrive.exe", action: "suspend", enabled: true },
          { name: "Dropbox.exe", action: "close", enabled: false },
        ],
        services: [{ name: "SysMain", enabled: true }],
        power: "performance",
        purge_memory: true,
      }),
    ),
    "One press will park 1 program, stop 1 service, switch to the performance power plan, and purge cached memory. Press again to undo it.",
  );
  assert.match(homePlan(true, profile()), /undo them/);
});

test("the home plan says what a quick scan adds and what is unsaved", () => {
  const scanning = { autoScan: true, unsaved: false };
  assert.equal(
    homePlan(
      false,
      profile({
        processes: [{ name: "OneDrive.exe", action: "suspend", enabled: true }],
      }),
      scanning,
    ),
    "One press will park 1 program, plus any low-risk programs and services a quick scan finds. Press again to undo it.",
  );
  assert.match(
    homePlan(false, profile(), scanning),
    /^Your park list is empty, so one press will park only the low-risk/,
    "an empty list is not 'nothing' while the scan is on",
  );
  const unsaved = { autoScan: false, unsaved: true };
  assert.match(
    homePlan(false, profile(), unsaved),
    /Unsaved Park list changes are not used yet\.$/,
  );
  assert.match(
    homePlan(true, profile(), unsaved),
    /Unsaved Park list changes are not used yet\.$/,
  );
  assert.doesNotMatch(homePlan(false, profile()), /Unsaved/);
});

test("the close hint follows the tray setting", () => {
  assert.match(closeHint(true), /leaves CompuQuiet in the tray/);
  assert.match(closeHint(false), /quits CompuQuiet/);
  assert.doesNotMatch(closeHint(false), /in the tray\. Right-click/);
});

test("every update state has a sentence, and only some allow a check", () => {
  const states: [UpdateStatus, RegExp, boolean][] = [
    [{ kind: "unavailable", reason: "A development build." }, /^A dev/, false],
    [{ kind: "idle" }, /now and then/, true],
    [{ kind: "checking" }, /Looking/, false],
    [{ kind: "up_to_date" }, /latest release/, true],
    [{ kind: "downloading", version: "1.2.0" }, /Downloading 1\.2\.0/, false],
    [
      { kind: "ready", version: "1.2.0", asks_permission: false },
      /1\.2\.0 is downloaded.*close the window to the tray\.$/,
      false,
    ],
    [{ kind: "failed", error: "offline" }, /failed \(offline\)/, true],
  ];
  for (const [status, expected, canCheck] of states) {
    assert.match(updateLine(status), expected, status.kind);
    assert.equal(canCheckForUpdates(status), canCheck, status.kind);
  }
  assert.match(
    updateLine({ kind: "ready", version: "1.2.0", asks_permission: true }),
    /Windows will ask for permission when it installs\.$/,
  );
});

const MIB = 1024 * 1024;

function measured(over: Partial<RunReport>): RunReport {
  return {
    suspended_bytes: 850 * MIB,
    closed_bytes: 180 * MIB,
    available_before: 5000 * MIB,
    available_after: 5180 * MIB,
    cpu_before: 22.4,
    cpu_after: 6.2,
    ...over,
  };
}

test("what a run freed is the measured rise in available memory, never negative", () => {
  assert.equal(memoryFreed(null), null);
  assert.equal(memoryFreed(measured({})), 180 * MIB);
  // Memory moves for other reasons too; a fall is not "negative freed".
  assert.equal(memoryFreed(measured({ available_after: 4000 * MIB })), 0);
});

test("the report keeps what parking holds apart from what came back", () => {
  assert.deepEqual(reportLines(null), []);
  const lines = reportLines(measured({}));
  assert.deepEqual(lines, [
    "Memory available 4.9 GB to 5.1 GB (approximate)",
    "CPU 22% to 6%",
    "Suspended programs still hold 850 MB: only closing one frees its memory",
    "Closed programs held 180 MB",
  ]);
  // Nothing suspended or closed: only the measurements remain.
  assert.equal(
    reportLines(measured({ suspended_bytes: 0, closed_bytes: 0 })).length,
    2,
  );
});
