/**
 * Unloading local AI models is a step of a run, off until it is ticked in the
 * Park list: the preview lists each model before the purge, the run asks its
 * server to let go of it, and nothing about it is journaled, since there is
 * nothing to put back.
 *
 * The fake machine's Ollama starts with one model, "llama3:8b" (5 GiB). Runs
 * after run-report; puts the option and the model back as it found them.
 */
import { strict as assert } from "node:assert";

import {
  type FakeModel,
  clickTab,
  fakeFail,
  fakeHeal,
  fakeModels,
  readJson,
  screenshot,
  texts,
  waitForPill,
} from "./support.ts";

interface Journal {
  done: { kind: string }[];
}

interface SavedSettings {
  profile: { unload_ai_models?: boolean };
}

const STEPS = "#preview-steps li";
const UNLOAD =
  "Unload llama3:8b (Ollama), 5.0 GB. It loads again when it is next used";
const SEEDED: FakeModel = {
  server: "ollama",
  name: "llama3:8b",
  bytes: 5 * 1024 ** 3,
};

async function setOption(on: boolean): Promise<void> {
  await clickTab("targets");
  if ((await $("#opt-unload").isSelected()) !== on)
    await $("#opt-unload").click();
  await $("#targets-save").click();
  await browser.waitUntil(
    async () =>
      (readJson<SavedSettings>("settings.json")?.profile.unload_ai_models ??
        false) === on,
    { timeout: 5_000, timeoutMsg: `settings.json did not record ${on}` },
  );
  await clickTab("dashboard");
}

async function steps(): Promise<string[]> {
  const open = await browser.execute(
    () => (document.getElementById("preview") as HTMLDetailsElement).open,
  );
  if (!open) await $("#preview summary").click();
  await browser.waitUntil(async () => (await texts(STEPS)).length > 0, {
    timeout: 10_000,
    timeoutMsg: "the preview listed nothing",
  });
  return texts(STEPS);
}

describe("unloading local AI models", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await clickTab("dashboard");
  });

  after(async () => {
    await fakeHeal();
    await fakeModels([SEEDED]);
    await setOption(false);
    await browser.reloadSession();
    await $("#toggle").waitForExist({ timeout: 30_000 });
  });

  it("is off until ticked: the preview does not list it and a look asks nothing", async () => {
    assert.equal(await $("#opt-unload").isExisting(), true);
    const listed = await steps();
    assert.ok(
      !listed.some((line) => /^Unload /.test(line)),
      listed.join(" | "),
    );
    assert.equal((await fakeModels()).length, 1);
  });

  it("lists the model before the purge once it is ticked, and unloads nothing yet", async () => {
    await setOption(true);
    await browser.waitUntil(async () => (await steps()).includes(UNLOAD), {
      timeout: 10_000,
      timeoutMsg: `the preview never listed the model: ${(await texts(STEPS)).join(" | ")}`,
    });
    const listed = await texts(STEPS);
    assert.equal(listed[listed.length - 2], UNLOAD, listed.join(" | "));
    assert.equal(listed[listed.length - 1], "Purge cached memory");
    assert.equal((await fakeModels()).length, 1, "looking unloads nothing");
    await screenshot("ai-models-preview");
  });

  it("unloads it in a run and leaves nothing in the journal to put back", async () => {
    await $("#toggle").click();
    await waitForPill("Quiet");
    assert.deepEqual(await fakeModels(), []);

    const log = await $$("#log li").map((line) => line.getText());
    assert.ok(
      log.includes("Unload llama3:8b from Ollama"),
      `the run log: ${log.join(" | ")}`,
    );
    const journal = readJson<Journal>("journal.json");
    assert.ok(journal, "journal.json was not written");
    assert.deepEqual(
      journal.done.map((step) => step.kind),
      [
        "power_plan_changed",
        "service_stopped",
        "process_suspended",
        "process_closed",
        "process_suspended",
        "memory_purged",
      ],
    );

    await $("#toggle").click();
    await waitForPill("Ready");
    assert.deepEqual(
      await fakeModels(),
      [],
      "restoring does not load it again",
    );
  });

  it("says so when nothing is loaded", async () => {
    await steps();
    await browser.waitUntil(
      async () =>
        (await texts("#preview-left li")).includes(
          "AI models — none is loaded in Ollama or LM Studio",
        ),
      {
        timeout: 10_000,
        timeoutMsg: `the left-alone list: ${(await texts("#preview-left li")).join(" | ")}`,
      },
    );
  });

  it("reports a model that will not unload and still finishes the run", async () => {
    await fakeModels([SEEDED]);
    await fakeFail("unload_model", null, "refused");
    await $("#toggle").click();
    await waitForPill("Quiet");
    const log = await $$("#log li").map((line) => line.getText());
    const failed = log.find((line) => line.includes("Unload llama3:8b"));
    assert.ok(failed, `the run log: ${log.join(" | ")}`);
    assert.match(failed, /refused/);
    assert.equal((await fakeModels()).length, 1, "it is still loaded");
    assert.match(await $("#summary").getText(), /Cached memory purged/);

    await fakeHeal();
    await $("#toggle").click();
    await waitForPill("Ready");
  });
});
