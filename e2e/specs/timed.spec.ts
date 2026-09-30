/**
 * A run can end by itself, after a time or when a program closes, and the PC
 * can be kept awake while it is on. Time is the fake machine's uptime, moved
 * on by hand: the wall clock plays no part, and the suite waits for none of
 * it.
 */
import { strict as assert } from "node:assert";

import { choose } from "./choose.ts";
import {
  advanceUntil,
  clickTab,
  fakeAdvance,
  fakeAwake,
  fakeProgram,
  readJson,
  text,
  waitForPill,
  waitForToast,
} from "./support.ts";

interface Journal {
  awake?: boolean;
  ending?: { kind: string; uptime?: number; name?: string };
}

interface Saved {
  still_on_hours: number;
  profile: { keep_awake: boolean };
}

const journal = (): Journal | undefined => readJson<Journal>("journal.json");
const ready = async (): Promise<boolean> =>
  (await text("#status-pill")) === "Ready";

async function chooseAndStart(choice: string): Promise<void> {
  await clickTab("dashboard");
  await choose("#until-choice", choice);
  await $("#toggle").click();
  await waitForPill("Quiet");
}

async function saved(): Promise<Saved> {
  const settings = readJson<Saved>("settings.json");
  assert.ok(settings, "settings.json was not written");
  return settings;
}

describe("a run that ends by itself", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
  });

  it("asks for a timed run, shows when it ends and journals the time", async () => {
    await chooseAndStart("120");

    assert.match(
      await text("#until-text"),
      /^Ends by itself in (2 h|1 h 59 min)\.$/,
    );
    assert.equal(await $("#until-now").isDisplayed(), true);
    assert.equal(await $("#until-choose").isDisplayed(), false);
    const ending = journal()?.ending;
    assert.equal(ending?.kind, "at");
    assert.equal(typeof ending?.uptime, "number");
  });

  it("adds an hour to a timer that is running", async () => {
    const before = journal()?.ending?.uptime ?? 0;
    await $("#until-extend").click();
    await browser.waitUntil(
      async () => (journal()?.ending?.uptime ?? 0) > before,
      {
        timeout: 5_000,
        timeoutMsg: "the new time was not journaled",
      },
    );
    assert.equal((journal()?.ending?.uptime ?? 0) - before, 3_600);
    assert.match(
      await text("#until-text"),
      /^Ends by itself in (3 h|2 h 59 min)\.$/,
    );
  });

  it("puts everything back once the machine's uptime reaches it", async () => {
    await advanceUntil(4 * 3_600, ready, "the timed run ending");
    await waitForToast(/The time you set is up, so everything is back\./);
    assert.equal(journal(), undefined, "the journal is removed with the run");
    assert.equal(await $("#until-choose").isDisplayed(), true);
    assert.equal(await $("#until-choice").getValue(), "none");
  });

  it("waits for a program to close, and for it to stay closed", async () => {
    await fakeProgram("launcher.exe", true);
    await clickTab("dashboard");
    await choose("#until-choice", "program");
    await $("#until-program").setValue("launcher.exe");
    await $("#toggle").click();
    await waitForPill("Quiet");

    assert.equal(
      await text("#until-text"),
      "Ends by itself once launcher.exe has closed.",
    );
    assert.deepEqual(journal()?.ending, {
      kind: "program_exits",
      name: "launcher.exe",
    });

    // Time passing with the program still running ends nothing.
    await fakeAdvance(3_600);
    await browser.pause(1_500);
    assert.equal(await text("#status-pill"), "Quiet");

    await fakeProgram("launcher.exe", false);
    await advanceUntil(31, ready, "the run ending after the program closed");
    await waitForToast(/launcher\.exe has closed, so everything is back\./);
  });

  it("refuses to wait for a program that is not running, and starts nothing", async () => {
    await clickTab("dashboard");
    await choose("#until-choice", "program");
    await $("#until-program").setValue("no-such-program.exe");
    await $("#toggle").click();
    await waitForToast(/no-such-program\.exe is not running/);
    assert.equal(await text("#status-pill"), "Ready");
    assert.equal(journal(), undefined);
    await choose("#until-choice", "none");
  });

  it("keeps the PC awake while Quiet Mode is on and lets it sleep after", async () => {
    await clickTab("targets");
    await $("#opt-awake").click();
    await $("#targets-save").click();
    await browser.waitUntil(async () => (await saved()).profile.keep_awake, {
      timeout: 5_000,
      timeoutMsg: "the choice was not saved",
    });

    await clickTab("dashboard");
    assert.equal(await fakeAwake(), false);
    await $("#toggle").click();
    await waitForPill("Quiet");
    assert.equal(await fakeAwake(), true);
    assert.equal(journal()?.awake, true);
    assert.match(await text("#summary"), /PC kept awake/);
    assert.match(await text("#log"), /Keep the PC awake/);

    await $("#toggle").click();
    await waitForPill("Ready");
    assert.equal(await fakeAwake(), false);
    assert.match(await text("#log"), /Let the PC sleep again/);

    await clickTab("targets");
    await $("#opt-awake").click();
    await $("#targets-save").click();
    await browser.waitUntil(async () => !(await saved()).profile.keep_awake, {
      timeout: 5_000,
      timeoutMsg: "the choice was not put back",
    });
  });

  it("says so when Quiet Mode has been on a long time with nothing to end it", async () => {
    await clickTab("settings");
    await choose("#set-still-on", "2");
    await browser.waitUntil(async () => (await saved()).still_on_hours === 2, {
      timeout: 5_000,
      timeoutMsg: "the reminder time was not saved",
    });

    await clickTab("dashboard");
    await $("#toggle").click();
    await waitForPill("Quiet");
    await advanceUntil(
      2 * 3_600 + 60,
      async () => /been on for 2 hours/.test(await text("#toast")),
      "the reminder",
    );
    assert.equal(
      await text("#status-pill"),
      "Quiet",
      "a reminder ends nothing",
    );

    await $("#toggle").click();
    await waitForPill("Ready");
    await clickTab("settings");
    await choose("#set-still-on", "4");
    await browser.waitUntil(async () => (await saved()).still_on_hours === 4, {
      timeout: 5_000,
      timeoutMsg: "the reminder time was not put back",
    });
    await clickTab("dashboard");
  });
});
