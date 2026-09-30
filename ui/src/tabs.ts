/** The tab bar: which view shows, moving between tabs by keyboard, Ctrl+1..4. */
import { dialogOpen } from "./dialog.ts";

export interface Tabs {
  /** Mark a tab as holding changes that are not saved yet. */
  setUnsaved(view: string, unsaved: boolean): void;
}

export function wireTabs(onShow: (view: string) => void): Tabs {
  const tabs = [...document.querySelectorAll<HTMLButtonElement>(".tab")];
  const viewOf = (tab: HTMLElement | undefined) =>
    tab?.dataset["view"] ?? "dashboard";

  // Only the selected tab is in the Tab order; the arrow keys move between
  // the others, and Enter or Space opens the one that has focus.
  const rove = (current: HTMLButtonElement) => {
    for (const tab of tabs) tab.tabIndex = tab === current ? 0 : -1;
  };
  const show = (view: string) => {
    for (const tab of tabs) {
      const selected = viewOf(tab) === view;
      tab.setAttribute("aria-selected", String(selected));
      if (selected) rove(tab);
    }
    for (const section of document.querySelectorAll<HTMLElement>(".view")) {
      section.hidden = section.id !== `view-${view}`;
    }
    onShow(view);
  };
  for (const tab of tabs) {
    tab.addEventListener("click", () => show(viewOf(tab)));
    tab.addEventListener("keydown", (event) => {
      const at = tabs.indexOf(tab);
      const target = {
        ArrowRight: tabs[(at + 1) % tabs.length],
        ArrowLeft: tabs[(at - 1 + tabs.length) % tabs.length],
        Home: tabs[0],
        End: tabs[tabs.length - 1],
      }[event.key];
      if (!target) return;
      event.preventDefault();
      rove(target);
      target.focus();
    });
  }
  for (const jump of document.querySelectorAll<HTMLButtonElement>(
    "[data-jump]",
  )) {
    jump.addEventListener("click", () =>
      show(jump.dataset["jump"] ?? "dashboard"),
    );
  }
  document.addEventListener("keydown", (event) => {
    if (dialogOpen() || !event.ctrlKey || event.key < "1" || event.key > "4")
      return;
    const tab = tabs[Number(event.key) - 1];
    if (tab) {
      event.preventDefault();
      show(viewOf(tab));
      tab.focus();
    }
  });

  return {
    setUnsaved(view, unsaved) {
      const tab = tabs.find((candidate) => viewOf(candidate) === view);
      if (!tab) return;
      tab.classList.toggle("unsaved", unsaved);
      if (unsaved)
        tab.setAttribute(
          "aria-label",
          `${tab.textContent ?? view}, unsaved changes`,
        );
      else tab.removeAttribute("aria-label");
    },
  };
}
