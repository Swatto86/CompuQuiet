/**
 * A development build never updates itself, and says so. This drives the two
 * commands the window uses for it through the real IPC boundary; the network
 * side is not exercised, because a build that could call out would reach the
 * real release server. The window's handling of what the backend would report
 * (a release announced, automatic installs switched off) is driven by sending
 * the same event the backend sends.
 */
import { strict as assert } from "node:assert";

import {
  checkForUpdates,
  clickTab,
  emitEvent,
  readJson,
  text,
  updateStatus,
} from "./support.ts";

const autoUpdate = () => readJson<{ auto_update?: boolean }>("settings.json");

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
    assert.match(
      await text("#about-update"),
      /Newer releases are at github\.com\/Swatto86\/CompuQuiet\/releases/,
      "a copy that cannot update itself says where to get the release",
    );
    assert.equal(await $("#update-check").isEnabled(), false);
    await clickTab("dashboard");
  });

  it("says a release is out when one is announced, and a check goes back to the truth", async () => {
    await clickTab("settings");
    await emitEvent("update-status", { kind: "available", version: "9.9.9" });
    await browser.waitUntil(
      async () => /9\.9\.9 is available/.test(await text("#about-update")),
      { timeout: 5_000, timeoutMsg: "About never announced the release" },
    );
    assert.match(await text("#about-update"), /Install updates automatically/);
    assert.equal(await $("#update-check").isEnabled(), true, "can look again");

    await $("#update-check").click();
    await browser.waitUntil(
      async () => /development build/.test(await text("#about-update")),
      { timeout: 5_000, timeoutMsg: "the backend's answer never replaced it" },
    );
    await clickTab("dashboard");
  });

  it("installs updates automatically unless that is switched off, and remembers it", async () => {
    await clickTab("settings");
    const box = $("#set-auto-update");
    assert.equal(
      await box.isSelected(),
      true,
      "on for a settings file written before the switch existed",
    );
    await box.click();
    await browser.waitUntil(async () => autoUpdate()?.auto_update === false, {
      timeout: 5_000,
      timeoutMsg: "settings.json did not record automatic updates off",
    });

    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await clickTab("settings");
    assert.equal(await $("#set-auto-update").isSelected(), false, "kept");

    await $("#set-auto-update").click();
    await browser.waitUntil(async () => autoUpdate()?.auto_update === true, {
      timeout: 5_000,
      timeoutMsg: "settings.json did not record automatic updates back on",
    });
    await clickTab("dashboard");
  });
});
