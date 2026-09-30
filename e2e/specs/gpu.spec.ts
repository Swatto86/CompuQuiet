/** Home shows the graphics card's memory, and says so when it cannot be read. */
import { strict as assert } from "node:assert";

import { fakeGpu, text, texts } from "./support.ts";

const GIB = 1024 ** 3;
/** The card the fake machine starts with, put back for the specs after this. */
const SEEDED = { name: "Fake GPU", used: 3 * GIB, total: 24 * GIB };

// The gauge asks every five seconds.
const POLL = 15_000;

async function waitForNames(expected: string[]): Promise<void> {
  await browser.waitUntil(
    async () =>
      JSON.stringify(await texts("#gpu-list .gpu-name")) ===
      JSON.stringify(expected),
    {
      timeout: POLL,
      timeoutMsg: `the gauge never showed ${expected.join(", ")}`,
    },
  );
}

describe("GPU memory", () => {
  after(async () => {
    await fakeGpu([SEEDED]);
  });

  it("shows the card's memory used of its total", async () => {
    await waitForNames(["Fake GPU"]);
    assert.equal(
      await text("#gpu-list .gpu-row .stat-value"),
      "3.0 GB of 24.0 GB",
    );
    const meter = $("#gpu-list [role=meter]");
    assert.equal(await meter.getAttribute("aria-valuenow"), "13");
    assert.equal(await meter.getAttribute("aria-label"), "Fake GPU memory");
  });

  it("shows every adapter the machine has and follows their memory", async () => {
    await fakeGpu([
      { name: "Card A", used: 1 * GIB, total: 8 * GIB },
      { name: "Card B", used: 8 * GIB, total: 8 * GIB },
    ]);
    await waitForNames(["Card A", "Card B"]);
    assert.deepEqual(await texts("#gpu-list .gpu-row .stat-value"), [
      "1.0 GB of 8.0 GB",
      "8.0 GB of 8.0 GB",
    ]);

    await fakeGpu([{ name: "Card A", used: 4 * GIB, total: 8 * GIB }]);
    await waitForNames(["Card A"]);
    assert.equal(
      await text("#gpu-list .gpu-row .stat-value"),
      "4.0 GB of 8.0 GB",
    );
  });

  it("says why when nothing can be read, rather than showing zero", async () => {
    await fakeGpu(null);
    await browser.waitUntil(
      async () => (await texts("#gpu-list .gpu-row")).length === 0,
      { timeout: POLL, timeoutMsg: "the gauge never gave way to a reason" },
    );
    assert.match(await text("#gpu-list"), /cannot be read/);
  });
});
