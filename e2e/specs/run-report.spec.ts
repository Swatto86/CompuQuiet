/**
 * The figures for what a run did are measured by the engine, so a run that
 * begins in the tray, with the window hidden and nothing polling, still
 * reports them, and parked memory is kept apart from what came back.
 *
 * Runs after quiet.spec, which pins the exact figures on the untouched fake
 * machine; here Dropbox is the copy Restore relaunched, so its size is
 * matched by shape.
 */
import { strict as assert } from "node:assert";

import {
  clickTab,
  setWindowVisible,
  text,
  trayMenu,
  waitForPill,
} from "./support.ts";

describe("the run report", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await clickTab("dashboard");
  });

  it("shows nothing measured until a run has happened", async () => {
    assert.equal(await text("#freed-value"), "—");
    assert.equal(await text("#summary"), "Nothing yet.");
  });

  it("is measured for a run started from the tray while the window is hidden", async () => {
    await setWindowVisible(false);
    await trayMenu("tray-toggle");
    await waitForPill("Quiet");
    await setWindowVisible(true);

    const summary = await text("#summary");
    assert.match(
      summary,
      /Memory available [\d.]+ GB to [\d.]+ GB \(approximate\)/,
    );
    assert.match(summary, /CPU 23% to 4%/);
    // Freezing keeps memory: OneDrive and Slack still hold theirs.
    assert.match(
      summary,
      /Suspended programs still hold 850 MB: only closing one frees its memory/,
    );
    assert.match(summary, /Closed programs held [\d.]+ MB/);
    // What came back is what closing Dropbox gave, not what was parked.
    assert.match(await text("#freed-value"), /^[\d.]+ MB$/);
  });

  it("goes with Quiet Mode", async () => {
    await trayMenu("tray-toggle");
    await waitForPill("Ready");
    assert.equal(await text("#freed-value"), "—");
    assert.equal(await text("#summary"), "Nothing yet.");
  });
});
