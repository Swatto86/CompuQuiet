// A release must be signed with the key the app trusts. The updater in every
// installed copy accepts only bundles signed by `plugins.updater.pubkey`, so a
// GitHub secret that was regenerated (or a public key that was not) publishes
// a release that no installed copy will accept, and nothing says so but a log
// line in each copy.
//
// Tauri wraps a minisign public key and a minisign signature in base64. Each
// holds, on its second line, a further base64 string: two bytes of algorithm,
// the eight-byte key ID, then the key or signature. Comparing the key IDs is
// enough to catch the mismatch; it is not a signature verification.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** The key ID, as hex, of a base64-wrapped minisign public key or signature. */
export function keyId(wrapped) {
  const lines = Buffer.from(wrapped.trim(), "base64")
    .toString("utf8")
    .split(/\r?\n/);
  const raw = Buffer.from(lines[1] ?? "", "base64");
  if (raw.length < 10)
    throw new Error("not a minisign public key or signature");
  return raw.subarray(2, 10).toString("hex");
}

/** Throws unless every signature was made by the key `pubkey` names. */
export function checkSignatures(pubkey, signatures) {
  const names = Object.keys(signatures);
  if (names.length === 0) throw new Error("no signatures were found to check");
  const trusted = keyId(pubkey);
  for (const name of names) {
    let signer;
    try {
      signer = keyId(signatures[name]);
    } catch (error) {
      throw new Error(`${name}: ${error.message}`);
    }
    if (signer !== trusted) {
      throw new Error(
        `${name} was signed by key ${signer}, but the app trusts key ${trusted}. ` +
          "Set TAURI_SIGNING_PRIVATE_KEY to the matching private key, or update " +
          "plugins.updater.pubkey (installed copies would then need a reinstall).",
      );
    }
  }
  return trusted;
}

/** Every `.sig` file under a directory, keyed by its path relative to it. */
export function readSignatures(dir) {
  const found = {};
  for (const name of fs.readdirSync(dir, { recursive: true })) {
    if (String(name).endsWith(".sig")) {
      found[name] = fs.readFileSync(path.join(dir, name), "utf8");
    }
  }
  return found;
}

const isMain =
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const [config, dir] = process.argv.slice(2);
  if (!config || !dir) {
    console.error("usage: check-update-key.mjs <tauri.conf.json> <bundle-dir>");
    process.exit(1);
  }
  try {
    const pubkey = JSON.parse(fs.readFileSync(config, "utf8")).plugins.updater
      .pubkey;
    const signatures = readSignatures(dir);
    const trusted = checkSignatures(pubkey, signatures);
    console.log(
      `${Object.keys(signatures).length} signature(s) match the app's update key ${trusted}`,
    );
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
