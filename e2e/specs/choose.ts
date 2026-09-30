/**
 * Choosing from a list the way a person does, for the specs that choose. Not
 * named `*.spec.ts`, as support.ts is not.
 */

/**
 * Pick an option as a person does: scroll to the list, take the keyboard
 * focus to it, then choose. WebKit raises `change` only when the pick differs
 * from the choice it last saw when the list took the focus or was opened, and
 * WebDriver's option click does neither, so a page that put the choice back
 * itself (a run that began, a refused save) would hear nothing when the same
 * option is picked again. The list is taken off the focus first so that the
 * focus is always a new one. Fails if the page was not told, rather than
 * leaving the spec to wait for what a silent pick never shows.
 */
export async function choose(selector: string, value: string): Promise<void> {
  const list = $(selector);
  await list.scrollIntoView({ block: "center", inline: "nearest" });
  const before = await browser.execute((sel: string) => {
    const select = document.querySelector(sel) as HTMLSelectElement;
    const page = window as unknown as { __cqChange?: boolean };
    page.__cqChange = false;
    select.addEventListener("change", () => (page.__cqChange = true), {
      once: true,
    });
    select.blur();
    select.focus();
    return select.value;
  }, selector);
  await list.selectByAttribute("value", value);
  const told = await browser.execute(
    () => (window as unknown as { __cqChange?: boolean }).__cqChange === true,
  );
  if (before !== value && !told)
    throw new Error(
      `picking "${value}" from ${selector} raised no change event; the page has the focus: ${await browser.execute(() => document.hasFocus())}`,
    );
}
