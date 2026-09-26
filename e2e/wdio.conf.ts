/** Drives the real debug binary (fake platform) through its real webview. */
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { appPids, prepareWorkspace } from "./workspace.ts";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");

export const application =
  process.env["COMPUQUIET_E2E_APPLICATION"] ??
  path.resolve(
    root,
    `target/debug/compuquiet${process.platform === "win32" ? ".exe" : ""}`,
  );

function assertFreshBuild(exe: string): void {
  if (!fs.existsSync(exe)) {
    throw new Error(`no binary at ${exe} — build it with: npm run e2e:build`);
  }
  const built = fs.statSync(exe).mtimeMs;
  const distIndex = path.resolve(root, "ui/dist/index.html");
  if (fs.existsSync(distIndex) && fs.statSync(distIndex).mtimeMs > built) {
    throw new Error(
      `${exe} is older than ui/dist and does not contain the current frontend. Rebuild with: npm run e2e:build`,
    );
  }
}

let tauriDriver: ChildProcess | undefined;

function stopDriver(): void {
  if (tauriDriver?.pid) {
    if (process.platform === "win32") {
      spawnSync("taskkill", ["/pid", String(tauriDriver.pid), "/T", "/F"], {
        stdio: "pipe",
      });
    } else {
      try {
        process.kill(-tauriDriver.pid, "SIGTERM");
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error;
      }
    }
  }
  tauriDriver = undefined;
}

const driverStatus = () =>
  fetch("http://127.0.0.1:4444/status", { signal: AbortSignal.timeout(1_000) });

async function assertDriverPortFree(): Promise<void> {
  try {
    await driverStatus();
  } catch {
    return;
  }
  throw new Error(
    "port 4444 is already held by a WebDriver process; stop the stale tauri-driver before rerunning",
  );
}

async function waitForDriver(): Promise<void> {
  const deadline = Date.now() + 30_000;
  let lastError = "no attempt made";
  while (Date.now() < deadline) {
    if (tauriDriver?.exitCode !== null && tauriDriver?.exitCode !== undefined) {
      throw new Error(
        `tauri-driver exited with code ${tauriDriver.exitCode} before binding 4444`,
      );
    }
    try {
      await driverStatus();
      return;
    } catch (error) {
      lastError = String(error);
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }
  throw new Error(
    `tauri-driver never accepted a connection on 127.0.0.1:4444 within 30s (last: ${lastError})`,
  );
}

function assertMicrosoftDriver(driver: string): void {
  const verifier = path.resolve(root, "scripts/assert-microsoft-signature.ps1");
  const checked = spawnSync(
    "pwsh",
    ["-NoProfile", "-NonInteractive", "-File", verifier, "-Path", driver],
    { stdio: "pipe", windowsHide: true, shell: false },
  );
  if (checked.error || checked.status !== 0) {
    throw new Error(
      `EdgeDriver signature validation failed: ${checked.stderr?.toString().trim() || checked.error?.message}`,
    );
  }
}

export const config: WebdriverIO.Config = {
  runner: "local",
  framework: "mocha",
  specs: [
    ["boot", "quiet", "scan", "persist", "exit"].map((name) =>
      path.resolve(here, `specs/${name}.spec.ts`),
    ),
  ],
  maxInstances: 1,
  logLevel: "error",
  reporters: ["spec"],
  mochaOpts: { ui: "bdd", timeout: 120_000 },
  hostname: "127.0.0.1",
  port: 4444,
  capabilities: [
    {
      // @ts-expect-error tauri:options is a tauri-driver capability.
      "tauri:options": { application },
      browserName: "wry",
      // WebdriverIO 9 otherwise requests a BiDi session, which tauri-driver
      // proxies verbatim to a native driver that answers with a blank page.
      "wdio:enforceWebDriverClassic": true,
    },
  ],

  onPrepare: async () => {
    try {
      assertFreshBuild(application);
      await assertDriverPortFree();
      if (appPids(application).length > 0) {
        throw new Error(
          "the e2e binary is already running; the suite will not terminate it",
        );
      }
      const dataDir = prepareWorkspace();
      console.log(`E2E state: ${dataDir}`);
      const nativeDriver =
        process.platform === "win32"
          ? path.resolve(root, ".webdriver/msedgedriver.exe")
          : (process.env["PATH"] ?? "")
              .split(path.delimiter)
              .map((dir) => path.resolve(dir, "WebKitWebDriver"))
              .find((candidate) => fs.existsSync(candidate));
      if (!nativeDriver)
        throw new Error("Install WebKitWebDriver before running Linux E2E");
      if (process.platform === "win32") assertMicrosoftDriver(nativeDriver);
      tauriDriver = spawn("tauri-driver", ["--native-driver", nativeDriver], {
        stdio: [null, process.stdout, process.stderr],
        shell: false,
        env: {
          ...process.env,
          ...(process.platform === "linux"
            ? { GDK_BACKEND: "x11", WEBKIT_DISABLE_DMABUF_RENDERER: "1" }
            : {}),
        },
        detached: process.platform !== "win32",
      });
      process.once("exit", stopDriver);
      await waitForDriver();
    } catch (error) {
      if (tauriDriver?.pid) tauriDriver.kill();
      console.error(error);
      process.exit(1);
    }
  },

  afterTest: async (_test, _context, { passed }) => {
    if (passed) return;
    const evidence = path.join(
      process.env["RUNNER_TEMP"] ?? process.env["COMPUQUIET_DATA_DIR"]!,
      "compuquiet-failure",
    );
    try {
      await browser.saveScreenshot(`${evidence}.png`);
      const url = await browser.getUrl();
      const source = (await browser.getPageSource()).length;
      console.error(`[diagnostic] url=${url} source=${source} chars`);
    } catch (error) {
      console.error(`[diagnostic] no session to capture: ${String(error)}`);
    }
  },

  onComplete: () => {
    stopDriver();
    console.log(`E2E evidence: ${process.env["COMPUQUIET_DATA_DIR"]}`);
  },
};
