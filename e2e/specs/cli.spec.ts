/**
 * Command-line control: `--quiet`, `--restore` and `--toggle` from a second
 * launch switch Quiet Mode in the copy that is running, without bringing its
 * window up, and start a copy of their own when none is running. An argument
 * that is not understood refuses the launch and changes nothing. The launches
 * are the real binary, so the journal on disk is what tells what was done.
 */
import { spawn } from "node:child_process";
import { strict as assert } from "node:assert";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { application } from "../wdio.conf.ts";
import { DATA_DIR_ENV, SEEDED_SETTINGS } from "../workspace.ts";
import { launchAgain } from "./launch.ts";
import {
  dataDir,
  readJson,
  setWindowVisible,
  waitForPill,
  windowVisible,
} from "./support.ts";

const journalKinds = (dir: string): string[] | undefined => {
  const file = path.join(dir, "journal.json");
  if (!fs.existsSync(file)) return undefined;
  const journal = JSON.parse(fs.readFileSync(file, "utf8")) as {
    done: { kind: string }[];
  };
  return journal.done.map((step) => step.kind);
};

async function untilJournal(dir: string, present: boolean): Promise<void> {
  await browser.waitUntil(
    async () => (journalKinds(dir) !== undefined) === present,
    {
      timeout: 20_000,
      timeoutMsg: `the journal never ${present ? "appeared" : "went"}`,
    },
  );
}

const requestsLeft = (dir: string): string[] => {
  const wake = path.join(dir, "wake");
  return fs.existsSync(wake) ? fs.readdirSync(wake) : [];
};

describe("the command line", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
  });

  after(async () => {
    await setWindowVisible(true);
  });

  it("switches the running copy without showing its window", async () => {
    const dir = dataDir();
    await setWindowVisible(false);
    assert.equal(journalKinds(dir), undefined, "expected Quiet Mode to be off");

    assert.equal(await launchAgain(["--quiet"]), 0);
    await untilJournal(dir, true);
    // Each step is on record before it happens, so the journal appears while
    // the run is still going: read it once the run has finished.
    await waitForPill("Quiet");
    assert.equal(await windowVisible(), false, "the window was brought up");
    const kinds = journalKinds(dir);
    assert.ok(kinds?.includes("process_suspended"), `${kinds}`);
    const began = readJson<{ started_at: number }>("journal.json")?.started_at;

    // Asked again, it is already so: nothing is done a second time.
    assert.equal(await launchAgain(["--quiet"]), 0);
    await browser.pause(1_500);
    assert.equal(
      readJson<{ started_at: number }>("journal.json")?.started_at,
      began,
    );
    assert.deepEqual(journalKinds(dir), kinds);

    assert.equal(await launchAgain(["--toggle"]), 0);
    await untilJournal(dir, false);

    // Nothing is parked, so there is nothing to put back.
    assert.equal(await launchAgain(["--restore"]), 0);
    await browser.pause(1_500);
    assert.equal(journalKinds(dir), undefined);

    assert.equal(await launchAgain(["--toggle"]), 0);
    await untilJournal(dir, true);
    assert.equal(await launchAgain(["--restore"]), 0);
    await untilJournal(dir, false);

    // Sent one after the other, the last says where it ends.
    assert.equal(await launchAgain(["--quiet"]), 0);
    assert.equal(await launchAgain(["--restore"]), 0);
    await browser.waitUntil(async () => journalKinds(dir) === undefined, {
      timeout: 20_000,
      timeoutMsg: "the restore was lost",
    });

    assert.equal(await windowVisible(), false, "the window was brought up");
    assert.deepEqual(requestsLeft(dir), [], "a request was left behind");

    // What the window says once it is opened is where the machine is.
    await setWindowVisible(true);
    await waitForPill("Ready");
  });

  it("refuses what it does not understand, and does nothing", async () => {
    const dir = dataDir();
    for (const args of [
      ["--quite"],
      ["quiet"],
      ["--quiet", "--restore"],
      ["--quiet", "extra"],
      ["--profile", "work"],
    ]) {
      assert.equal(await launchAgain(args), 2, `${args}`);
    }
    await browser.pause(1_500);
    assert.deepEqual(requestsLeft(dir), [], "a refused launch left a request");
    assert.equal(
      journalKinds(dir),
      undefined,
      "a refused launch parked programs",
    );
  });

  it("starts a copy of its own when none is running, and acts", async () => {
    // Its own state and its own fake machine, so nothing here is the suite's.
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "compuquiet-cli-"));
    fs.writeFileSync(
      path.join(dir, "settings.json"),
      JSON.stringify(SEEDED_SETTINGS),
    );
    const env = {
      ...process.env,
      [DATA_DIR_ENV]: dir,
      // A browser process of its own, beside the suite's.
      WEBVIEW2_USER_DATA_FOLDER: path.join(dir, "webview"),
    };
    const copy = spawn(application, ["--quiet"], {
      stdio: "ignore",
      windowsHide: true,
      env,
    });
    const gone = new Promise<void>((resolve) =>
      copy.once("exit", () => resolve()),
    );
    try {
      // The journal appears with the run's first step; wait for a later one.
      await browser.waitUntil(
        async () => journalKinds(dir)?.includes("process_suspended") === true,
        {
          timeout: 20_000,
          timeoutMsg: "the copy never parked a program",
        },
      );
      // It is the running copy now: a later launch reaches it.
      assert.equal(await launchAgain(["--restore"], env), 0);
      await untilJournal(dir, false);

      // What an update would relaunch it with has lost the command.
      const log = path.join(dir, "compuquiet.log");
      assert.ok(fs.existsSync(log), "the copy keeps no log");
      assert.doesNotMatch(
        fs.readFileSync(log, "utf8"),
        /kept the command in the launch arguments/,
      );
    } finally {
      copy.kill();
      await gone;
      try {
        fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 });
      } catch {
        // The browser processes may hold their folder a moment longer; the
        // temp directory is not the suite's state.
      }
    }
  });
});
