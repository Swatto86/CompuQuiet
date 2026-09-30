/**
 * Shared helpers for the specs. Not named `*.spec.ts`: the runner globs for
 * that suffix and a helper picked up as a spec would open an app session
 * that asserts nothing.
 */
import fs from "node:fs";
import path from "node:path";

import { DATA_DIR_ENV } from "../workspace.ts";

export function dataDir(): string {
  const dir = process.env[DATA_DIR_ENV];
  if (!dir)
    throw new Error(`${DATA_DIR_ENV} is not set; onPrepare did not run`);
  return dir;
}

export function readJson<T>(name: string): T | undefined {
  const file = path.join(dataDir(), name);
  if (!fs.existsSync(file)) return undefined;
  return JSON.parse(fs.readFileSync(file, "utf8")) as T;
}

export async function clickTab(
  view: "dashboard" | "targets" | "settings" | "scan",
): Promise<void> {
  await $(`#tab-${view}`).click();
  await browser.waitUntil(
    async () => !(await $(`#view-${view}`).getAttribute("hidden")),
    {
      timeout: 5_000,
      timeoutMsg: `the ${view} view did not appear`,
    },
  );
}

/**
 * The DOM text, not the rendered text: `getText()` applies CSS, so a pill
 * styled `text-transform: uppercase` reads "IDLE" and the app's own wording
 * cannot be asserted through it.
 */
export async function text(selector: string): Promise<string> {
  return browser.execute(
    (sel: string) => document.querySelector(sel)?.textContent?.trim() ?? "",
    selector,
  );
}

/** The DOM text of every element matching `selector`, in order. */
export async function texts(selector: string): Promise<string[]> {
  return browser.execute(
    (sel: string) =>
      Array.from(document.querySelectorAll(sel)).map(
        (element) => element.textContent?.trim() ?? "",
      ),
    selector,
  );
}

export async function waitForPill(
  expected: string,
  timeout = 30_000,
): Promise<void> {
  try {
    await browser.waitUntil(
      async () => (await text("#status-pill")) === expected,
      { timeout },
    );
  } catch (cause) {
    // Read the pill now: an option built before the wait would report the
    // state from before the click, not the one that never changed.
    const now = await text("#status-pill").catch(() => "unreadable");
    throw new Error(
      `the status pill never read "${expected}" (it reads "${now}")`,
      { cause },
    );
  }
}

/** Click the in-app dialog's button whose label matches, failing loudly if none. */
export async function clickDialogButton(label: string): Promise<void> {
  await $(".dialog-overlay").waitForExist({ timeout: 5_000 });
  for (const button of await $$(".dialog-overlay .dialog-buttons button")) {
    if ((await button.getText()) === label) {
      await button.click();
      return;
    }
  }
  throw new Error(`the dialog had no "${label}" button`);
}

/**
 * Call a Tauri command straight through the webview's IPC, as the page's own
 * code does, and return what it returns. For commands the UI has no control
 * for: the fake machine's fault injection, and the window's own visibility.
 */
async function invokeCommand<T = void>(
  command: string,
  args: Record<string, unknown>,
): Promise<T> {
  const outcome = await browser.executeAsync(
    (
      name: string,
      payload: Record<string, unknown>,
      done: (outcome?: { value?: unknown; failure?: string }) => void,
    ) => {
      const internals = (
        window as unknown as {
          __TAURI_INTERNALS__: {
            invoke: (cmd: string, body: object) => Promise<unknown>;
          };
        }
      ).__TAURI_INTERNALS__;
      internals.invoke(name, payload).then(
        (value: unknown) => done({ value }),
        (error: unknown) => done({ failure: JSON.stringify(error) }),
      );
    },
    command,
    args,
  );
  if (outcome?.failure !== undefined)
    throw new Error(`${command} failed: ${outcome.failure}`);
  return outcome?.value as T;
}

/**
 * What the webview says when a command is refused for want of a permission.
 * Fails if the command ran, so a grant that should not exist shows up here.
 */
export async function refusedCommand(
  command: string,
  args: Record<string, unknown>,
): Promise<string> {
  try {
    await invokeCommand(command, args);
  } catch (error) {
    return String(error);
  }
  throw new Error(`${command} ran: the page has a permission it should not`);
}

/** Hide or show the main window, and read whether it is showing. */
export async function setWindowVisible(visible: boolean): Promise<void> {
  await invokeCommand(`plugin:window|${visible ? "show" : "hide"}`, {
    label: "main",
  });
}

export function windowVisible(): Promise<boolean> {
  return invokeCommand<boolean>("plugin:window|is_visible", { label: "main" });
}

/** The theme the window frame (title bar, scroll bars) is drawn in. */
export function windowTheme(): Promise<string> {
  return invokeCommand<string>("plugin:window|theme", { label: "main" });
}

/** Send an app event to every listener, the page's own included. */
export function emitEvent(
  event: string,
  payload: unknown = null,
): Promise<void> {
  return invokeCommand("plugin:event|emit", { event, payload });
}

/**
 * Run a tray menu item through the same Rust dispatch the real menu uses:
 * the tray cannot be clicked through WebDriver.
 */
export function trayMenu(
  id: "tray-toggle" | "tray-show" | "tray-quit",
): Promise<void> {
  return invokeCommand("simulate_tray_menu", { id });
}

/** Everything in compuquiet.log so far; empty until the app writes to it. */
export function logText(): string {
  const file = path.join(dataDir(), "compuquiet.log");
  return fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "";
}

/**
 * Make a call on the fake machine fail until `fakeHeal`. Names are spelled as
 * in `cq_platform::fake`: `start_service`, `refused` / `timed_out` /
 * `needs_elevation`; `target` is a service or program name.
 */
export function fakeFail(
  call: string,
  target: string | null,
  failure: string,
): Promise<void> {
  return invokeCommand("fake_fail", { call, target, failure });
}

export function fakeHeal(): Promise<void> {
  return invokeCommand("fake_heal", {});
}

/**
 * Make the fake machine run on battery (true), on mains (false) or have no
 * battery at all (null).
 */
export function fakeBattery(onBattery: boolean | null): Promise<void> {
  return invokeCommand("fake_battery", { onBattery });
}

/**
 * Give the fake machine these graphics adapters (bytes), or none whose memory
 * can be read (null). It starts with one: "Fake GPU", 3 GiB of 24 GiB.
 */
export function fakeGpu(
  adapters: { name: string; used: number; total: number }[] | null,
): Promise<void> {
  return invokeCommand("fake_gpu", { adapters });
}

/** Open or close a program on the fake machine, as the user would. */
export function fakeProgram(name: string, running: boolean): Promise<void> {
  return invokeCommand("fake_program", { name, running });
}

/**
 * Let time pass on the fake machine: its uptime moves on by `seconds`. The
 * wall clock plays no part in a timed run, so this is all the time a spec
 * has to wait for.
 */
export function fakeAdvance(seconds: number): Promise<void> {
  return invokeCommand("fake_advance", { seconds });
}

/** Whether anything is holding the fake machine awake. */
export function fakeAwake(): Promise<boolean> {
  return invokeCommand<boolean>("fake_awake", {});
}

/**
 * Move the fake machine's uptime on by `seconds` at a time until `done`
 * holds. The app looks at the machine twice a second and counts a program's
 * absence from the first look that saw it, so one jump can be over before
 * that look; jumping again after each look cannot miss it.
 */
export async function advanceUntil(
  seconds: number,
  done: () => Promise<boolean>,
  what: string,
  timeout = 20_000,
): Promise<void> {
  await browser.waitUntil(
    async () => {
      if (await done()) return true;
      await fakeAdvance(seconds);
      return false;
    },
    { timeout, interval: 700, timeoutMsg: `${what} never happened` },
  );
}

/** The DOM text of the toast, until it goes. */
export async function waitForToast(
  wanted: RegExp,
  timeout = 10_000,
): Promise<void> {
  await browser.waitUntil(async () => wanted.test(await text("#toast")), {
    timeout,
    timeoutMsg: `no toast matched ${wanted}`,
  });
}

/** Give the element keyboard focus, as a Tab press would. */
export async function focus(selector: string): Promise<void> {
  await browser.execute(
    (sel: string) => (document.querySelector(sel) as HTMLElement).focus(),
    selector,
  );
}

/** What has keyboard focus: its accessible name if it has one, else its id. */
export async function focused(): Promise<string> {
  return browser.execute(() => {
    const element = document.activeElement;
    return element?.getAttribute("aria-label") ?? element?.id ?? "";
  });
}

export async function attribute(
  selector: string,
  name: string,
): Promise<string> {
  return browser.execute(
    (sel: string, key: string) =>
      document.querySelector(sel)?.getAttribute(key) ?? "",
    selector,
    name,
  );
}

/** Where self-updating stands, and the manual check that asks again. */
export type UpdateStatus = { kind: string; reason?: string };

export function updateStatus(): Promise<UpdateStatus> {
  return invokeCommand<UpdateStatus>("update_status", {});
}

export function checkForUpdates(): Promise<UpdateStatus> {
  return invokeCommand<UpdateStatus>("check_for_updates", {});
}

export async function screenshot(name: string): Promise<void> {
  await browser.saveScreenshot(path.join(dataDir(), `${name}.png`));
}

/**
 * What the webview is actually showing, printed before a wait rather than
 * guessed at afterwards. A `tauri://` or `http://tauri.localhost` URL is a
 * production binary; `localhost:5173` is a dev build pointed at a server that
 * is not running; `about:blank` with an empty document is a driver problem.
 */
export async function logWebviewDiagnostics(): Promise<void> {
  const url = await browser
    .getUrl()
    .catch((e: unknown) => `getUrl failed: ${String(e)}`);
  const source = await browser.getPageSource().catch(() => "");
  console.log(`[diagnostic] url=${url} source=${source.length} chars`);
  const handles = await browser.getWindowHandles().catch(() => [] as string[]);
  console.log(`[diagnostic] window handles: ${handles.length}`);
}
