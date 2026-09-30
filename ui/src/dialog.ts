/**
 * An in-app dialog and a toast. Browser `confirm` is not used: it cannot be
 * styled, breaks keyboard focus in a webview, and gives no control over the
 * button labels — "Restore and quit" versus "Quit without restoring" is the
 * whole point.
 */

export interface DialogButton {
  label: string;
  value: string;
  primary?: boolean;
  danger?: boolean;
}

export interface DialogOptions {
  title: string;
  body: string;
  buttons: DialogButton[];
  /** Value returned on Escape or a click outside; defaults to the last button. */
  cancel?: string;
}

/** Closes the dialog on show as cancelled; null while none is open. */
let dismissOpen: (() => void) | null = null;
let dialogCount = 0;

/** A dialog is showing, so shortcuts elsewhere on the page must stand aside. */
export function dialogOpen(): boolean {
  return dismissOpen !== null;
}

export function showDialog(options: DialogOptions): Promise<string> {
  const root = document.getElementById("dialog-root");
  if (!root) return Promise.reject(new Error("no #dialog-root"));
  // One at a time. The window's close button can be pressed again, or over
  // another dialog; the newest request replaces the one showing, which ends
  // as cancelled, so nothing stacks and no choice is answered twice.
  dismissOpen?.();
  const previous =
    document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;
  const cancelValue =
    options.cancel ?? options.buttons[options.buttons.length - 1]?.value ?? "";
  // Nothing behind the dialog may take focus, a click or a key.
  const behind = [
    ...(root.parentElement?.querySelectorAll<HTMLElement>(
      ":scope > header, :scope > #banner, :scope > main",
    ) ?? []),
  ];

  return new Promise((resolve) => {
    const id = `dialog-${(dialogCount += 1)}`;
    const overlay = document.createElement("div");
    overlay.className = "dialog-overlay";
    const dialog = document.createElement("div");
    dialog.className = "dialog";
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-modal", "true");
    const title = document.createElement("h2");
    title.textContent = options.title;
    title.id = `${id}-title`;
    dialog.setAttribute("aria-labelledby", title.id);
    const body = document.createElement("p");
    body.textContent = options.body;
    body.id = `${id}-body`;
    dialog.setAttribute("aria-describedby", body.id);
    const buttons = document.createElement("div");
    buttons.className = "dialog-buttons";

    const finish = (value: string) => {
      document.removeEventListener("keydown", onKey);
      dismissOpen = null;
      overlay.remove();
      for (const element of behind) element.inert = false;
      previous?.focus();
      resolve(value);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        finish(cancelValue);
      }
      if (event.key === "Tab") {
        const focusable = [
          ...buttons.querySelectorAll<HTMLButtonElement>("button"),
        ];
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        if (!first || !last) return;
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }
    };

    let focusTarget: HTMLButtonElement | null = null;
    for (const spec of options.buttons) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = spec.label;
      if (spec.primary) button.classList.add("primary");
      if (spec.danger) button.classList.add("danger");
      button.addEventListener("click", () => finish(spec.value));
      buttons.appendChild(button);
      if (spec.primary || !focusTarget) focusTarget = button;
    }

    overlay.addEventListener("click", (event) => {
      if (event.target === overlay) finish(cancelValue);
    });
    dismissOpen = () => finish(cancelValue);
    document.addEventListener("keydown", onKey);
    dialog.append(title, body, buttons);
    overlay.appendChild(dialog);
    root.appendChild(overlay);
    for (const element of behind) element.inert = true;
    focusTarget?.focus();
  });
}

let toastTimer: number | undefined;
let announceTimer: number | undefined;

/**
 * The toast is for the eyes; a screen reader is told through a live region
 * that is always on the page, because a node that appears with its text
 * already set is often not announced.
 */
function announce(message: string, isError: boolean): void {
  const live = document.getElementById("announce");
  if (!live) return;
  live.setAttribute("aria-live", isError ? "assertive" : "polite");
  live.textContent = "";
  window.clearTimeout(announceTimer);
  announceTimer = window.setTimeout(() => {
    live.textContent = isError ? `Error: ${message}` : message;
  }, 50);
}

export function toast(message: string, isError = false): void {
  const element = document.getElementById("toast");
  if (!element) return;
  element.textContent = message;
  element.classList.toggle("error", isError);
  element.hidden = false;
  announce(message, isError);
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(
    () => {
      element.hidden = true;
    },
    isError ? 8000 : 4000,
  );
}
