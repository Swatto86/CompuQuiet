import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { checkSignatures, keyId, readSignatures } from "./check-update-key.mjs";

// The layout Tauri writes, with made-up key material: two bytes of algorithm,
// the eight-byte key ID, then the key (32 bytes) or signature (64 bytes).
const wrap = (comment, algorithm, id, body) =>
  Buffer.from(
    `untrusted comment: ${comment}\n` +
      `${Buffer.concat([Buffer.from(algorithm), Buffer.from(id, "hex"), Buffer.alloc(body)]).toString("base64")}\n` +
      "trusted comment: timestamp:1\nAAAA\n",
  ).toString("base64");

const publicKey = (id) => wrap("minisign public key", "Ed", id, 32);
const signature = (id) => wrap("signature from tauri secret key", "ED", id, 64);

test("the key ID is read from a public key and from a signature alike", () => {
  assert.equal(keyId(publicKey("7e1681cbf229bb5e")), "7e1681cbf229bb5e");
  assert.equal(keyId(signature("7e1681cbf229bb5e")), "7e1681cbf229bb5e");
});

test("signatures made by the trusted key pass", () => {
  const id = "0dd8752d32b105ab";
  const trusted = checkSignatures(publicKey(id), {
    "nsis/setup.exe.sig": signature(id),
    "appimage/app.AppImage.sig": signature(id),
  });
  assert.equal(trusted, id);
});

test("a signature from another key is refused, naming both keys", () => {
  assert.throws(
    () =>
      checkSignatures(publicKey("0dd8752d32b105ab"), {
        "nsis/setup.exe.sig": signature("7e1681cbf229bb5e"),
      }),
    /setup\.exe\.sig was signed by key 7e1681cbf229bb5e, but the app trusts key 0dd8752d32b105ab/,
  );
});

test("finding no signature is a failure, not a pass", () => {
  assert.throws(
    () => checkSignatures(publicKey("0dd8752d32b105ab"), {}),
    /no signatures/,
  );
});

test("a file that is not a signature is refused", () => {
  assert.throws(
    () =>
      checkSignatures(publicKey("0dd8752d32b105ab"), {
        "bad.sig": Buffer.from("just some text").toString("base64"),
      }),
    /bad\.sig: not a minisign/,
  );
});

test("the public key in the app's configuration can be read", () => {
  const config = JSON.parse(
    fs.readFileSync(
      new URL("../src-tauri/tauri.conf.json", import.meta.url),
      "utf8",
    ),
  );
  assert.match(keyId(config.plugins.updater.pubkey), /^[0-9a-f]{16}$/);
});

test("signatures are found in nested folders and nothing else is", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "compuquiet-sigs-"));
  fs.mkdirSync(path.join(dir, "nsis"));
  fs.writeFileSync(path.join(dir, "nsis", "setup.exe.sig"), "sig");
  fs.writeFileSync(path.join(dir, "nsis", "setup.exe"), "installer");
  assert.deepEqual(Object.keys(readSignatures(dir)), [
    path.join("nsis", "setup.exe.sig"),
  ]);
});
