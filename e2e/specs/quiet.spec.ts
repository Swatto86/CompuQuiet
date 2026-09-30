/**
 * The primary workflow, end to end against the fake machine: Quiet Mode on
 * writes an undo journal and reports what it did; Restore replays it and
 * removes the journal.
 */
import { strict as assert } from "node:assert";

import { clickTab, readJson, screenshot, waitForPill } from "./support.ts";

interface Journal {
  version: number;
  started_at: number;
  done: { kind: string; name?: string }[];
}

describe("Quiet Mode", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await clickTab("dashboard");
  });

  it("parks the seeded targets and journals every step to disk", async () => {
    await $("#toggle").click();
    await waitForPill("Quiet");

    assert.equal(await $("#toggle").getAttribute("data-quiet"), "true");
    assert.equal(await $("#hero-title").getText(), "Quiet Mode is on");

    const journal = readJson<Journal>("journal.json");
    assert.ok(journal, "journal.json was not written");
    const kinds = journal.done.map(
      (step) => `${step.kind}${step.name ? `:${step.name}` : ""}`,
    );
    assert.deepEqual(kinds, [
      "power_plan_changed",
      "service_stopped:SysMain",
      "process_suspended:OneDrive.exe",
      "process_closed:Dropbox.exe",
      "process_suspended:Slack.exe",
      "memory_purged",
    ]);

    const summary = await $("#summary").getText();
    assert.match(summary, /1 service stopped/);
    assert.match(summary, /2 processes suspended/);
    assert.match(summary, /1 process closed/);
    assert.match(summary, /Performance power plan active/);
    assert.match(summary, /Cached memory purged/);

    // Measured by the engine around the run. Suspending OneDrive (210 MB) and
    // Slack (640 MB) keeps their memory; closing Dropbox (180 MB) gave its
    // back, and only that shows in what came free.
    assert.match(summary, /Suspended programs still hold 850 MB/);
    assert.match(summary, /Closed programs held 180 MB/);
    assert.match(summary, /Memory available [\d.]+ GB to [\d.]+ GB/);
    assert.match(summary, /CPU 23% to 4%/);
    assert.equal(await $("#freed-value").getText(), "180 MB");

    const skipped = await $("#skipped").getText();
    assert.match(skipped, /DiagTrack — already stopped/);
    assert.match(skipped, /NoSuchService — not installed/);
    assert.match(skipped, /NotRunningApp — not running/);
    assert.doesNotMatch(
      skipped,
      /Spooler/,
      "a disabled target is not mentioned",
    );

    const log = await $$("#log li");
    assert.equal(log.length, 6);
    for (const line of log) {
      assert.ok(
        !(await line.getAttribute("class"))?.includes("failed"),
        await line.getText(),
      );
    }
    await screenshot("quiet");
  });

  it("restores everything in reverse and removes the journal", async () => {
    await $("#toggle").click();
    await waitForPill("Ready");

    assert.equal(
      readJson("journal.json"),
      undefined,
      "journal.json should be gone",
    );
    assert.equal(
      await $("#hero-title").getText(),
      "Ready for a game or local AI",
    );
    const lines = await $$("#log li").map((line) => line.getText());
    assert.deepEqual(lines, [
      "Resume Slack.exe (PID 103)",
      "Relaunch Dropbox.exe",
      "Resume OneDrive.exe (PID 100)",
      "Start service SysMain",
      "Restore the Balanced power plan",
    ]);
    assert.equal(await $("#summary").getText(), "Nothing yet.");
  });
});
