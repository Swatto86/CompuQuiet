/**
 * The two buttons on a Scan row. Never touch keeps a find off every list for
 * good, a service as well as a program, and leaves the Park list's unsaved
 * edits alone. Close instead changes what adding a find does, and a medium-risk
 * program (the ones that hold something unsaved) is only closed once the user
 * has been asked. The spec puts the Park list back as it found it.
 */
import { strict as assert } from "node:assert";

import {
  clickDialogButton,
  clickTab,
  readJson,
  text,
  waitForToast,
} from "./support.ts";

interface Profile {
  processes: { name: string; action: string; enabled: boolean }[];
  services: { name: string; enabled: boolean }[];
  keep_alive: string[];
}

function savedProfile(): Profile {
  return readJson<{ profile: Profile }>("settings.json")!.profile;
}

/** What the row for this find says it will do, or null while it is not listed. */
async function actionOf(name: string): Promise<string | null> {
  return browser.execute((wanted: string) => {
    for (const row of Array.from(document.querySelectorAll("#scan-rows tr"))) {
      const cells = row.querySelectorAll("td");
      if (cells[1]?.textContent?.trim() === wanted)
        return cells[2]?.textContent?.trim() ?? "";
    }
    return null;
  }, name);
}

async function waitForFind(name: string, listed = true): Promise<void> {
  await browser.waitUntil(
    async () => ((await actionOf(name)) !== null) === listed,
    {
      timeout: 15_000,
      timeoutMsg: `${name} ${listed ? "was never" : "is still"} listed by the scan`,
    },
  );
}

/** The Park list once it has heard from the machine (a service shows what it is called). */
async function openParkList(): Promise<void> {
  await clickTab("targets");
  await $("#service-targets .display-name").waitForExist({
    timeout: 10_000,
    timeoutMsg: "the Park list never heard from the machine",
  });
}

describe("Scan row actions", () => {
  let original: Profile;

  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    original = savedProfile();
    await clickTab("scan");
    await waitForFind("WSearch");
  });

  it("Never touch on a service keeps it off the scan, and leaves unsaved edits alone", async () => {
    await openParkList();
    await $('input[aria-label="Enable SysMain"]').click();
    assert.match(await text("#targets-status"), /Unsaved/);

    await clickTab("scan");
    await waitForFind("WSearch");
    await $('button[aria-label="Never touch WSearch"]').click();
    await waitForToast(/WSearch will never be touched/);
    await waitForFind("WSearch", false);

    const saved = savedProfile();
    assert.ok(saved.keep_alive.includes("WSearch"), saved.keep_alive.join());
    assert.equal(
      saved.services.find((service) => service.name === "SysMain")?.enabled,
      true,
      "Scan saves the list as it was saved, not the unsaved edit",
    );

    await openParkList();
    assert.ok(
      await $('button[aria-label="Stop protecting WSearch"]').isExisting(),
      "the promise is on show, and can be taken back",
    );
    assert.equal(
      await $('input[aria-label="Enable SysMain"]').isSelected(),
      false,
      "the unsaved edit survives",
    );
    assert.match(await text("#targets-status"), /Unsaved/);
    await $('input[aria-label="Enable SysMain"]').click();
    assert.equal(await text("#targets-status"), "");
  });

  it("Never touch on a program does the same", async () => {
    await clickTab("scan");
    await waitForFind("GoogleUpdate.exe");
    await $('button[aria-label="Never touch GoogleUpdate.exe"]').click();
    await waitForToast(/GoogleUpdate\.exe will never be touched/);
    await waitForFind("GoogleUpdate.exe", false);
    assert.ok(savedProfile().keep_alive.includes("GoogleUpdate.exe"));
    assert.ok(
      !savedProfile().processes.some((t) => t.name === "GoogleUpdate.exe"),
    );
  });

  it("offers neither button on a row that is already on the list", async () => {
    await waitForFind("OneDrive.exe");
    assert.equal(
      await $('button[aria-label="Never touch OneDrive.exe"]').isExisting(),
      false,
    );
    assert.equal(
      await $('button[aria-label="Close OneDrive.exe instead"]').isExisting(),
      false,
    );
  });

  it("Close instead switches a find between suspending and closing", async () => {
    await waitForFind("render-farm.exe");
    assert.equal(await actionOf("render-farm.exe"), "Suspend");
    await $('button[aria-label="Close render-farm.exe instead"]').click();
    assert.equal(await actionOf("render-farm.exe"), "Close & relaunch");
    await $('button[aria-label="Suspend render-farm.exe instead"]').click();
    assert.equal(await actionOf("render-farm.exe"), "Suspend");
    await $('button[aria-label="Close render-farm.exe instead"]').click();
    assert.equal(await actionOf("render-farm.exe"), "Close & relaunch");
    assert.equal(
      await $(
        'input[aria-label="Close & relaunch render-farm.exe"]',
      ).isExisting(),
      true,
      "the tick box is named for what it now does",
    );
  });

  it("closing a medium-risk find asks first, and Cancel adds nothing", async () => {
    await $('input[aria-label="Close & relaunch render-farm.exe"]').click();
    await $("#scan-apply").click();
    await $(".dialog-overlay").waitForExist({ timeout: 5_000 });
    assert.match(await text(".dialog p"), /render-farm\.exe/);
    await clickDialogButton("Cancel");
    await $(".dialog-overlay").waitForExist({ reverse: true, timeout: 5_000 });
    assert.ok(
      !savedProfile().processes.some((t) => t.name === "render-farm.exe"),
      "cancelled, so nothing was added",
    );
    assert.equal(await actionOf("render-farm.exe"), "Close & relaunch");
  });

  it("adds it as a close once the user says so", async () => {
    await $("#scan-apply").click();
    await clickDialogButton("Add and close");
    await browser.waitUntil(
      async () =>
        savedProfile().processes.some(
          (t) => t.name === "render-farm.exe" && t.action === "close",
        ),
      {
        timeout: 10_000,
        timeoutMsg: "render-farm.exe was not saved as a close",
      },
    );
    await waitForFind("render-farm.exe");
    assert.equal(await actionOf("render-farm.exe"), "already on the list");
  });

  it("leaves the Park list as it found it", async () => {
    await openParkList();
    // Taking a row off the list puts its name under Never touch; take that back too.
    await $('button[aria-label="Remove render-farm.exe"]').click();
    for (const name of ["render-farm.exe", "GoogleUpdate.exe", "WSearch"])
      await $(`button[aria-label="Stop protecting ${name}"]`).click();
    await $("#targets-save").click();
    await browser.waitUntil(
      async () => (await text("#targets-status")) === "",
      { timeout: 5_000, timeoutMsg: "the save did not settle" },
    );
    assert.deepEqual(savedProfile(), original);
    await clickTab("dashboard");
  });
});
