/**
 * Quiet Mode that starts when a chosen program runs and ends when it has
 * closed: off until the user turns it on, never acting over a run somebody
 * pressed for, and gentler than a press. Time is the fake machine's uptime.
 */
import { strict as assert } from "node:assert";

import {
  advanceUntil,
  clickTab,
  fakeAdvance,
  fakeProgram,
  readJson,
  text,
  texts,
  waitForPill,
  waitForToast,
} from "./support.ts";

interface Journal {
  ending?: { kind: string; program?: string };
  done: { kind: string; name?: string }[];
}

interface Saved {
  auto_quiet: { enabled: boolean; programs: string[] };
}

const journal = (): Journal | undefined => readJson<Journal>("journal.json");
const pill = (): Promise<string> => text("#status-pill");
const quiet = async (): Promise<boolean> => (await pill()) === "Quiet";
const ready = async (): Promise<boolean> => (await pill()) === "Ready";

describe("Quiet Mode that starts by itself", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
  });

  it("is off until it is turned on, and starts nothing while it is off", async () => {
    await clickTab("settings");
    assert.equal(await $("#set-auto-quiet").isSelected(), false);
    assert.equal(
      readJson<Saved>("settings.json")?.auto_quiet.enabled ?? false,
      false,
    );

    await fakeProgram("steam.exe", true);
    await fakeAdvance(600);
    await browser.pause(1_500);
    await fakeAdvance(600);
    await browser.pause(1_500);
    assert.equal(await pill(), "Ready");
    await fakeProgram("steam.exe", false);
  });

  it("keeps the programs the user chose, and turns on only when asked", async () => {
    await $("#auto-quiet-name").setValue("steam.exe");
    await $("#auto-quiet-add button[type=submit]").click();
    await $("#set-auto-quiet").click();
    await browser.waitUntil(
      async () => {
        const auto = readJson<Saved>("settings.json")?.auto_quiet;
        return auto?.enabled === true && auto.programs.includes("steam.exe");
      },
      { timeout: 5_000, timeoutMsg: "the list was not saved" },
    );
    assert.deepEqual(await texts("#auto-quiet-list li"), ["steam.exe✕"]);
    await clickTab("dashboard");
  });

  it("starts Quiet Mode gently once the program has run for a while", async () => {
    await fakeProgram("steam.exe", true);
    await advanceUntil(11, quiet, "Quiet Mode starting");

    const run = journal();
    assert.deepEqual(run?.ending, { kind: "trigger", program: "steam.exe" });
    const kinds = run?.done.map((step) => step.kind) ?? [];
    assert.ok(kinds.includes("process_suspended"), kinds.join());
    // Nobody pressed the button: nothing is closed and the cache is left.
    assert.ok(
      !kinds.includes("process_closed") && !kinds.includes("memory_purged"),
      kinds.join(),
    );
    const log = await text("#log");
    assert.match(log, /Started because steam\.exe is running/);
    assert.match(await text("#skipped"), /Memory purge — left alone/);
    assert.equal(
      await text("#until-text"),
      "Started because steam.exe is running. Ends by itself once none of your auto-quiet programs is.",
    );
    // The program it started for is never parked by it.
    assert.ok(
      !run?.done.some((step) => step.name?.toLowerCase().startsWith("steam")),
    );
  });

  it("does not start again over a game after it was put back by hand", async () => {
    await $("#toggle").click();
    await waitForPill("Ready");
    for (let round = 0; round < 3; round += 1) {
      await fakeAdvance(60);
      await browser.pause(800);
    }
    assert.equal(await pill(), "Ready", "steam is still running");
    assert.equal(journal(), undefined);
  });

  it("ends by itself once the program has been gone for a while", async () => {
    await fakeProgram("steam.exe", false);
    await fakeAdvance(60);
    await browser.pause(1_000);
    await fakeProgram("steam.exe", true);
    await advanceUntil(11, quiet, "Quiet Mode starting again");

    await fakeProgram("steam.exe", false);
    // A program that comes back within the wait does not end the run.
    await fakeAdvance(10);
    await browser.pause(1_000);
    await fakeProgram("steam.exe", true);
    await fakeAdvance(60);
    await browser.pause(1_500);
    assert.equal(await pill(), "Quiet", "it came back in time");

    await fakeProgram("steam.exe", false);
    await advanceUntil(31, ready, "the run ending");
    await waitForToast(/steam\.exe has closed, so everything is back\./);
    assert.equal(journal(), undefined);
  });

  it("never ends, or starts over, a run somebody pressed for", async () => {
    await $("#toggle").click();
    await waitForPill("Quiet");
    assert.equal(journal()?.ending, undefined);

    await fakeProgram("steam.exe", true);
    await fakeAdvance(120);
    await browser.pause(1_500);
    await fakeProgram("steam.exe", false);
    for (let round = 0; round < 3; round += 1) {
      await fakeAdvance(61);
      await browser.pause(800);
    }
    assert.equal(
      await pill(),
      "Quiet",
      "a pressed run stays until it is undone",
    );
    assert.equal(journal()?.ending, undefined);

    await $("#toggle").click();
    await waitForPill("Ready");
  });

  it("can be turned off again", async () => {
    await clickTab("settings");
    await $("#set-auto-quiet").click();
    await $("#auto-quiet-list li button").click();
    await browser.waitUntil(
      async () => {
        const auto = readJson<Saved>("settings.json")?.auto_quiet;
        return auto?.enabled === false && auto.programs.length === 0;
      },
      { timeout: 5_000, timeoutMsg: "the list was not put back" },
    );
    await clickTab("dashboard");
  });
});
