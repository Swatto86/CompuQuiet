/**
 * Keyboard and screen-reader use of the tab bar and the Park list: focus stays
 * on the control that was just used, unsaved edits show where they matter, and
 * every row's control is named for its row.
 */
import { strict as assert } from "node:assert";

import {
  attribute,
  clickTab,
  focus,
  focused,
  text,
  watchInvokes,
} from "./support.ts";

describe("the window, by keyboard", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await watchInvokes();
    // Keys go to the window that has the operating system's focus; a click
    // gives it that, and the first spec of a session has clicked nothing yet.
    await $("#hero-title").click();
  });

  after(async () => {
    // The Park list edits made here were never saved, and the next spec shares
    // this window: start it from a fresh one rather than on top of them.
    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
  });

  describe("the tabs", () => {
    it("move focus with the arrow keys and open only on Enter", async () => {
      await focus("#tab-dashboard");
      await browser.keys("ArrowRight");
      assert.equal(await focused(), "tab-scan");
      assert.equal(await $("#view-dashboard").isDisplayed(), true);
      assert.equal(await attribute("#tab-scan", "tabindex"), "0");
      assert.equal(await attribute("#tab-dashboard", "tabindex"), "-1");
      assert.equal(await attribute("#tab-scan", "aria-controls"), "view-scan");
      await browser.keys("Enter");
      await $("#view-scan").waitForDisplayed({ timeout: 5_000 });
      assert.equal(await attribute("#tab-scan", "aria-selected"), "true");

      await browser.keys("ArrowLeft");
      assert.equal(await focused(), "tab-dashboard");
      await browser.keys("End");
      assert.equal(await focused(), "tab-settings");
      await clickTab("dashboard");
    });

    it("leave the big button's name to its label, not to a pressed state", async () => {
      assert.equal(await attribute("#toggle", "aria-pressed"), "");
      assert.equal(await text("#power-label"), "Free up this PC");
    });
  });

  describe("failure text", () => {
    it("can be selected and copied", async () => {
      const selectable = await browser.execute(() =>
        ["#log", "#skipped", "#toast"].map(
          (sel) =>
            getComputedStyle(document.querySelector(sel) as Element).userSelect,
        ),
      );
      assert.deepEqual(selectable, ["text", "text", "text"]);
    });
  });

  describe("the Park list", () => {
    before(async () => {
      await clickTab("targets");
    });

    it("keeps focus on a row's box that Space ticks, and shows unsaved edits where they matter", async () => {
      const box = 'input[aria-label="Enable Slack"]';
      const was = await $(box).isSelected();
      await focus(box);
      await browser.keys("Space");
      assert.equal(await $(box).isSelected(), !was);
      assert.equal(await focused(), "Enable Slack");
      assert.equal(await text("#targets-status"), "Unsaved changes");
      assert.match(
        (await $("#tab-targets").getAttribute("class")) ?? "",
        /unsaved/,
      );
      assert.equal(
        await attribute("#tab-targets", "aria-label"),
        "Park list, unsaved changes",
      );

      await clickTab("dashboard");
      assert.match(
        await text("#hero-plan"),
        /Unsaved Park list changes are not used yet\.$/,
      );
      await clickTab("targets");

      await focus(box);
      await browser.keys("Space");
      assert.equal(await $(box).isSelected(), was);
      assert.equal(await text("#targets-status"), "");
      assert.doesNotMatch(
        (await $("#tab-targets").getAttribute("class")) ?? "",
        /unsaved/,
      );
      await clickTab("dashboard");
      assert.doesNotMatch(await text("#hero-plan"), /Unsaved/);
      await clickTab("targets");
    });

    it("keeps focus on an Action select that the arrow keys change", async () => {
      const select = 'select[aria-label="Action for Slack"]';
      await focus(select);
      await browser.keys("ArrowDown");
      assert.equal(await $(select).getValue(), "close");
      assert.equal(await focused(), "Action for Slack");
      await browser.keys("ArrowUp");
      assert.equal(await $(select).getValue(), "suspend");
      assert.equal(await focused(), "Action for Slack");
    });

    it("hands focus to the row that takes a removed row's place", async () => {
      await $('button[aria-label="Remove Dropbox"]').click();
      assert.equal(await focused(), "Remove Slack");
      const chip = 'button[aria-label="Stop protecting Dropbox"]';
      assert.ok(
        await $(chip).isExisting(),
        "Dropbox is not listed as protected",
      );
      await $(chip).click();
      assert.equal(await focused(), "Stop protecting game");
    });

    it("brings a new row into view, and says when protection takes a row off the list", async () => {
      await $("#process-name").setValue("zz-new-app.exe");
      await $('#process-add button[type="submit"]').click();
      const fresh = await $("#process-targets tr.fresh");
      await fresh.waitForExist({ timeout: 5_000 });
      assert.match(await fresh.getText(), /zz-new-app\.exe/);
      assert.equal(await fresh.isDisplayed({ withinViewport: true }), true);

      await $("#keep-name").setValue("Slack");
      await $('#keep-add button[type="submit"]').click();
      await $("#toast").waitForDisplayed({ timeout: 5_000 });
      const said = "Slack is protected now, so it was taken off the park list";
      assert.equal(await text("#toast"), said);
      assert.equal(
        await $('input[aria-label="Enable Slack"]').isExisting(),
        false,
      );
      await browser.waitUntil(
        async () => (await text("#announce")).includes(said),
        {
          timeout: 3_000,
          timeoutMsg: "the toast was not announced to a screen reader",
        },
      );
    });

    it("names each row's box and the column of remove buttons", async () => {
      assert.ok(await $('input[aria-label="Enable OneDrive"]').isExisting());
      assert.equal(
        await text("#view-targets th:last-child .sr-only"),
        "Remove",
      );
    });
  });
});
