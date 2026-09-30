/**
 * A restore step that can never succeed must not keep Quiet Mode on for ever.
 * The fake machine refuses to start a service again; Restore leaves that one
 * entry and Home says so. Giving up is confirmed, names the entry and what it
 * leaves behind, ends Quiet Mode and keeps the record as journal.json.bad.
 */
import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";

import {
  clickDialogButton,
  clickTab,
  dataDir,
  fakeFail,
  fakeHeal,
  logText,
  readJson,
  text,
  waitForPill,
} from "./support.ts";

interface Journal {
  done: { kind: string; name?: string }[];
}

describe("a restore that cannot finish", () => {
  const journalFile = () => path.join(dataDir(), "journal.json");
  const keptFile = () => path.join(dataDir(), "journal.json.bad");

  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await clickTab("dashboard");
  });

  after(async () => {
    // A fresh fake machine for the specs after this one.
    await fakeHeal();
    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
  });

  it("leaves Quiet Mode on with only the entry that failed, and says so", async () => {
    await $("#toggle").click();
    await waitForPill("Quiet");
    assert.equal(await $("#recovery").isDisplayed(), false, "nothing is stuck");

    await fakeFail("start_service", "SysMain", "refused");
    const logged = logText().length;
    await $("#toggle").click();
    await $("#recovery").waitForDisplayed({ timeout: 15_000 });

    assert.equal(await text("#status-pill"), "Quiet", "still on");
    assert.match(await text("#recovery-list"), /Start service SysMain/);
    const journal = readJson<Journal>("journal.json");
    assert.deepEqual(
      journal?.done.map((step) => `${step.kind}:${step.name}`),
      ["service_stopped:SysMain"],
      "only what could not be put back is left on record",
    );
    assert.match(
      logText().slice(logged),
      /Start service SysMain failed \(platform\)/,
      "the failed step is in compuquiet.log, which outlasts the window",
    );
  });

  it("gives up only once confirmed, and keeps the record", async () => {
    await $("#recovery-action").click();
    await $(".dialog-overlay").waitForExist({ timeout: 5_000 });
    const body = await text(".dialog p");
    assert.match(body, /Start service SysMain/);
    assert.match(body, /SysMain stays stopped/);

    await clickDialogButton("Cancel");
    await $(".dialog-overlay").waitForExist({ reverse: true, timeout: 5_000 });
    assert.ok(fs.existsSync(journalFile()), "cancelling changes nothing");
    assert.equal(await text("#status-pill"), "Quiet");

    await $("#recovery-action").click();
    await clickDialogButton("Give up on these");
    await waitForPill("Ready");
    await $("#recovery").waitForDisplayed({ reverse: true, timeout: 5_000 });

    assert.equal(fs.existsSync(journalFile()), false, "Quiet Mode has ended");
    const kept = JSON.parse(fs.readFileSync(keptFile(), "utf8")) as Journal;
    assert.deepEqual(
      kept.done.map((step) => step.name),
      ["SysMain"],
      "what was given up is still on record",
    );
    assert.match(await text("#log"), /Gave up putting back 1 item/);
  });

  it("can go quiet again afterwards", async () => {
    await fakeHeal();
    await $("#toggle").click();
    await waitForPill("Quiet");
    assert.ok(readJson<Journal>("journal.json"), "a new journal is written");
    await $("#toggle").click();
    await waitForPill("Ready");
  });
});
