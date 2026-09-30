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
export function emitEvent(event: string): Promise<void> {
  return invokeCommand("plugin:event|emit", { event, payload: null });
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
 * Put a counter and a fault switch in front of the page's own IPC, so a spec
 * can see which commands the page calls and how often, and make one fail as
 * if the backend refused it. Idempotent; lasts until the page reloads.
 *
 * `__TAURI_INTERNALS__.invoke` is locked, so this stands in front of what it
 * sends. Depending on the webview a command goes out as a `fetch` to the
 * `ipc` protocol (an answer marked `Tauri-Response: error` rejects the
 * page's promise) or as a message posted to the host (the error callback it
 * names is run to reject it). WebView2 posts; the other two are written from
 * Tauri's own script and not exercised on this machine.
 */
export async function watchInvokes(): Promise<void> {
  await browser.execute(() => {
    interface Host {
      postMessage: (message: unknown) => void;
    }
    const page = window as unknown as {
      __cq?: { calls: Record<string, number>; failing: Record<string, string> };
      __TAURI_INTERNALS__: {
        runCallback: (id: number, data: unknown) => void;
      };
      chrome?: { webview?: Host };
      webkit?: { messageHandlers?: { ipc?: Host } };
    };
    if (page.__cq) return;
    const state = {
      calls: {} as Record<string, number>,
      failing: {} as Record<string, string>,
    };
    page.__cq = state;
    /** Count the command; the reason to refuse it, if it is set to fail. */
    const note = (command: string): string | undefined => {
      state.calls[command] = (state.calls[command] ?? 0) + 1;
      return state.failing[command];
    };

    const original = window.fetch.bind(window);
    window.fetch = (input, init) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const ipc =
        /^(?:https?:\/\/ipc\.localhost|ipc:\/\/localhost)\/([^?#]+)/.exec(url);
      const reason =
        ipc?.[1] === undefined ? undefined : note(decodeURIComponent(ipc[1]));
      if (reason === undefined) return original(input, init);
      return Promise.resolve(
        new Response(JSON.stringify({ code: "refused", message: reason }), {
          status: 500,
          headers: {
            "Content-Type": "application/json",
            "Tauri-Response": "error",
          },
        }),
      );
    };

    for (const host of [
      page.chrome?.webview,
      page.webkit?.messageHandlers?.ipc,
    ]) {
      if (!host) continue;
      const post = host.postMessage.bind(host);
      host.postMessage = (message) => {
        const sent: { cmd?: string; error?: number } | null =
          typeof message === "string" && message.startsWith("{")
            ? JSON.parse(message)
            : null;
        const reason = sent?.cmd === undefined ? undefined : note(sent.cmd);
        if (reason === undefined || sent?.error === undefined)
          return post(message);
        page.__TAURI_INTERNALS__.runCallback(sent.error, {
          code: "refused",
          message: reason,
        });
      };
    }
  });
}

/** How many times the page has called `command` since `watchInvokes`. */
export function invokeCount(command: string): Promise<number> {
  return browser.execute(
    (name: string) =>
      (window as unknown as { __cq: { calls: Record<string, number> } }).__cq
        .calls[name] ?? 0,
    command,
  );
}

/** Make the page's calls to `command` fail with `message`; null lets them through. */
export async function failInvokes(
  command: string,
  message: string | null,
): Promise<void> {
  await browser.execute(
    (name: string, reason: string | null) => {
      const failing = (
        window as unknown as { __cq: { failing: Record<string, string> } }
      ).__cq.failing;
      if (reason === null) delete failing[name];
      else failing[name] = reason;
    },
    command,
    message,
  );
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
