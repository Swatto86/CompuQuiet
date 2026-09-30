import assert from "node:assert/strict";
import test from "node:test";

import type { Preview, PreviewItem } from "./bridge.ts";
import { fromScan, itemLine, leftAlone } from "./preview-text.ts";

const MIB = 1024 * 1024;

function item(over: Partial<PreviewItem>): PreviewItem {
  return {
    action: "suspend",
    name: "Slack.exe",
    processes: 1,
    memory_bytes: 640 * MIB,
    relaunch: null,
    ...over,
  };
}

test("a step reads as what it does, with how many processes and how much they hold", () => {
  assert.deepEqual(itemLine(item({})), {
    text: "Suspend Slack.exe (1 process, 640 MB)",
    command: null,
  });
  assert.equal(
    itemLine(item({ processes: 3, memory_bytes: 1536 * MIB })).text,
    "Suspend Slack.exe (3 processes, 1.5 GB)",
  );
  assert.equal(
    itemLine(item({ action: "stop_service", name: "SysMain", processes: 0 }))
      .text,
    "Stop service SysMain",
  );
  assert.equal(
    itemLine(item({ action: "power", name: "", processes: 0 })).text,
    "Switch to the performance power plan",
  );
  assert.equal(
    itemLine(item({ action: "purge", name: "", processes: 0 })).text,
    "Purge cached memory",
  );
});

test("a closed program shows the command line it is opened with, or says it cannot be", () => {
  const closed = itemLine(
    item({
      action: "close",
      name: "Dropbox.exe",
      relaunch: 'C:/Apps/Dropbox.exe --profile "my work"',
    }),
  );
  assert.match(closed.text, /^Close Dropbox\.exe \(1 process, 640 MB\)\./);
  assert.equal(closed.command, 'C:/Apps/Dropbox.exe --profile "my work"');

  const unknown = itemLine(item({ action: "close", relaunch: null }));
  assert.equal(unknown.command, null);
  assert.match(unknown.text, /path could not be read/);
});

test("what is left alone is grouped by reason, in the order first met", () => {
  assert.deepEqual(
    leftAlone([
      { name: "Dropbox", reason: "not running" },
      { name: "Spooler", reason: "protected: essential to the system" },
      { name: "Slack", reason: "not running" },
      { name: "Power plan", reason: "on battery, so it is skipped" },
    ]),
    [
      "Dropbox, Slack — not running",
      "Spooler — protected: essential to the system",
      "Power plan — on battery, so it is skipped",
    ],
  );
  assert.deepEqual(leftAlone([]), []);
});

test("the scan's additions are named, and said to be for this press only", () => {
  const preview: Preview = {
    items: [],
    skipped: [],
    from_scan: ["Steam", "Teams"],
    taken_at: 0,
  };
  assert.match(fromScan(preview) ?? "", /Steam, Teams .* this press/);
  assert.equal(fromScan({ ...preview, from_scan: [] }), null);
});
