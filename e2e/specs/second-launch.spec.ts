/**
 * A second launch while CompuQuiet runs (the Start menu shortcut while the
 * tray copy is hidden) brings up the running copy's window and leaves no
 * second copy behind. It rides on the data directory's lock and a request
 * file, which works between an elevated and an unelevated copy; this suite
 * runs from one unelevated shell, so that pairing is a manual check.
 */
import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";

import { application } from "../wdio.conf.ts";
import { appPids } from "../workspace.ts";
import { launchAgain } from "./launch.ts";
import { dataDir, setWindowVisible, windowVisible } from "./support.ts";

describe("a second launch", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
  });

  it("shows the running window, exits, and leaves one copy", async () => {
    const running = appPids(application);
    assert.equal(running.length, 1, "expected exactly one running copy");
    assert.ok(await windowVisible(), "the window starts shown");
    await setWindowVisible(false);
    assert.equal(
      await windowVisible(),
      false,
      "the window could not be hidden",
    );

    assert.equal(await launchAgain(), 0, "the second launch must exit cleanly");

    await browser.waitUntil(windowVisible, {
      timeout: 10_000,
      timeoutMsg:
        "the running copy's window was not shown by the second launch",
    });
    assert.deepEqual(appPids(application), running, "a second copy is running");
    const left = path.join(dataDir(), "wake");
    assert.deepEqual(
      fs.existsSync(left) ? fs.readdirSync(left) : [],
      [],
      "the request was left in the data directory",
    );
  });
});
