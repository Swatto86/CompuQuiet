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

export async function waitForPill(
  expected: string,
  timeout = 30_000,
): Promise<void> {
  await browser.waitUntil(
    async () => (await text("#status-pill")) === expected,
    {
      timeout,
      timeoutMsg: `the status pill never read "${expected}" (it reads "${await text("#status-pill")}")`,
    },
  );
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

/** Hide or show the main window, and read whether it is showing. */
export async function setWindowVisible(visible: boolean): Promise<void> {
  await invokeCommand(`plugin:window|${visible ? "show" : "hide"}`, {
    label: "main",
  });
}

export function windowVisible(): Promise<boolean> {
  return invokeCommand<boolean>("plugin:window|is_visible", { label: "main" });
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
