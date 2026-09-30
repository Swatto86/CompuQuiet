import { strict as assert } from "node:assert";
import { test } from "node:test";

import type { EngineState } from "./bridge.ts";
import { recoveryConfirmation, recoveryNotice } from "./recovery-text.ts";

function state(over: Partial<EngineState>): EngineState {
  return {
    quiet: true,
    busy: false,
    started_at: 1,
    summary: {
      services_stopped: 0,
      processes_suspended: 0,
      processes_closed: 0,
      power_changed: false,
      memory_purged: false,
    },
    run_report: null,
    skipped: [],
    log: [],
    capabilities: {
      services: true,
      power: true,
      memory_purge: true,
      elevated: true,
      can_elevate: false,
    },
    data_dir: "C:/data",
    os: "windows",
    recovered: false,
    startup_error: null,
    settings_unreadable: null,
    unrestored: [],
    ...over,
  };
}

const stuck = state({
  unrestored: [
    {
      label: "Start service WSearch",
      consequence: "WSearch stays stopped until you start it or restart the PC",
      error: "starting service WSearch: The service cannot be started",
    },
    {
      label: "Relaunch Dropbox",
      consequence: "Dropbox stays closed until you open it yourself",
      error: null,
    },
  ],
});

test("nothing is said while nothing is stuck", () => {
  assert.equal(recoveryNotice(state({})), null);
});

test("a failed restore lists each entry with its reason", () => {
  const notice = recoveryNotice(stuck);
  assert.equal(notice?.kind, "stuck");
  assert.deepEqual(notice?.items, [
    "Start service WSearch: starting service WSearch: The service cannot be started",
    "Relaunch Dropbox",
  ]);
});

test("an unreadable record is offered for setting aside, after failed entries", () => {
  const broken = state({ startup_error: "expected value at line 1" });
  const notice = recoveryNotice(broken);
  assert.equal(notice?.kind, "journal");
  assert.deepEqual(notice?.items, ["expected value at line 1"]);
  assert.equal(recoveryNotice({ ...stuck, startup_error: "x" })?.kind, "stuck");
});

test("the confirmation names every entry and what it leaves behind", () => {
  const confirmation = recoveryConfirmation("stuck", stuck);
  for (const entry of stuck.unrestored) {
    assert.ok(confirmation.body.includes(entry.label), entry.label);
    assert.ok(confirmation.body.includes(entry.consequence), entry.label);
  }
  assert.match(confirmation.body, /journal\.json\.bad/);
  assert.equal(confirmation.confirm, "Give up on these");
  assert.match(
    recoveryConfirmation("journal", state({})).body,
    /stays that way/,
  );
});
