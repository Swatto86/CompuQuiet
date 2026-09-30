/**
 * Counting the page's own IPC and making a command fail, for the specs that
 * check what the window asks the backend to do. Not named `*.spec.ts`, as
 * support.ts is not.
 */

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
