/**
 * A clean exit: quitting while quiet offers to restore first, does so, and
 * the process is gone with no journal left behind.
 */
import { strict as assert } from "node:assert";

import { application } from "../wdio.conf.ts";
import { appPids } from "../workspace.ts";
import {
  clickDialogButton,
  clickTab,
  readJson,
  text,
  waitForPill,
} from "./support.ts";

describe("quitting", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await clickTab("dashboard");
  });

  it("restores before exiting when asked, and the process ends", async () => {
    await $("#toggle").click();
    await waitForPill("Quiet");
    assert.ok(readJson("journal.json"), "the journal exists while quiet");
    const pids = appPids(application);
    assert.ok(
      pids.length > 0,
      "the app process was not found by executable path",
    );

    await clickTab("settings");
    await $("#quit").click();
    await clickDialogButton("Restore and quit");

    const deadline = Date.now() + 30_000;
    while (Date.now() < deadline && appPids(application).length > 0) {
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
    assert.deepEqual(appPids(application), [], "the app is still running");
    assert.equal(
      readJson("journal.json"),
      undefined,
      "restore-before-quit left a journal",
    );

    // A fresh session so the runner's teardown has something to close.
    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
    assert.equal(await text("#status-pill"), "Ready");
  });

  it("exits when the tray Quit action runs while idle", async () => {
    const pids = appPids(application);
    assert.ok(pids.length > 0, "the app process was not found");

    // Hidden on purpose (not a user control); WebDriver refuses .click() on it.
    await browser.execute(() => {
      document.getElementById("e2e-tray-quit")?.click();
    });

    const deadline = Date.now() + 30_000;
    while (Date.now() < deadline && appPids(application).length > 0) {
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
    assert.deepEqual(
      appPids(application),
      [],
      "tray Quit left the app running",
    );

    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
    assert.equal(await text("#status-pill"), "Ready");
  });
});
