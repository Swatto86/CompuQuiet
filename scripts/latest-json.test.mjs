import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { latestManifest } from "./latest-json.mjs";

test("latest.json points at the signed bundle for each platform", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "compuquiet-latest-"));
  const written = {
    "CompuQuiet_1.1.4_x64-setup.exe.sig": "win-sig\n",
    "CompuQuiet_1.1.4_amd64.AppImage.sig": "lin-sig\n",
    "CompuQuiet.app.tar.gz.sig": "mac-sig\n",
    "CompuQuiet-portable-macos-arm64.app.tar.gz": "not signed",
  };
  for (const [name, body] of Object.entries(written)) {
    fs.writeFileSync(path.join(dir, name), body);
  }

  const manifest = latestManifest(dir, "1.1.4", "v1.1.4");
  assert.equal(manifest.version, "1.1.4");
  assert.equal(manifest.platforms["windows-x86_64"].signature, "win-sig\n");
  assert.equal(
    manifest.platforms["windows-x86_64"].url,
    "https://github.com/Swatto86/CompuQuiet/releases/download/v1.1.4/CompuQuiet_1.1.4_x64-setup.exe",
  );
  assert.match(manifest.platforms["linux-x86_64"].url, /AppImage$/);
  assert.match(
    manifest.platforms["darwin-aarch64"].url,
    /\/CompuQuiet\.app\.tar\.gz$/,
  );
  assert.equal(Object.keys(manifest.platforms).length, 3);
});
