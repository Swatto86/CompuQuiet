/**
 * The Home preview lists what one press would do on the machine as it is, and
 * what it would leave alone, without changing anything. On battery the power
 * plan and the memory purge are left out, in the preview and in the run,
 * unless Settings allows them.
 *
 * Runs after quiet.spec, so the fake Dropbox is the one Restore relaunched:
 * its size is matched by shape, not by value.
 */
import { strict as assert } from "node:assert";

import {
  clickTab,
  fakeBattery,
  readJson,
  screenshot,
  text,
  texts,
  waitForPill,
} from "./support.ts";

interface Journal {
  done: { kind: string; name?: string }[];
}

interface SavedSettings {
  allow_on_battery: boolean;
}

const STEPS = "#preview-steps li";
const ON_BATTERY =
  "on battery, so it is skipped to save charge (Settings can allow it)";

async function isOpen(): Promise<boolean> {
  return browser.execute(
    () => (document.getElementById("preview") as HTMLDetailsElement).open,
  );
}

async function waitForSteps(count: number): Promise<string[]> {
  try {
    await browser.waitUntil(async () => (await texts(STEPS)).length === count, {
      timeout: 10_000,
    });
  } catch (cause) {
    const listed = await texts(STEPS);
    throw new Error(
      `the preview never listed ${count} steps; it lists: ${listed.join(" | ")}`,
      { cause },
    );
  }
  return texts(STEPS);
}

async function setBattery(on: boolean): Promise<void> {
  await clickTab("settings");
  if ((await $("#set-battery").isSelected()) !== on)
    await $("#set-battery").click();
  await browser.waitUntil(
    async () =>
      readJson<SavedSettings>("settings.json")?.allow_on_battery === on,
    { timeout: 5_000, timeoutMsg: `settings.json did not record ${on}` },
  );
  await clickTab("dashboard");
}

describe("the preview", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await clickTab("dashboard");
  });

  after(async () => {
    await fakeBattery(null);
    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
  });

  it("stays closed until asked, then lists what one press would do and leave alone", async () => {
    assert.equal(await isOpen(), false);
    assert.equal((await texts(STEPS)).length, 0, "nothing is fetched unasked");

    await $("#preview summary").click();
    const steps = await waitForSteps(6);
    assert.equal(steps[0], "Switch to the performance power plan");
    assert.equal(steps[1], "Stop service SysMain");
    assert.equal(steps[2], "Suspend OneDrive.exe (1 process, 210 MB)");
    assert.match(
      steps[3] ?? "",
      /^Close Dropbox\.exe \(1 process, [\d.]+ MB\)\. Restore opens it again with C:\/fake\/Dropbox\.exe --background$/,
    );
    assert.equal(steps[4], "Suspend Slack.exe (1 process, 640 MB)");
    assert.equal(steps[5], "Purge cached memory");

    assert.deepEqual(await texts("#preview-left li"), [
      "DiagTrack — already stopped",
      "NoSuchService — not installed",
      "NotRunningApp — not running",
    ]);
    assert.match(
      await text("#preview-status"),
      /^As of .*Looking changes nothing/,
    );
    await screenshot("preview");
  });

  it("changes nothing: no run, no journal, no program touched", async () => {
    assert.equal(readJson("journal.json"), undefined);
    assert.equal(await text("#status-pill"), "Ready");
    assert.equal(await text("#hero-title"), "Ready for a game or local AI");
  });

  it("holds the power plan and the purge back on battery, and Settings allows them", async () => {
    await fakeBattery(true);
    // Returning to Home looks again while the preview is open.
    await clickTab("settings");
    await clickTab("dashboard");
    const steps = await waitForSteps(4);
    assert.ok(
      !steps.some((step) => /power plan|Purge/.test(step)),
      steps.join(" | "),
    );
    const left = await texts("#preview-left li");
    assert.ok(
      left.includes(`Power plan, Memory purge — ${ON_BATTERY}`),
      left.join(" | "),
    );

    await setBattery(true);
    const allowed = await waitForSteps(6);
    assert.equal(allowed[0], "Switch to the performance power plan");
    await setBattery(false);
    await waitForSteps(4);
  });

  it("does not run them on battery either, and says so", async () => {
    await $("#toggle").click();
    await waitForPill("Quiet");
    const journal = readJson<Journal>("journal.json");
    assert.ok(journal, "journal.json was not written");
    assert.deepEqual(
      journal.done.map(
        (step) => `${step.kind}${step.name ? `:${step.name}` : ""}`,
      ),
      [
        "service_stopped:SysMain",
        "process_suspended:OneDrive.exe",
        "process_closed:Dropbox.exe",
        "process_suspended:Slack.exe",
      ],
    );
    assert.doesNotMatch(await text("#summary"), /power plan|purged/i);
    const skipped = await text("#skipped");
    assert.ok(
      skipped.includes(`Power plan — ${ON_BATTERY}`),
      `the left-alone list: ${skipped}`,
    );
    assert.equal(await isOpen(), false, "the preview is for before a run");

    await $("#toggle").click();
    await waitForPill("Ready");
  });

  it("runs the whole plan again on mains", async () => {
    await fakeBattery(false);
    await $("#preview summary").click();
    assert.equal((await waitForSteps(6)).length, 6);
  });
});
