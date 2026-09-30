/**
 * Launching the app again from the suite, as a shortcut, a script or a
 * launcher would. Not named `*.spec.ts`, so the runner does not open a
 * session for it.
 */
import { spawn } from "node:child_process";

import { application } from "../wdio.conf.ts";

/**
 * Launch the app with `args`, in this run's environment unless `env` says
 * otherwise; resolves with its exit code. A launch that finds a copy running
 * exits once that copy has taken its request.
 */
export function launchAgain(
  args: string[] = [],
  env: NodeJS.ProcessEnv = process.env,
): Promise<number | null> {
  return new Promise((resolve, reject) => {
    const child = spawn(application, args, {
      stdio: "ignore",
      windowsHide: true,
      env,
    });
    const gaveUp = setTimeout(() => {
      child.kill();
      reject(new Error(`the launch with [${args}] did not exit within 20 s`));
    }, 20_000);
    child.once("error", reject);
    child.once("exit", (code) => {
      clearTimeout(gaveUp);
      resolve(code);
    });
  });
}
