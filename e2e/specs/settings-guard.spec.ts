/**
 * A settings.json the app cannot read is kept, not replaced: nothing saves and
 * Quiet Mode stays off behind a banner that cannot be dismissed, until the
 * file is set aside. A fresh start then saves defaults that leave the memory
 * purge off.
 */
import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";

import { choose } from "./choose.ts";
import { clickTab, dataDir, text } from "./support.ts";

/** Cut off mid-value, as an interrupted hand edit would leave it. */
const BROKEN = '{ "version": 1, "profile": ';

describe("an unreadable settings file", () => {
  const settingsFile = () => path.join(dataDir(), "settings.json");
  const journalFile = () => path.join(dataDir(), "journal.json");
  let original: string;

  before(async () => {
    original = fs.readFileSync(settingsFile(), "utf8");
    fs.writeFileSync(settingsFile(), BROKEN);
    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
  });

  after(async () => {
    // The specs after this one expect the seeded targets back.
    fs.writeFileSync(settingsFile(), original);
    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
  });

  it("says so in a banner that cannot be dismissed", async () => {
    await $("#banner").waitForDisplayed({ timeout: 10_000 });
    assert.match(await text("#banner-text"), /could not read its settings/);
    assert.equal(await $("#banner-dismiss").isDisplayed(), false);
    assert.equal(await text("#banner-action"), "Set the file aside");
  });

  it("refuses a change and Quiet Mode, and leaves the file as it was", async () => {
    await clickTab("settings");
    await choose("#set-theme", "dark");
    await $("#toast").waitForDisplayed({ timeout: 5_000 });
    assert.match(await text("#toast"), /nothing is saved/);
    assert.equal(fs.readFileSync(settingsFile(), "utf8"), BROKEN);

    // The refusal above is still on screen; wait for the next one.
    await browser.execute(() => {
      document.getElementById("toast")!.hidden = true;
    });
    await clickTab("dashboard");
    await $("#toggle").click();
    await $("#toast").waitForDisplayed({ timeout: 5_000 });
    assert.match(await text("#toast"), /Quiet Mode stays off/);
    assert.equal(await text("#status-pill"), "Ready");
    assert.equal(fs.existsSync(journalFile()), false, "nothing was parked");
    assert.equal(fs.readFileSync(settingsFile(), "utf8"), BROKEN);
    assert.equal(await $("#banner").isDisplayed(), true);
  });

  it("sets the file aside on request and then saves normally", async () => {
    await $("#banner-action").click();
    await $("#banner").waitForDisplayed({ reverse: true, timeout: 5_000 });
    assert.equal(fs.readFileSync(`${settingsFile()}.bad`, "utf8"), BROKEN);
    assert.equal(fs.existsSync(settingsFile()), false, "absent means defaults");

    await clickTab("settings");
    await choose("#set-theme", "light");
    await browser.waitUntil(async () => fs.existsSync(settingsFile()), {
      timeout: 5_000,
      timeoutMsg: "the change was not saved once the file was set aside",
    });
    const saved = JSON.parse(fs.readFileSync(settingsFile(), "utf8")) as {
      theme: string;
      profile: { purge_memory: boolean };
    };
    assert.equal(saved.theme, "light");
    assert.equal(saved.profile.purge_memory, false, "the purge is opt-in");
    assert.equal(fs.readFileSync(`${settingsFile()}.bad`, "utf8"), BROKEN);
  });
});
