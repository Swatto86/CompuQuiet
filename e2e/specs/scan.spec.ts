/**
 * The scanner against the fake machine: finds are listed with the right
 * defaults, accepted finds become targets and are parked in the same click,
 * and auto-scan parks low-risk finds without touching the saved targets.
 */
import { strict as assert } from "node:assert";

import {
  clickTab,
  readJson,
  screenshot,
  text,
  waitForPill,
} from "./support.ts";

interface Journal {
  done: { kind: string; name?: string }[];
}

interface SavedSettings {
  auto_scan: boolean;
  profile: { processes: { name: string }[]; services: { name: string }[] };
}

interface Row {
  name: string;
  action: string;
  risk: string;
  checked: boolean;
  disabled: boolean;
}

async function rows(): Promise<Row[]> {
  return $$("#scan-rows tr").map(async (row) => {
    const cells = await row.$$("td").map((cell) => cell.getText());
    const input = row.$("input");
    return {
      name: cells[1] ?? "",
      action: cells[2] ?? "",
      // The chip is uppercased by CSS; compare the words, not the styling.
      risk: (cells[4] ?? "").toLowerCase(),
      checked: await input.isSelected(),
      disabled: !(await input.isEnabled()),
    };
  });
}

function journalKinds(): string[] {
  const journal = readJson<Journal>("journal.json");
  assert.ok(journal, "journal.json was not written");
  return journal.done.map(
    (step) => `${step.kind}${step.name ? `:${step.name}` : ""}`,
  );
}

describe("Scan", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await clickTab("scan");
  });

  it("lists finds with low risk pre-ticked and existing targets greyed out", async () => {
    await browser.waitUntil(
      async () => (await $$("#scan-rows tr")).length > 1,
      {
        timeout: 15_000,
        timeoutMsg: "the scan produced no rows",
      },
    );
    const found = await rows();
    const byName = new Map(found.map((row) => [row.name, row]));

    assert.deepEqual(byName.get("GoogleUpdate.exe"), {
      name: "GoogleUpdate.exe",
      action: "Suspend",
      risk: "low",
      checked: true,
      disabled: false,
    });
    assert.deepEqual(byName.get("render-farm.exe"), {
      name: "render-farm.exe",
      action: "Suspend",
      risk: "medium",
      checked: false,
      disabled: false,
    });
    assert.equal(byName.get("WSearch")?.checked, true);
    assert.equal(
      byName.get("OneDrive.exe")?.disabled,
      true,
      "already a target",
    );
    assert.equal(byName.get("OneDrive.exe")?.action, "already a target");
    assert.equal(
      byName.get("Spooler")?.disabled,
      true,
      "a disabled target still counts",
    );
    assert.ok(
      !byName.has("game.exe"),
      "the foreground game is never suggested",
    );
    assert.ok(!byName.has("explorer.exe"), "the shell is never suggested");
    assert.match(await text("#scan-summary"), /new finds/);
    await screenshot("scan");
  });

  it("adds the ticked finds to the targets and parks them in the same click", async () => {
    await $("#scan-apply-quiet").click();
    await waitForPill("Quiet");
    const kinds = journalKinds();
    assert.ok(
      kinds.includes("process_suspended:GoogleUpdate.exe"),
      kinds.join(", "),
    );
    assert.ok(kinds.includes("service_stopped:WSearch"), kinds.join(", "));
    assert.ok(
      !kinds.some((k) => k.includes("render-farm")),
      "medium risk is not applied unasked",
    );

    const saved = readJson<SavedSettings>("settings.json")!;
    assert.ok(
      saved.profile.processes.some((t) => t.name === "GoogleUpdate.exe"),
    );
    assert.ok(saved.profile.services.some((t) => t.name === "WSearch"));

    await clickTab("dashboard");
    await $("#toggle").click();
    await waitForPill("Ready");
  });

  it("auto-scan parks low-risk finds without changing the saved targets", async () => {
    await clickTab("targets");
    await $('button[aria-label="Remove GoogleUpdate.exe"]').click();
    await $("#targets-save").click();
    await browser.waitUntil(
      async () => (await $("#targets-status").getText()) === "",
      {
        timeout: 5_000,
        timeoutMsg: "the save did not settle",
      },
    );

    await clickTab("settings");
    await $("#set-auto-scan").click();
    await browser.waitUntil(
      async () => readJson<SavedSettings>("settings.json")?.auto_scan === true,
      {
        timeout: 5_000,
        timeoutMsg: "settings.json did not record auto_scan",
      },
    );

    await clickTab("dashboard");
    await $("#toggle").click();
    await waitForPill("Quiet");
    const kinds = journalKinds();
    assert.ok(
      kinds.includes("process_suspended:GoogleUpdate.exe"),
      kinds.join(", "),
    );
    assert.match(await $("#log").getText(), /Scan added/);
    const saved = readJson<SavedSettings>("settings.json")!;
    assert.ok(
      !saved.profile.processes.some((t) => t.name === "GoogleUpdate.exe"),
      "auto-scan must not edit the saved targets",
    );

    await $("#toggle").click();
    await waitForPill("Ready");
    await clickTab("settings");
    await $("#set-auto-scan").click();
    await browser.waitUntil(
      async () => readJson<SavedSettings>("settings.json")?.auto_scan === false,
      {
        timeout: 5_000,
        timeoutMsg: "settings.json did not record auto_scan off",
      },
    );
    await clickTab("dashboard");
  });
});
