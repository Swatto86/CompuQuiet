/**
 * The Park list's service picker and filter box, against the fake machine's
 * services: the picker offers what the machine has (not what Quiet Mode must
 * never stop), a listed service says what the machine calls it, and the
 * filter keeps the rows that match by name or by that name. Nothing is saved.
 */
import { strict as assert } from "node:assert";

import { clickTab, screenshot, text, texts } from "./support.ts";

async function suggestions(): Promise<string[]> {
  return browser.execute(() =>
    Array.from(
      document.querySelectorAll<HTMLOptionElement>("#running-services option"),
    ).map((option) => `${option.value} | ${option.label}`),
  );
}

/** Type into the filter, or (for nothing) empty it with Backspace: a driver's own clear fires no `input` event. */
async function filterBy(words: string): Promise<void> {
  await $("#park-filter").setValue(words === "" ? "x" : words);
  if (words === "") await browser.keys("Backspace");
  await browser.waitUntil(
    async () => (await $("#park-filter").getValue()) === words,
    { timeoutMsg: `the filter did not take "${words}"` },
  );
}

describe("Park list picker and filter", () => {
  let programs = 0;
  let services = 0;

  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await clickTab("targets");
    await $("#service-targets .display-name").waitForExist({
      timeout: 10_000,
      timeoutMsg: "the Park list never heard from the machine",
    });
    programs = (await $$("#process-targets tr").length) as number;
    services = (await $$("#service-targets tr").length) as number;
  });

  it("offers the services the machine has, and leaves out the essential ones", async () => {
    const offered = await suggestions();
    assert.ok(
      offered.includes("Spooler | Print Spooler · running"),
      offered.join(),
    );
    assert.ok(offered.includes("Fax | Fax · stopped"), offered.join());
    assert.ok(
      !offered.some((entry) => entry.startsWith("AudioSrv")),
      "Quiet Mode never stops sound, so the picker does not offer it",
    );
  });

  it("says what the machine calls a listed service, where it knows", async () => {
    const names = await texts("#service-targets td.name");
    assert.ok(names.includes("SysMainSuperfetch"), names.join());
    assert.ok(
      names.includes("NoSuchService"),
      "an unknown name has nothing to add",
    );
  });

  it("keeps the rows whose name, or the machine's name for them, matches", async () => {
    await filterBy("spool");
    await screenshot("park-list");
    assert.deepEqual(await texts("#service-targets td.name"), [
      "SpoolerPrint Spooler",
    ]);
    assert.deepEqual(await texts("#process-targets td"), [
      "Nothing on this list matches the filter",
    ]);
    assert.equal(
      await text("#park-filter-status"),
      `Showing 0 of ${programs} programs and 1 of ${services} services`,
    );

    await filterBy("SUPERFETCH");
    assert.deepEqual(await texts("#service-targets td.name"), [
      "SysMainSuperfetch",
    ]);

    await filterBy("drive one");
    assert.deepEqual(await texts("#process-targets td.name"), ["OneDrive"]);
    await filterBy("one slack");
    assert.deepEqual(
      await texts("#process-targets td"),
      ["Nothing on this list matches the filter"],
      "every word has to be in the same row",
    );

    await filterBy("");
    assert.equal(await $$("#process-targets tr").length, programs);
    assert.equal(await $$("#service-targets tr").length, services);
    assert.equal(await text("#park-filter-status"), "");
  });

  it("shows a row that was just added even when the filter would hide it", async () => {
    await filterBy("spool");
    await $("#service-name").setValue("Fax");
    await $('#service-add button[type="submit"]').click();
    assert.equal(await $("#park-filter").getValue(), "");
    const fresh = $("#service-targets tr.fresh");
    await fresh.waitForExist({ timeout: 5_000 });
    assert.equal(await fresh.$("td.name").getText(), "Fax");

    // Nothing here was saved: take the edit back out.
    await $('button[aria-label="Remove Fax"]').click();
    await $('button[aria-label="Stop protecting Fax"]').click();
    assert.equal(await text("#targets-status"), "");
    await clickTab("dashboard");
  });
});
