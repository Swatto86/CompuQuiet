/**
 * Settings and targets survive a save and a restart of the app, and a Quiet
 * Mode left on from before the machine last restarted is finished at startup.
 */
import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";

import {
  clickTab,
  dataDir,
  engineState,
  readJson,
  waitForPill,
} from "./support.ts";

interface SavedSettings {
  theme: string;
  close_to_tray: boolean;
  profile: {
    processes: { name: string; action: string }[];
    keep_alive: string[];
  };
}

describe("persistence", () => {
  it("saves an added target and a changed theme to settings.json", async () => {
    await clickTab("targets");
    await $("#process-name").setValue("Spotify");
    await $("#process-action").selectByAttribute("value", "close");
    await $("#process-add button[type=submit]").click();
    assert.equal(await $("#targets-status").getText(), "Unsaved changes");
    await $("#targets-save").click();
    await browser.waitUntil(
      async () => (await $("#targets-status").getText()) === "",
      {
        timeout: 5_000,
        timeoutMsg: "the save did not settle",
      },
    );

    await clickTab("settings");
    await $("#set-theme").selectByAttribute("value", "light");
    await browser.waitUntil(
      async () => readJson<SavedSettings>("settings.json")?.theme === "light",
      {
        timeout: 5_000,
        timeoutMsg: "settings.json did not record the theme",
      },
    );
    const saved = readJson<SavedSettings>("settings.json")!;
    assert.deepEqual(
      saved.profile.processes.find((target) => target.name === "Spotify"),
      { name: "Spotify", action: "close", enabled: true },
    );
    assert.deepEqual(
      saved.profile.keep_alive,
      ["game"],
      "seeded entries survive an edit",
    );
    assert.equal(
      await browser.execute(() =>
        document.documentElement.getAttribute("data-theme"),
      ),
      "light",
    );
  });

  it("comes back with the same targets and theme after a restart", async () => {
    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
    assert.equal(
      await browser.execute(() =>
        document.documentElement.getAttribute("data-theme"),
      ),
      "light",
    );
    await clickTab("targets");
    const names = await $$("#process-targets td.name").map((cell) =>
      cell.getText(),
    );
    assert.ok(names.includes("Spotify"), names.join(", "));
    await clickTab("dashboard");
  });

  it("finishes a Quiet Mode left on from before the last restart", async () => {
    // As if written before the fake machine last booted (its uptime was
    // higher then): what it parked is gone, so nothing is relaunched with
    // stale arguments.
    fs.writeFileSync(
      path.join(dataDir(), "journal.json"),
      JSON.stringify({
        version: 1,
        started_at: 1_000_000_000,
        began: { uptime: 999_999, sign_in: 0 },
        done: [
          { kind: "service_stopped", name: "SysMain" },
          {
            kind: "process_closed",
            name: "Dropbox.exe",
            exe: "C:/fake/Dropbox.exe",
            args: ["Dropbox.exe"],
            cwd: null,
          },
        ],
      }),
    );
    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await browser.waitUntil(async () => !readJson("journal.json"), {
      timeout: 15_000,
      timeoutMsg: "the stale journal was not finished at startup",
    });
    await waitForPill("Ready");

    // Finished, not undone: a full restore would also empty the journal and
    // end on Ready, but it would relaunch Dropbox from its old arguments and
    // start SysMain again. The two steps the restart overtook are skipped.
    const { log } = (await engineState()) as {
      log: { label: string; ok: boolean; detail: string | null }[];
    };
    const skipped = log
      .filter((line) => line.detail?.startsWith("Skipped:"))
      .map((line) => `${line.label} ${line.detail}`);
    assert.equal(skipped.length, 2, JSON.stringify(log));
    assert.match(
      skipped.join("\n"),
      /^Relaunch Dropbox\.exe Skipped: you have signed in again since/m,
    );
    assert.match(
      skipped.join("\n"),
      /^Start service SysMain Skipped: the PC has restarted since/m,
    );
    assert.ok(
      log.every((line) => line.ok),
      JSON.stringify(log),
    );
  });
});
