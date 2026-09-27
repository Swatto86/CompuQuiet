// The static manifest Tauri's updater reads from the GitHub release.
// Each platform entry is the signed bundle the bundler produced, not the
// portable copy (that archive is unsigned).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = "https://github.com/Swatto86/CompuQuiet/releases/download";

const platforms = [
  ["windows-x86_64", (name) => name.endsWith("_x64-setup.exe")],
  ["linux-x86_64", (name) => name.endsWith(".AppImage")],
  [
    "darwin-aarch64",
    (name) => name.endsWith(".app.tar.gz") && !name.includes("portable"),
  ],
];

export function latestManifest(dir, version, tag) {
  const names = fs.readdirSync(dir);
  const body = { version, platforms: {} };
  for (const [key, match] of platforms) {
    const sigs = names.filter(
      (name) => name.endsWith(".sig") && match(name.slice(0, -".sig".length)),
    );
    if (sigs.length !== 1) {
      throw new Error(
        `${key}: expected one signed artifact, found ${sigs.join(", ") || "none"}`,
      );
    }
    const artifact = sigs[0].slice(0, -".sig".length);
    body.platforms[key] = {
      signature: fs.readFileSync(path.join(dir, sigs[0]), "utf8"),
      url: `${repo}/${tag}/${encodeURIComponent(artifact)}`,
    };
  }
  return body;
}

const isMain =
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const [dir, version, tag] = process.argv.slice(2);
  if (!dir || !version || !tag) {
    console.error("usage: latest-json.mjs <artifacts-dir> <version> <tag>");
    process.exit(1);
  }
  process.stdout.write(
    `${JSON.stringify(latestManifest(dir, version, tag), null, 2)}\n`,
  );
}
