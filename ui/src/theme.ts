import { getCurrentWindow } from "@tauri-apps/api/window";

import type { Theme } from "./bridge.ts";

/** The `data-theme` attribute value; `null` removes the override so the OS decides. */
export function themeAttribute(theme: Theme): string | null {
  return theme === "system" ? null : theme;
}

/** What the window itself (title bar, scroll bars) was last told. */
let windowTheme: Theme = "system";

export function applyTheme(theme: Theme): void {
  const value = themeAttribute(theme);
  if (value === null) document.documentElement.removeAttribute("data-theme");
  else document.documentElement.setAttribute("data-theme", value);
  if (theme === windowTheme) return;
  windowTheme = theme;
  // The page follows the setting by itself; the frame around it does not.
  getCurrentWindow()
    .setTheme(theme === "system" ? null : theme)
    .catch((error: unknown) => {
      console.warn("the window frame kept its theme:", error);
    });
}
