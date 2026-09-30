import assert from "node:assert/strict";
import test from "node:test";

import type { EndingState } from "./bridge.ts";
import {
  MAX_MINUTES,
  asked,
  endingLine,
  extendedMinutes,
  secondsLeft,
} from "./ending-text.ts";

const timer = (seconds_left: number | null): EndingState => ({
  kind: "timer",
  program: null,
  seconds_left,
});

test("a choice asks for nothing, minutes, or a program, and nothing else", () => {
  assert.deepEqual(asked("none", ""), { ok: true, until: null });
  assert.deepEqual(asked("120", ""), {
    ok: true,
    until: { kind: "minutes", minutes: 120 },
  });
  assert.deepEqual(asked("program", "  game.exe "), {
    ok: true,
    until: { kind: "program_exits", name: "game.exe" },
  });
  for (const bad of ["", "0", "-5", "1.5", "soon", String(MAX_MINUTES + 1)])
    assert.equal(asked(bad, "x").ok, false, bad);
});

test("a program to wait for has to be named", () => {
  const nothing = asked("program", "   ");
  assert.equal(nothing.ok, false);
  assert.equal(
    nothing.ok ? "" : nothing.reason,
    "Choose a program to wait for.",
  );
});

test("a timer counts down from the engine's figure and stops at zero", () => {
  assert.equal(secondsLeft(timer(7200), 0), 7200);
  assert.equal(secondsLeft(timer(7200), 60), 7140);
  assert.equal(secondsLeft(timer(30), 500), 0);
  // A clock that steps back does not add time.
  assert.equal(secondsLeft(timer(100), -50), 100);
  assert.equal(secondsLeft(timer(null), 0), null);
  assert.equal(
    secondsLeft(
      { kind: "program", program: "game.exe", seconds_left: null },
      5,
    ),
    null,
  );
});

test("every way a run can end reads as a sentence", () => {
  assert.equal(endingLine(timer(7140), 7140), "Ends by itself in 1 h 59 min.");
  assert.equal(endingLine(timer(0), 0), "Ending now.");
  assert.equal(
    endingLine(
      { kind: "program", program: "game.exe", seconds_left: null },
      null,
    ),
    "Ends by itself once game.exe has closed.",
  );
  assert.equal(
    endingLine({ kind: "trigger", program: "steam", seconds_left: null }, null),
    "Started because steam is running. Ends by itself once none of your auto-quiet programs is.",
  );
});

test("adding an hour rounds the time left up and stays within the limit", () => {
  assert.equal(extendedMinutes(7140), 179);
  assert.equal(extendedMinutes(1), 61);
  assert.equal(extendedMinutes(MAX_MINUTES * 60), MAX_MINUTES);
});
