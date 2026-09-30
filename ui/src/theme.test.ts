import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const css = readFileSync(new URL("./theme.css", import.meta.url), "utf8");

/** The custom properties declared in the block that starts with `opening`. */
function tokens(opening: string): Map<string, string> {
  const start = css.indexOf(opening);
  assert.ok(start >= 0, `theme.css has no ${opening} block`);
  const body = css
    .slice(start + opening.length, css.indexOf("}", start))
    .replace(/\/\*[\s\S]*?\*\//g, "");
  const found = new Map<string, string>();
  for (const statement of body.split(";")) {
    const at = statement.indexOf(":");
    const name = statement.slice(0, at).trim();
    if (at > 0 && name.startsWith("--"))
      found.set(
        name,
        statement
          .slice(at + 1)
          .replace(/\s+/g, " ")
          .trim(),
      );
  }
  return found;
}

const dark = tokens(":root {");
const light = tokens(':root[data-theme="light"] {');
const followsSystem = tokens(
  ':root:not([data-theme="dark"]):not([data-theme="light"]) {',
);

function luminance(hex: string): number {
  const value = parseInt(hex.slice(1), 16);
  const [r, g, b] = [value >> 16, (value >> 8) & 255, value & 255].map((c) => {
    const channel = c / 255;
    return channel <= 0.03928
      ? channel / 12.92
      : ((channel + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [
    number,
    number,
  ];
  return (high + 0.05) / (low + 0.05);
}

function colour(theme: Map<string, string>, name: string): string {
  const value = theme.get(name);
  assert.match(value ?? "", /^#[0-9a-f]{6}$/, `${name} is not a hex colour`);
  return value as string;
}

test("the light theme is declared twice and both copies agree", () => {
  assert.ok(light.size > 10, "the light block was not read");
  assert.deepEqual([...followsSystem], [...light]);
});

test("text colours meet 4.5:1 on every surface they sit on, in both themes", () => {
  const surfaces = ["--bg", "--bg-elev", "--surface", "--surface-2"];
  const text = [
    "--fg",
    "--fg-muted",
    "--accent-text",
    "--accent-2-text",
    "--ok-text",
    "--warn-text",
    "--danger-text",
  ];
  for (const [name, theme] of [
    ["dark", dark],
    ["light", light],
  ] as const) {
    for (const foreground of text) {
      for (const surface of surfaces) {
        const ratio = contrast(
          colour(theme, foreground),
          colour(theme, surface),
        );
        assert.ok(
          ratio >= 4.5,
          `${foreground} on ${surface} in the ${name} theme is ${ratio.toFixed(2)}:1`,
        );
      }
    }
    const filled = contrast(
      colour(theme, "--on-danger"),
      colour(theme, "--danger"),
    );
    assert.ok(
      filled >= 4.5,
      `text on a danger fill in the ${name} theme is ${filled.toFixed(2)}:1`,
    );
  }
});

test("the check fails for the colours it was written to catch", () => {
  // The brand orange and green, as they were used for small text on white.
  assert.ok(contrast("#d97706", "#ffffff") < 4.5);
  assert.ok(contrast("#059669", "#ffffff") < 4.5);
});
