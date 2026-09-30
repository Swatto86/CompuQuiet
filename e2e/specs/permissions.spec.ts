/**
 * The page holds only the permissions `capabilities/main.json` lists: the
 * three calls it makes itself. This build adds four for the suite's own
 * calls (`e2e/tauri.conf.json`); nothing else may be granted. The tray, the
 * app, path and window controls came with `core:default` and were never used,
 * and a script running in the window could have removed the tray icon with
 * them.
 */
import { strict as assert } from "node:assert";

import { refusedCommand, windowVisible } from "./support.ts";

describe("what the page may ask of the shell", () => {
  it("can still ask whether the window is showing", async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
    assert.equal(typeof (await windowVisible()), "boolean");
  });

  it("is refused the tray, the app and window controls it never uses", async () => {
    for (const [command, args] of [
      ["plugin:tray|set_visible", { id: "main", visible: false }],
      ["plugin:tray|remove", { id: "main" }],
      ["plugin:app|version", {}],
      ["plugin:window|title", { label: "main" }],
      ["plugin:window|close", { label: "main" }],
    ] as const) {
      assert.match(await refusedCommand(command, args), /not allowed/, command);
    }
  });
});
