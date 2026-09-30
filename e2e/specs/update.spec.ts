/**
 * A development build never updates itself, and says so. This drives the two
 * commands the window uses for it through the real IPC boundary; the network
 * side is not exercised, because a build that could call out would reach the
 * real release server.
 */
import { strict as assert } from "node:assert";

import { checkForUpdates, clickTab, text, updateStatus } from "./support.ts";

describe("self-updating in a development build", () => {
  it("reports that this copy cannot update itself, and why", async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    const status = await updateStatus();
    assert.equal(status.kind, "unavailable");
    assert.match(status.reason ?? "", /development build/);
  });

  it("stays unavailable when a check is asked for", async () => {
    const status = await checkForUpdates();
    assert.equal(status.kind, "unavailable");
    assert.deepEqual(await updateStatus(), status);
  });

  it("says so under About, and offers no check that could not run", async () => {
    await clickTab("settings");
    await browser.waitUntil(
      async () => /development build/.test(await text("#about-update")),
      { timeout: 5_000, timeoutMsg: "About never showed the update status" },
    );
    assert.equal(await $("#update-check").isEnabled(), false);
    await clickTab("dashboard");
  });
});
