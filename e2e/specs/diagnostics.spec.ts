/**
 * Copy diagnostics puts one report on the clipboard: what this copy is, what
 * Quiet Mode did and could not undo, and the log's last lines, with the home
 * folder hidden and no program's path or command line. The spec stands in for
 * the clipboard, so that running the suite never overwrites what the person
 * running it has copied.
 */
import { strict as assert } from "node:assert";
import os from "node:os";

import {
  clickTab,
  dataDir,
  fakeFail,
  fakeHeal,
  text,
  waitForPill,
} from "./support.ts";

interface PageClipboard {
  text?: string;
  refuse: boolean;
}

/**
 * What the page has written to the stand-in clipboard: null until it has.
 * (WebDriver hands `undefined` back as null, so a check for `undefined` would
 * pass at once.)
 */
const written = (): Promise<string | null> =>
  browser.execute(
    () =>
      (window as unknown as { __clipboard: PageClipboard }).__clipboard.text ??
      null,
  );

/**
 * Replace what the page writes with: a note of the text, or a refusal, as a
 * webview gives when the document is not focused. Fails if the page has no
 * clipboard at all, which would send the real button to its fallback.
 */
async function standInForTheClipboard(): Promise<void> {
  await browser.execute(() => {
    const page = window as unknown as { __clipboard: PageClipboard };
    page.__clipboard = { refuse: false };
    Object.defineProperty(navigator.clipboard, "writeText", {
      configurable: true,
      value: (copied: string) => {
        if (page.__clipboard.refuse) {
          return Promise.reject(
            new DOMException("Document is not focused.", "NotAllowedError"),
          );
        }
        page.__clipboard.text = copied;
        return Promise.resolve();
      },
    });
  });
}

async function refuseTheClipboard(refuse: boolean): Promise<void> {
  await browser.execute((on: boolean) => {
    (window as unknown as { __clipboard: PageClipboard }).__clipboard.refuse =
      on;
  }, refuse);
}

/** Press the button and return what reached the clipboard. */
async function copyDiagnostics(): Promise<string> {
  await clickTab("settings");
  await browser.execute(() => {
    (window as unknown as { __clipboard: PageClipboard }).__clipboard.text =
      undefined;
  });
  await $("#diagnostics-copy").click();
  let copied: string | null = null;
  await browser.waitUntil(async () => (copied = await written()) !== null, {
    timeout: 10_000,
    timeoutMsg: "nothing reached the clipboard",
  });
  return copied ?? "";
}

describe("copy diagnostics", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    await standInForTheClipboard();
  });

  after(async () => {
    await fakeHeal();
    await clickTab("dashboard");
  });

  it("copies a report of this copy with the home folder hidden", async () => {
    const version = (await text("#about-version")).replace(/^v/, "");
    assert.match(version, /^\d+\.\d+\.\d+$/);

    const report = await copyDiagnostics();

    assert.match(report, /^CompuQuiet diagnostics$/m);
    assert.match(
      report,
      new RegExp(
        `^Version: ${version.replaceAll(".", "\\.")} \\(development build\\)$`,
        "m",
      ),
    );
    assert.match(report, /^Quiet Mode: off$/m);
    assert.match(report, /^Settings file: readable$/m);
    assert.match(report, /automatic updates yes$/m);
    assert.match(report, /^Updates: this copy cannot update itself/m);
    assert.match(await text("#toast"), /Diagnostics copied/);

    const home = os.homedir().toLowerCase();
    assert.ok(
      !report.toLowerCase().includes(home),
      `the home folder ${home} is in the report`,
    );
    const folder = dataDir();
    if (folder.toLowerCase().startsWith(home)) {
      assert.ok(
        report.includes(`Data folder: ~${folder.slice(home.length)}`),
        report,
      );
    } else {
      assert.ok(report.includes(`Data folder: ${folder}`), report);
    }
  });

  it("names what Quiet Mode did and what could not be put back, never a program's path", async () => {
    await clickTab("dashboard");
    await $("#toggle").click();
    await waitForPill("Quiet");
    await fakeFail("start_service", "SysMain", "refused");
    await $("#toggle").click();
    await $("#recovery").waitForDisplayed({ timeout: 15_000 });

    const report = await copyDiagnostics();

    assert.match(report, /^Quiet Mode: on since \d{4}-\d\d-\d\dT/m);
    assert.match(
      report,
      /^On record in the journal \(1\):\n {2}- stopped service SysMain$/m,
    );
    assert.match(report, /^Not put back \(1\):$/m);
    assert.match(
      report,
      /^ {2}- Start service SysMain: SysMain stays stopped.* \(last error: .+\)$/m,
    );
    assert.match(report, /^Last run \(\d+ lines\):$/m);
    assert.match(
      report,
      /^compuquiet\.log \(warnings and errors.*\):\n(?:.*\n)*.*Start service SysMain failed \(platform\)/m,
      "the log's last lines are included",
    );
    assert.ok(
      !report.includes("C:/fake/"),
      "a program's path is in the report",
    );

    await fakeHeal();
    await clickTab("dashboard");
    await $("#toggle").click();
    await waitForPill("Ready");
  });

  it("shows the report to copy by hand when the clipboard refuses", async () => {
    await refuseTheClipboard(true);
    await clickTab("settings");
    await $("#diagnostics-copy").click();
    await $("#diagnostics-text").waitForDisplayed({ timeout: 10_000 });

    const shown = await $("#diagnostics-text").getValue();
    assert.match(shown, /^CompuQuiet diagnostics\n/);
    assert.match((await $("#toast").getAttribute("class")) ?? "", /error/);
    assert.match(await text("#toast"), /shown below to copy by hand/);

    // Once the clipboard works again the hand-copy box goes away.
    await refuseTheClipboard(false);
    await copyDiagnostics();
    await $("#diagnostics-text").waitForDisplayed({
      reverse: true,
      timeout: 5_000,
    });
  });
});
