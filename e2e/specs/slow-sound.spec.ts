/**
 * Two ways a run looks after a program it could hurt, against the fake
 * machine: "Slow down" keeps a program running at the lowest priority instead
 * of freezing or closing it, and a program with sound running (a call, a
 * song) is left alone, with the reason.
 *
 * The fake machine seeds Dropbox as a Close target and Slack as a Suspend one.
 * Runs after ai-models, so Dropbox is the copy Restore relaunched: its size
 * and PID are matched by shape. Puts Dropbox back as Close and the sound off.
 */
import { strict as assert } from "node:assert";

import {
  clickTab,
  fakeAudio,
  fakeFail,
  fakeHeal,
  fakeSlowed,
  readJson,
  screenshot,
  texts,
  waitForPill,
} from "./support.ts";

interface Journal {
  done: { kind: string; name?: string; previous?: { priority: number } }[];
}

interface SavedSettings {
  profile: {
    processes: { name: string; action: string; slow_down?: boolean }[];
  };
}

const STEPS = "#preview-steps li";
const LEFT = "#preview-left li";
const DROPBOX_ACTION = 'select[aria-label="Action for Dropbox"]';
const SLOWED =
  /^Slow down Dropbox\.exe \(1 process, [\d.]+ MB\)\. It keeps running at the lowest priority$/;
const SPARED =
  "Slack.exe — playing or recording sound right now, so it is left alone";

function savedDropbox() {
  return readJson<SavedSettings>("settings.json")?.profile.processes.find(
    (target) => target.name === "Dropbox",
  );
}

async function setDropbox(handling: string): Promise<void> {
  await clickTab("targets");
  await $(DROPBOX_ACTION).selectByAttribute("value", handling);
  await $("#targets-save").click();
  await browser.waitUntil(
    async () => {
      const saved = savedDropbox();
      return handling === "slow_down"
        ? saved?.slow_down === true
        : saved?.action === handling && saved.slow_down === undefined;
    },
    { timeout: 5_000, timeoutMsg: `settings.json did not record ${handling}` },
  );
  await clickTab("dashboard");
}

/** Open the preview and look again, so it shows the machine as it is now. */
async function look(): Promise<{ steps: string[]; left: string[] }> {
  const open = await browser.execute(
    () => (document.getElementById("preview") as HTMLDetailsElement).open,
  );
  if (!open) await $("#preview summary").click();
  await $("#preview-refresh").click();
  await browser.waitUntil(
    async () => (await $("#preview-status").getText()).startsWith("As of"),
    { timeout: 10_000, timeoutMsg: "the preview never looked at the machine" },
  );
  return { steps: await texts(STEPS), left: await texts(LEFT) };
}

describe("slowing a program down, and sparing one with sound", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await clickTab("dashboard");
  });

  after(async () => {
    await fakeHeal();
    await fakeAudio([]);
    await setDropbox("close");
    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
  });

  it("saves a slowed program as a suspend that an older release reads as one", async () => {
    await setDropbox("slow_down");
    assert.deepEqual(savedDropbox(), {
      name: "Dropbox",
      action: "suspend",
      enabled: true,
      slow_down: true,
    });
    await clickTab("targets");
    assert.equal(await $(DROPBOX_ACTION).getValue(), "slow_down");
    await clickTab("dashboard");
  });

  it("lists it in the preview, and a look slows nothing", async () => {
    const { steps } = await look();
    assert.ok(
      steps.some((line) => SLOWED.test(line)),
      steps.join(" | "),
    );
    assert.ok(!steps.some((line) => line.startsWith("Close Dropbox")));
    assert.deepEqual(await fakeSlowed(), []);
    await screenshot("slow-down-preview");
  });

  it("slows it in a run without freezing or closing it, and Restore speeds it up", async () => {
    await $("#toggle").click();
    await waitForPill("Quiet");
    assert.deepEqual(await fakeSlowed(), ["Dropbox.exe"]);
    assert.match(
      await $("#summary").getText(),
      /1 process slowed down \(back to full speed on restore\)/,
    );

    const journal = readJson<Journal>("journal.json");
    assert.ok(journal, "journal.json was not written");
    const dropbox = journal.done.filter((step) => step.name === "Dropbox.exe");
    assert.deepEqual(
      dropbox.map((step) => step.kind),
      ["process_slowed"],
      "neither suspended nor closed",
    );
    assert.equal(
      typeof dropbox[0]?.previous?.priority,
      "number",
      "how it ran before is on record",
    );

    await $("#toggle").click();
    await waitForPill("Ready");
    assert.deepEqual(await fakeSlowed(), []);
    const lines = await $$("#log li").map((line) => line.getText());
    assert.ok(
      lines.some((line) => /^Speed up Dropbox\.exe \(PID \d+\)$/.test(line)),
      lines.join(" | "),
    );
  });

  it("leaves a program alone while it plays sound, and says why", async () => {
    await fakeAudio(["Slack"]);
    const { steps, left } = await look();
    assert.ok(!steps.some((line) => line.includes("Slack")), steps.join(" | "));
    assert.ok(left.includes(SPARED), left.join(" | "));

    await $("#toggle").click();
    await waitForPill("Quiet");
    const journal = readJson<Journal>("journal.json");
    assert.ok(
      !journal?.done.some((step) => step.name === "Slack.exe"),
      "Slack is not on record: nothing was done to it",
    );
    assert.match(await $("#skipped").getText(), /Slack\.exe — playing or/);
    await screenshot("sound-spared");
    await $("#toggle").click();
    await waitForPill("Ready");
  });

  it("parks it again once the sound stops", async () => {
    await fakeAudio([]);
    const { steps, left } = await look();
    assert.ok(
      steps.some((line) => line.startsWith("Suspend Slack.exe")),
      steps.join(" | "),
    );
    assert.ok(!left.includes(SPARED));
  });

  it("says so when it cannot tell who is using sound, and still parks", async () => {
    await fakeAudio(["Slack"]);
    await fakeFail("audio_users", null, "refused");
    const { steps, left } = await look();
    assert.ok(
      steps.some((line) => line.startsWith("Suspend Slack.exe")),
      "nothing is spared without an answer",
    );
    assert.ok(
      left.some((line) => line.startsWith("Sound — who is using it")),
      left.join(" | "),
    );
    await fakeHeal();
  });
});
