import { strict as assert } from "node:assert";
import { test } from "node:test";

import {
  formatBytes,
  formatPercent,
  formatSince,
  homePlan,
  summaryLines,
} from "./format.ts";
import type { Profile } from "./bridge.ts";

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
