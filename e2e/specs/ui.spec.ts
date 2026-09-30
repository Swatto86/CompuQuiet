/**
 * How the window behaves for the keyboard and a screen reader, and the small
 * promises the page makes: focus stays where the user put it, a dialog is one
 * dialog and holds everything behind it still, a refused add does not go on to
 * start Quiet Mode, and an idle tray window does not keep sampling the machine.
 * The page's own IPC is counted and made to fail through `watchInvokes`, since
 * the fake machine cannot be made to refuse a settings save.
 */
import { strict as assert } from "node:assert";

import { choose } from "./choose.ts";
import {
  attribute,
  clickTab,
  emitEvent,
  engineState,
  focus,
  focused,
  readJson,
  setWindowVisible,
  text,
  waitForPill,
  windowTheme,
} from "./support.ts";
import { failInvokes, invokeCount, watchInvokes } from "./ipc-watch.ts";

interface SavedSettings {
  close_to_tray: boolean;
  theme: string;
}

/** A scan has finished: its button is back, and there is something to show. */
async function scanned(): Promise<void> {
  await browser.waitUntil(
    async () =>
      (await $("#scan-run").isEnabled()) &&
      (await browser.execute(
        () => document.querySelectorAll("#scan-rows tr").length,
      )) > 1,
    { timeout: 15_000, timeoutMsg: "the scan did not finish" },
  );
}

describe("the window", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await watchInvokes();
  });

  afterEach(async () => {
    // A spec that fails must not leave a command refused for the ones after it.
    await failInvokes("apply_recommendations", null);
    await failInvokes("save_settings", null);
  });

  describe("Scan", () => {
    const medium = 'input[aria-label="Suspend render-farm.exe"]';
    const low = 'input[aria-label="Suspend GoogleUpdate.exe"]';

    it("keeps focus on a box that Space ticks, and the ticks through a visit elsewhere", async () => {
      await clickTab("scan");
      await scanned();
      assert.equal(await $(medium).isSelected(), false);
      assert.equal(await $(low).isSelected(), true);

      await focus(medium);
      await browser.keys("Space");
      assert.equal(await $(medium).isSelected(), true);
      assert.equal(await focused(), "Suspend render-farm.exe");
      await $(low).click();
      assert.equal(await $(low).isSelected(), false);

      await clickTab("targets");
      await clickTab("scan");
      await browser.waitUntil(() => $("#scan-run").isEnabled(), {
        timeout: 15_000,
        timeoutMsg: "the second scan did not finish",
      });
      await scanned();
      assert.equal(await $(medium).isSelected(), true, "the tick was lost");
      assert.equal(await $(low).isSelected(), false, "the untick was lost");
    });

    it("names a service's box for what it does", async () => {
      assert.ok(
        await $('input[aria-label="Stop service WSearch"]').isExisting(),
      );
    });

    it("does not start Quiet Mode when adding the finds was refused", async () => {
      await failInvokes("apply_recommendations", "the disk is full");
      await $("#scan-apply-quiet").click();
      await $("#toast").waitForDisplayed({ timeout: 5_000 });
      assert.match(await text("#toast"), /the disk is full/);
      assert.match((await $("#toast").getAttribute("class")) ?? "", /error/);
      await browser.pause(750);
      await failInvokes("apply_recommendations", null);
      assert.equal(await invokeCount("go_quiet"), 0, "Quiet Mode was started");
      assert.equal(await text("#status-pill"), "Ready");
    });

    it("hides the button that also goes quiet while Quiet Mode is on", async () => {
      await clickTab("dashboard");
      await $("#toggle").click();
      await waitForPill("Quiet");
      await clickTab("scan");
      assert.equal(await $("#scan-apply-quiet").isDisplayed(), false);
      assert.equal(await $("#scan-apply").isDisplayed(), true);
      await clickTab("dashboard");
      await $("#toggle").click();
      await waitForPill("Ready");
      await clickTab("scan");
      assert.equal(await $("#scan-apply-quiet").isDisplayed(), true);
    });

    it("says a scan figure is a share of one core, and lets it pass 100", async () => {
      // The CPU column: the seventh, now that the row buttons have the last.
      assert.match(
        await text("#view-scan th.num:nth-child(7)"),
        /100% = one core/,
      );
    });
  });

  describe("Home", () => {
    it("turns the button off for a run it did not start, and on when it ends", async () => {
      await clickTab("dashboard");
      const idle = await engineState();
      // What a tray press sends before its first step: busy, nothing logged.
      await emitEvent("quiet-state", { ...idle, busy: true, log: [] });
      await waitForPill("Working");
      assert.equal(await $("#toggle").isEnabled(), false, "clickable mid-run");
      await emitEvent("quiet-state", idle);
      await waitForPill("Ready");
      assert.equal(await $("#toggle").isEnabled(), true);
    });
  });

  describe("a dialog", () => {
    it("is one dialog however often the window is closed, and holds the page behind it", async () => {
      await clickTab("dashboard");
      await $("#toggle").click();
      await waitForPill("Quiet");

      // The window's close button while Keep running in the tray is off.
      await emitEvent("confirm-quit");
      await $(".dialog-overlay").waitForExist({ timeout: 5_000 });
      await emitEvent("confirm-quit");
      await browser.pause(300);
      assert.equal((await $$(".dialog-overlay")).length, 1, "dialogs stacked");

      const behind = await browser.execute(() =>
        ["header", "#banner", "main"].map(
          (sel) => (document.querySelector(sel) as HTMLElement).inert,
        ),
      );
      assert.deepEqual(behind, [true, true, true]);
      const described = await attribute(".dialog", "aria-describedby");
      assert.ok(
        await browser.execute(
          (id: string) => document.getElementById(id)?.tagName === "P",
          described,
        ),
        "the dialog's text is not linked to it",
      );

      await browser.keys(["Control", "2"]);
      assert.equal(
        await $("#view-dashboard").isDisplayed(),
        true,
        "a shortcut switched tabs behind the dialog",
      );

      await browser.keys("Escape");
      await browser.waitUntil(
        async () => !(await $(".dialog-overlay").isExisting()),
      );
      assert.equal(await text("#status-pill"), "Quiet", "Escape must not quit");
      assert.equal(
        await browser.execute(
          () => (document.querySelector("main") as HTMLElement).inert,
        ),
        false,
      );

      await $("#toggle").click();
      await waitForPill("Ready");
    });

    it("scrolls a long text and keeps its buttons in reach", async () => {
      // A restore that cannot undo many steps lists them all. The markup is
      // the one `showDialog` builds, under the page's own stylesheet.
      const fit = await browser.execute(() => {
        const overlay = document.createElement("div");
        overlay.className = "dialog-overlay";
        const dialog = document.createElement("div");
        dialog.className = "dialog";
        const body = document.createElement("p");
        body.textContent = Array.from(
          { length: 40 },
          (_, i) =>
            `• Start service S${i}: S${i} stays stopped until you start it`,
        ).join("\n");
        const buttons = document.createElement("div");
        buttons.className = "dialog-buttons";
        const button = document.createElement("button");
        button.textContent = "Give up";
        buttons.append(button);
        dialog.append(document.createElement("h2"), body, buttons);
        overlay.append(dialog);
        document.body.append(overlay);
        try {
          const box = button.getBoundingClientRect();
          return {
            top: box.top,
            bottom: box.bottom,
            window: window.innerHeight,
            scrolls: body.scrollHeight > body.clientHeight,
          };
        } finally {
          overlay.remove();
        }
      });
      assert.ok(fit.scrolls, "the text is cut off instead of scrolling");
      assert.ok(
        fit.top >= 0 && fit.bottom <= fit.window,
        `the buttons are outside the window: ${JSON.stringify(fit)}`,
      );
    });
  });

  describe("Settings", () => {
    it("says what closing the window does, following Keep running in the tray", async () => {
      await clickTab("settings");
      await $("#set-close-tray").click();
      await browser.waitUntil(
        async () =>
          readJson<SavedSettings>("settings.json")?.close_to_tray === false,
        { timeout: 5_000, timeoutMsg: "the setting was not saved" },
      );
      await clickTab("dashboard");
      assert.match(await text("#close-hint"), /quits CompuQuiet/);

      await clickTab("settings");
      await $("#set-close-tray").click();
      await browser.waitUntil(
        async () =>
          readJson<SavedSettings>("settings.json")?.close_to_tray === true,
        { timeout: 5_000, timeoutMsg: "the setting was not put back" },
      );
      await clickTab("dashboard");
      assert.match(await text("#close-hint"), /leaves CompuQuiet in the tray/);
    });

    it("draws the window frame in the chosen theme, and puts it back when the save is refused", async () => {
      await clickTab("settings");
      await choose("#set-theme", "light");
      await browser.waitUntil(async () => (await windowTheme()) === "light", {
        timeout: 5_000,
        timeoutMsg: "the title bar kept the old theme",
      });
      assert.equal(await attribute("html", "data-theme"), "light");

      await failInvokes("save_settings", "the file is locked");
      await choose("#set-theme", "dark");
      await $("#toast").waitForDisplayed({ timeout: 5_000 });
      assert.match(await text("#toast"), /the file is locked/);
      await failInvokes("save_settings", null);
      await browser.waitUntil(
        async () => (await attribute("html", "data-theme")) === "light",
        {
          timeout: 5_000,
          timeoutMsg: "the page kept a theme that was refused",
        },
      );
      assert.equal(await $("#set-theme").getValue(), "light");
      assert.equal(await windowTheme(), "light");

      // As seeded, for what follows.
      await choose("#set-theme", "dark");
      await browser.waitUntil(async () => (await windowTheme()) === "dark", {
        timeout: 5_000,
        timeoutMsg: "the title bar did not go back",
      });
      await browser.waitUntil(
        async () => readJson<SavedSettings>("settings.json")?.theme === "dark",
      );
      await clickTab("dashboard");
    });

    it("asks the system about sign-in start only when Settings opens", async () => {
      const before = await invokeCount("get_autostart");
      await $("#toggle").click();
      await waitForPill("Quiet");
      await $("#toggle").click();
      await waitForPill("Ready");
      assert.equal(
        await invokeCount("get_autostart"),
        before,
        "a change of state queried the sign-in task",
      );
      await clickTab("settings");
      await browser.waitUntil(
        async () => (await invokeCount("get_autostart")) > before,
        { timeout: 5_000, timeoutMsg: "opening Settings did not query it" },
      );
      await clickTab("dashboard");
    });
  });

  describe("the tray window", () => {
    it("stops sampling the machine while hidden, and starts again when shown", async () => {
      // The graphics memory is asked every five seconds, so wait for it too.
      await browser.waitUntil(async () => (await invokeCount("get_gpu")) > 0, {
        timeout: 10_000,
        timeoutMsg: "the graphics memory was never asked for",
      });
      await browser.waitUntil(
        async () => (await invokeCount("get_stats")) > 0,
        {
          timeout: 10_000,
          timeoutMsg: "the live figures were never asked for",
        },
      );
      await setWindowVisible(false);
      try {
        await browser.pause(500);
        const asked = await invokeCount("get_stats");
        const askedGpu = await invokeCount("get_gpu");
        await browser.pause(5_500);
        assert.equal(
          await invokeCount("get_stats"),
          asked,
          "a hidden window kept polling",
        );
        assert.equal(
          await invokeCount("get_gpu"),
          askedGpu,
          "a hidden window kept asking the graphics driver",
        );
      } finally {
        await setWindowVisible(true);
      }
      const asked = await invokeCount("get_stats");
      await browser.waitUntil(
        async () => (await invokeCount("get_stats")) > asked,
        {
          timeout: 8_000,
          timeoutMsg: "the figures did not resume when the window was shown",
        },
      );
      const askedGpu = await invokeCount("get_gpu");
      await browser.waitUntil(
        async () => (await invokeCount("get_gpu")) > askedGpu,
        {
          timeout: 8_000,
          timeoutMsg: "the graphics memory did not resume when shown",
        },
      );
    });
  });
});
