/**
 * A run started from the tray that fails must never look like a dead click,
 * even with notifications turned off and the window hidden: the window comes
 * forward and the page shows the error. Failures also reach compuquiet.log,
 * which outlasts the window.
 */
import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";

import {
  clickTab,
  dataDir,
  fakeFail,
  fakeHeal,
  logText,
  setWindowVisible,
  text,
  trayMenu,
  waitForPill,
  windowVisible,
} from "./support.ts";

/** Cut off mid-value, as a torn write would leave it. */
const TORN = '{ "began": ';

describe("a tray run that fails", () => {
  const settingsFile = () => path.join(dataDir(), "settings.json");
  const journalFile = () => path.join(dataDir(), "journal.json");
  let original: string;

  before(async () => {
    original = fs.readFileSync(settingsFile(), "utf8");
    const quietly = JSON.parse(original) as { notifications: boolean };
    quietly.notifications = false;
    fs.writeFileSync(settingsFile(), JSON.stringify(quietly));
    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await clickTab("dashboard");
  });

  after(async () => {
    await fakeHeal();
    fs.rmSync(journalFile(), { force: true });
    fs.writeFileSync(settingsFile(), original);
    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
  });

  async function hideThenUseTheTray(): Promise<void> {
    await browser.execute(() => {
      document.getElementById("toast")!.hidden = true;
    });
    await setWindowVisible(false);
    assert.equal(await windowVisible(), false, "the window is hidden");
    await trayMenu("tray-toggle");
    await browser.waitUntil(() => windowVisible(), {
      timeout: 10_000,
      timeoutMsg: "the failed tray run left the window hidden",
    });
    await $("#toast").waitForDisplayed({ timeout: 10_000 });
    assert.match(
      (await $("#toast").getAttribute("class")) ?? "",
      /error/,
      "shown as an error",
    );
  }

  it("shows a restore that stopped part way, with notifications off", async () => {
    await $("#toggle").click();
    await waitForPill("Quiet");
    await fakeFail("start_service", "SysMain", "refused");

    const logged = logText().length;
    await hideThenUseTheTray();

    assert.match(await text("#toast"), /could not be restored/);
    assert.equal(await text("#status-pill"), "Quiet", "still on");
    assert.match(
      logText().slice(logged),
      /Start service SysMain failed \(platform\)/,
    );

    await fakeHeal();
    await $("#toggle").click();
    await waitForPill("Ready");
  });

  it("shows an error, with notifications off, and logs it", async () => {
    fs.writeFileSync(journalFile(), TORN);

    const logged = logText().length;
    await hideThenUseTheTray();

    assert.match(await text("#toast"), /could not be read/);
    assert.equal(await text("#status-pill"), "Ready");
    assert.match(
      logText().slice(logged),
      /turning Quiet Mode on failed \(journal_unreadable\)/,
    );
  });
});
