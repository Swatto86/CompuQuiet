/**
 * Named profiles: one list for a game and another for a model run, chosen on
 * Home, from the tray, from the command line and by the program that starts
 * Quiet Mode. The journal and settings.json on disk are what say which profile
 * a run was made from and which is in use; the window and the tray dispatch
 * are the real ones. It ends with the single profile it began with.
 */
import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";

import { launchAgain } from "./launch.ts";
import { choose } from "./choose.ts";
import {
  advanceUntil,
  clickDialogButton,
  clickTab,
  dataDir,
  fakeProgram,
  readJson,
  text,
  texts,
  trayMenu,
  waitForPill,
  waitForToast,
} from "./support.ts";

interface Saved {
  profile_name: string;
  profile: { processes: { name: string; enabled: boolean }[] };
  other_profiles: {
    name: string;
    profile: { processes: { name: string }[]; keep_alive: string[] };
  }[];
  auto_quiet: {
    enabled?: boolean;
    programs?: string[];
    profiles?: Record<string, string>;
  };
}

interface Journal {
  profile?: string;
  ending?: { kind: string; program?: string };
  done: { kind: string; name?: string }[];
}

const saved = (): Saved => readJson<Saved>("settings.json")!;
const journal = (): Journal | undefined => readJson<Journal>("journal.json");
const parked = (): string[] =>
  (journal()?.done ?? [])
    .filter((step) => step.kind === "process_suspended")
    .map((step) => step.name ?? "");

async function untilSaved(
  what: string,
  done: (settings: Saved) => boolean,
): Promise<void> {
  await browser.waitUntil(() => done(saved()), {
    timeout: 5_000,
    timeoutMsg: `settings.json never ${what}`,
  });
}

/** The profile the page shows as in use, from the Park list's own choice. */
const shown = (): Promise<string> =>
  browser.execute(
    () =>
      (document.getElementById("profile-edit-choice") as HTMLSelectElement)
        .value,
  );

async function addProfile(name: string): Promise<void> {
  await $("#profile-name").setValue(name);
  await $("#profile-add button[type=submit]").click();
}

describe("named profiles", () => {
  before(async () => {
    await $("#toggle").waitForExist({ timeout: 30_000 });
  });

  it("offers no choice while there is one profile", async () => {
    await clickTab("targets");
    assert.deepEqual(await texts("#profile-edit-choice option"), ["Default"]);
    assert.equal(await $("#profile-delete").isEnabled(), false);
    await clickTab("dashboard");
    assert.equal(await $("#profile-bar").isDisplayed(), false);
    assert.doesNotMatch(await text("#hero-plan"), /^Default:/);
    assert.equal(saved().profile_name, "Default");
  });

  it("refuses a name that is empty or already taken, and adds nothing", async () => {
    await clickTab("targets");
    await $("#profile-add button[type=submit]").click();
    await waitForToast(/Give the profile a name/);
    await addProfile("default");
    await waitForToast(/There is a profile called Default already/);
    assert.deepEqual(await texts("#profile-edit-choice option"), ["Default"]);
    assert.deepEqual(saved().other_profiles, []);
  });

  it("adds a profile as a copy, and uses it", async () => {
    await addProfile("Gaming");
    await waitForToast(/Added Gaming, and using it/);
    await untilSaved(
      "made Gaming the profile in use",
      (s) => s.profile_name === "Gaming",
    );
    assert.deepEqual(
      saved().other_profiles.map((other) => other.name),
      ["Default"],
    );
    assert.deepEqual(await texts("#profile-edit-choice option"), [
      "Default",
      "Gaming",
    ]);
    assert.equal(await shown(), "Gaming");

    await clickTab("dashboard");
    assert.equal(await $("#profile-bar").isDisplayed(), true);
    assert.equal(await $("#profile-choice").getValue(), "Gaming");
    assert.match(await text("#hero-plan"), /^Gaming: one press will /);
  });

  it("keeps a park list of its own", async () => {
    await clickTab("targets");
    await $('input[aria-label="Enable Slack"]').click();
    await $("#targets-save").click();
    await untilSaved("saved Gaming without Slack", (s) =>
      s.profile.processes.some(
        (target) => target.name === "Slack" && !target.enabled,
      ),
    );
    // Default was left as it was: it still parks Slack.
    const other = saved().other_profiles[0]!;
    assert.equal(other.name, "Default");
    assert.ok(
      other.profile.processes.some((target) => target.name === "Slack"),
    );
  });

  it("runs the profile in use, says which, and locks the choice while it is on", async () => {
    await clickTab("dashboard");
    await $("#toggle").click();
    await waitForPill("Quiet");
    assert.equal(journal()?.profile, "Gaming");
    assert.ok(parked().length > 0, "Gaming parks something");
    assert.ok(
      !parked().some((name) => name.startsWith("Slack")),
      parked().join(),
    );
    assert.equal(
      await text("#hero-sub"),
      "Gaming profile: background work is parked. Press again when you are done.",
    );
    assert.match(await text("#log"), /Using the Gaming profile/);
    assert.equal(await $("#profile-choice").isEnabled(), false);
    await clickTab("targets");
    assert.equal(await $("#profile-edit-choice").isEnabled(), false);
    assert.equal(await $("#profile-lock").isDisplayed(), true);
    await clickTab("dashboard");

    // The tray refuses too, and says why, and the profile is as it was.
    await trayMenu("tray-profile:Default");
    await waitForToast(/Put everything back before changing profiles/);
    assert.equal(saved().profile_name, "Gaming");

    await $("#toggle").click();
    await waitForPill("Ready");
  });

  it("switches from Home, and the next press runs that profile", async () => {
    await choose("#profile-choice", "Default");
    await waitForToast(/Using Default/);
    await untilSaved(
      "made Default the profile in use",
      (s) => s.profile_name === "Default",
    );
    assert.match(await text("#hero-plan"), /^Default: one press will /);

    await $("#toggle").click();
    await waitForPill("Quiet");
    assert.equal(journal()?.profile, "Default");
    assert.ok(
      parked().some((name) => name.startsWith("Slack")),
      parked().join(),
    );
    await $("#toggle").click();
    await waitForPill("Ready");
  });

  it("switches from the tray, and the window follows", async () => {
    await trayMenu("tray-profile:Gaming");
    await waitForToast(/Now using Gaming/);
    assert.equal(saved().profile_name, "Gaming");
    assert.equal(await $("#profile-choice").getValue(), "Gaming");
    await clickTab("targets");
    assert.equal(await shown(), "Gaming");
    await clickTab("dashboard");

    // A name that is not there changes nothing.
    await trayMenu("tray-profile:Nobody");
    await browser.pause(800);
    assert.equal(saved().profile_name, "Gaming");
  });

  it("does not drop unsaved Park list changes without asking", async () => {
    await clickTab("targets");
    await $('input[aria-label="Enable OneDrive"]').click();
    assert.equal(await text("#targets-status"), "Unsaved changes");

    await choose("#profile-edit-choice", "Default");
    await clickDialogButton("Cancel");
    assert.equal(await shown(), "Gaming", "the choice was put back");
    assert.equal(await text("#targets-status"), "Unsaved changes");
    assert.equal(saved().profile_name, "Gaming");

    await choose("#profile-edit-choice", "Default");
    await clickDialogButton("Discard changes");
    await waitForToast(/Using Default/);
    await untilSaved(
      "made Default the profile in use",
      (s) => s.profile_name === "Default",
    );
    assert.equal(await text("#targets-status"), "");
    await choose("#profile-edit-choice", "Gaming");
    await waitForToast(/Using Gaming/);
  });

  it("runs another profile from the command line without switching to it", async () => {
    await trayMenu("tray-profile:Default");
    await waitForToast(/Now using Default/);

    assert.equal(await launchAgain(["--quiet", "--profile", "gaming"]), 0);
    await browser.waitUntil(() => journal() !== undefined, {
      timeout: 20_000,
      timeoutMsg: "the command line never started Quiet Mode",
    });
    assert.equal(journal()?.profile, "Gaming");
    assert.ok(
      !parked().some((name) => name.startsWith("Slack")),
      parked().join(),
    );
    assert.equal(
      saved().profile_name,
      "Default",
      "the profile in use is as it was",
    );
    assert.equal(await launchAgain(["--restore"]), 0);
    await browser.waitUntil(() => journal() === undefined, {
      timeout: 20_000,
      timeoutMsg: "the command line never put everything back",
    });

    // A profile that is not saved refuses the launch, and nothing starts.
    assert.equal(await launchAgain(["--quiet", "--profile", "Nobody"]), 2);
    assert.equal(await launchAgain(["--profile", "Gaming"]), 2);
    await browser.pause(1_500);
    assert.equal(journal(), undefined);
    const wake = path.join(dataDir(), "wake");
    assert.deepEqual(fs.existsSync(wake) ? fs.readdirSync(wake) : [], []);
  });

  it("starts the profile chosen for the program that starts Quiet Mode", async () => {
    await clickTab("settings");
    await $("#auto-quiet-name").setValue("steam.exe");
    await $("#auto-quiet-add button[type=submit]").click();
    const choice = 'select[aria-label="Profile to start for steam.exe"]';
    await $(choice).waitForExist({ timeout: 5_000 });
    assert.deepEqual(await texts(`${choice} option`), [
      "Profile in use",
      "Default",
      "Gaming",
    ]);
    await choose(choice, "Gaming");
    await untilSaved(
      "kept the choice",
      (s) => s.auto_quiet.profiles?.["steam.exe"] === "Gaming",
    );
    await $("#set-auto-quiet").click();
    await untilSaved(
      "turned auto-quiet on",
      (s) => s.auto_quiet.enabled === true,
    );
    await clickTab("dashboard");

    await fakeProgram("steam.exe", true);
    await advanceUntil(
      11,
      async () => (await text("#status-pill")) === "Quiet",
      "Quiet Mode starting",
    );
    assert.equal(journal()?.profile, "Gaming", "not the profile in use");
    assert.deepEqual(journal()?.ending, {
      kind: "trigger",
      program: "steam.exe",
    });
    assert.equal(saved().profile_name, "Default");

    await fakeProgram("steam.exe", false);
    await advanceUntil(
      31,
      async () => (await text("#status-pill")) === "Ready",
      "the run ending",
    );
  });

  it("puts a removed row under Never touch in every profile, and says so", async () => {
    await trayMenu("tray-profile:Gaming");
    await waitForToast(/Now using Gaming/);
    await clickTab("targets");
    await $("#process-name").setValue("zz-shared.exe");
    await $("#process-add button[type=submit]").click();
    await $('button[aria-label="Remove zz-shared.exe"]').click();
    await waitForToast(
      /zz-shared\.exe is under Never touch now, in every profile/,
    );
    await $("#targets-save").click();
    await untilSaved("kept the name under Never touch", (s) =>
      s.other_profiles[0]!.profile.keep_alive.includes("zz-shared.exe"),
    );
    // Taken off again, so the state after this spec is the state before it.
    await $('button[aria-label="Stop protecting zz-shared.exe"]').click();
    await $("#targets-save").click();
    await untilSaved(
      "let go of the name",
      (s) => !s.other_profiles[0]!.profile.keep_alive.includes("zz-shared.exe"),
    );
  });

  it("renames a profile, and what starts it follows", async () => {
    await $("#profile-name").setValue("Games");
    await $("#profile-rename").click();
    await waitForToast(/Gaming is now called Games/);
    assert.equal(saved().profile_name, "Games");
    assert.equal(saved().auto_quiet.profiles?.["steam.exe"], "Games");
    assert.deepEqual(await texts("#profile-edit-choice option"), [
      "Default",
      "Games",
    ]);
    assert.equal(await shown(), "Games");
  });

  it("deletes a profile after asking, and is back to one", async () => {
    await $("#profile-delete").click();
    await clickDialogButton("Cancel");
    assert.equal(saved().profile_name, "Games", "cancelling deletes nothing");

    await $("#profile-delete").click();
    await clickDialogButton("Delete");
    await waitForToast(/Deleted Games/);
    await untilSaved("deleted Games", (s) => s.other_profiles.length === 0);
    assert.equal(saved().profile_name, "Default");
    assert.equal(saved().auto_quiet.profiles?.["steam.exe"], undefined);
    assert.deepEqual(await texts("#profile-edit-choice option"), ["Default"]);
    assert.equal(await $("#profile-delete").isEnabled(), false);
    await clickTab("dashboard");
    assert.equal(await $("#profile-bar").isDisplayed(), false);

    // The auto-quiet list goes back as it was too.
    await clickTab("settings");
    await $("#set-auto-quiet").click();
    await $("#auto-quiet-list li button").click();
    await untilSaved(
      "put the auto-quiet list back",
      (s) =>
        s.auto_quiet.enabled === false && s.auto_quiet.programs?.length === 0,
    );
    await clickTab("dashboard");
  });
});
