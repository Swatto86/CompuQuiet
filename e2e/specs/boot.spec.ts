/** Boot to the main window: the dashboard renders, live figures arrive. */
import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";

import {
  clickTab,
  logWebviewDiagnostics,
  screenshot,
  text,
} from "./support.ts";

describe("CompuQuiet boots", () => {
  it("shows the dashboard ready for Quiet Mode", async () => {
    await logWebviewDiagnostics();
    await $("#toggle").waitForExist({ timeout: 30_000 });
    assert.equal(
      await $("#hero-title").getText(),
      "Ready for a game or local AI",
    );
    assert.equal(await text("#status-pill"), "Ready");
    assert.equal(await $("#toggle").getAttribute("aria-pressed"), "false");
    assert.equal(
      await browser.execute(() =>
        document.documentElement.getAttribute("data-theme"),
      ),
      "dark",
      "the seeded theme is applied before the window is revealed",
    );
  });

  it("reports live CPU and memory figures", async () => {
    await browser.waitUntil(
      async () => (await $("#mem-value").getText()) !== "—",
      {
        timeout: 10_000,
        timeoutMsg: "the memory figure never arrived",
      },
    );
    const memory = await $("#mem-value").getText();
    assert.match(memory, /of .* GB$/, memory);
    assert.match(await $("#proc-value").getText(), /^\d+$/);
    await screenshot("dashboard");
  });

  it("names the version and platform under Settings", async () => {
    await clickTab("settings");
    const version = JSON.parse(
      fs.readFileSync(path.resolve(process.cwd(), "package.json"), "utf8"),
    ).version as string;
    assert.equal(await text("#about-version"), `v${version}`);
    assert.ok(
      ["Windows", "Linux", "macOS"].includes(await $("#about-os").getText()),
    );
    assert.equal(await $("#about-build").getText(), "development");
    await clickTab("dashboard");
  });
});
