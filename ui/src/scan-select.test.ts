import { strict as assert } from "node:assert";
import { test } from "node:test";

import type { Recommendation } from "./bridge.ts";
import {
  carryClosing,
  carrySelection,
  chosenKind,
  defaultSelection,
  kindLabel,
  riskyCloses,
  rowKey,
  selectedItems,
  summarize,
  tickLabel,
  withChoices,
} from "./scan-select.ts";

function item(
  name: string,
  risk: "low" | "medium",
  targeted = false,
  memory = 100,
): Recommendation {
  return {
    kind: { kind: "process", action: "suspend" },
    name,
    reason: "test",
    risk,
    memory_bytes: memory,
    cpu_percent: 0,
    instances: 1,
    already_targeted: targeted,
  };
}

test("low-risk finds start ticked, medium and already-targeted do not", () => {
  const items = [
    item("OneDrive", "low"),
    item("Discord", "medium"),
    item("Slack", "low", true),
  ];
  assert.deepEqual([...defaultSelection(items)], [rowKey(items[0]!)]);
  const picked = selectedItems(items, new Set(items.map(rowKey)));
  assert.deepEqual(
    picked.map((p) => p.name),
    ["OneDrive", "Discord"],
    "an already-targeted row cannot be applied even if ticked",
  );
});

test("row labels name the action and the summary counts only what can be added", () => {
  assert.equal(
    kindLabel({ kind: "process", action: "close" }),
    "Close & relaunch",
  );
  assert.equal(kindLabel({ kind: "process", action: "suspend" }), "Suspend");
  assert.equal(kindLabel({ kind: "service" }), "Stop service");
  assert.equal(kindLabel({ kind: "power_plan" }), "Power plan");
  assert.equal(kindLabel({ kind: "memory_purge" }), "Purge cache");

  const summary = summarize([
    item("A", "low", false, 10),
    item("B", "medium", false, 20),
    item("C", "low", true, 40),
    {
      ...item("Cached memory", "low", false, 999),
      kind: { kind: "memory_purge" },
    },
  ]);
  assert.deepEqual(summary, {
    total: 4,
    selectable: 3,
    low: 2,
    medium: 1,
    targeted: 1,
    memoryBytes: 30,
  });
});

test("a rescan keeps what the user ticked and unticked, and starts new finds as usual", () => {
  const before = [
    item("OneDrive", "low"),
    item("Discord", "medium"),
    item("Dropbox", "low"),
  ];
  // The user ticked the medium find and unticked a low one.
  const chosen = new Set([rowKey(before[0]!), rowKey(before[1]!)]);
  const after = [
    item("Zoom", "low"),
    item("Dropbox", "low"),
    item("Discord", "medium"),
    item("OneDrive", "low"),
    item("Steam", "medium"),
  ];
  const next = carrySelection(before, chosen, after);
  assert.deepEqual(
    after.filter((found) => next.has(rowKey(found))).map((found) => found.name),
    ["Zoom", "Discord", "OneDrive"],
    "rows move but their ticks follow the name; a new low find is ticked",
  );
});

test("a find that became a target is never ticked, and one that stopped being a target starts as usual", () => {
  const before = [item("OneDrive", "low"), item("Slack", "low", true)];
  const after = [item("OneDrive", "low", true), item("Slack", "low")];
  const next = carrySelection(before, new Set([rowKey(before[0]!)]), after);
  assert.deepEqual([...next], [rowKey(after[1]!)]);
});

test("a tick box is named for what ticking it does", () => {
  assert.equal(tickLabel(item("OneDrive.exe", "low")), "Suspend OneDrive.exe");
  assert.equal(
    tickLabel({ ...item("WSearch", "low"), kind: { kind: "service" } }),
    "Stop service WSearch",
  );
  assert.equal(
    tickLabel({
      ...item("Power plan: High performance", "low"),
      kind: { kind: "power_plan" },
    }),
    "Add Power plan: High performance",
  );
});

test("a program marked close instead is applied as a close and nothing else changes", () => {
  const items = [
    item("OneDrive", "low"),
    item("Slack", "low"),
    { ...item("WSearch", "low"), kind: { kind: "service" } as const },
  ];
  const closing = new Set([rowKey(items[0]!), rowKey(items[2]!)]);
  assert.deepEqual(chosenKind(items[0]!, closing), {
    kind: "process",
    action: "close",
  });
  assert.equal(kindLabel(chosenKind(items[0]!, closing)), "Close & relaunch");
  assert.deepEqual(
    withChoices(items, closing).map((found) => found.kind),
    [
      { kind: "process", action: "close" },
      { kind: "process", action: "suspend" },
      { kind: "service" },
    ],
    "a service has no close, whatever is marked",
  );
  assert.equal(
    items[0]!.kind.kind === "process" && items[0]!.kind.action,
    "suspend",
    "the scan's own finds are not changed",
  );
});

test("only a close of a medium-risk program asks for a second look", () => {
  const items = withChoices(
    [
      item("Discord", "medium"),
      item("OneDrive", "low"),
      item("Zoom", "medium"),
    ],
    new Set([
      rowKey(item("Discord", "medium")),
      rowKey(item("OneDrive", "low")),
    ]),
  );
  assert.deepEqual(
    riskyCloses(items).map((found) => found.name),
    ["Discord"],
    "OneDrive closes without asking, Zoom is only suspended",
  );
});

test("a mark follows its find to the next scan and goes when the find does", () => {
  const before = new Set([
    rowKey(item("Discord", "medium")),
    rowKey(item("Slack", "low")),
  ]);
  const after = [item("Discord", "medium"), item("Slack", "low", true)];
  assert.deepEqual([...carryClosing(before, after)], [rowKey(after[0]!)]);
  assert.equal(carryClosing(before, []).size, 0);
});
