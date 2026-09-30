/**
 * The rows of the Park list's two tables. Each is built from a target and
 * what to do when it is edited; the Targets view keeps the profile.
 */
import type {
  ProcessAction,
  ProcessRow,
  ProcessTarget,
  ServiceTarget,
} from "./bridge.ts";
import { formatBytes } from "./format.ts";

interface Edits {
  toggle(enabled: boolean): void;
  remove(): void;
}

export function processRow(
  target: ProcessTarget,
  live: ProcessRow | undefined,
  edits: Edits & { act(action: ProcessAction): void },
): HTMLTableRowElement {
  const name = document.createElement("td");
  name.className = "name";
  name.textContent = target.name;
  const action = document.createElement("td");
  const select = document.createElement("select");
  select.setAttribute("aria-label", `Action for ${target.name}`);
  for (const [value, label] of [
    ["suspend", "Suspend"],
    ["close", "Close & relaunch"],
  ] as const) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    option.selected = target.action === value;
    select.appendChild(option);
  }
  select.addEventListener("change", () =>
    edits.act(select.value as ProcessAction),
  );
  action.appendChild(select);
  const now = document.createElement("td");
  const state = document.createElement("span");
  state.className = live ? "state running" : "state";
  state.textContent = live
    ? `${live.instances} running · ${formatBytes(live.memory_bytes)}`
    : "not running";
  now.appendChild(state);
  return row(checkbox(target, edits), name, action, now, remove(target, edits));
}

/** `display` is what the machine calls the service, when it has said. */
export function serviceRow(
  target: ServiceTarget,
  display: string,
  edits: Edits,
): HTMLTableRowElement {
  const name = document.createElement("td");
  name.className = "name";
  name.textContent = target.name;
  if (display !== "" && display !== target.name) {
    const note = document.createElement("div");
    note.className = "display-name";
    note.textContent = display;
    name.appendChild(note);
  }
  return row(checkbox(target, edits), name, remove(target, edits));
}

/** The rows, or one line saying the filter hides every target of a list. */
export function orNote(
  rows: HTMLTableRowElement[],
  listed: number,
  columns: number,
): HTMLTableRowElement[] {
  if (rows.length > 0 || listed === 0) return rows;
  const cell = document.createElement("td");
  cell.colSpan = columns;
  cell.className = "muted";
  cell.textContent = "Nothing on this list matches the filter";
  return [row(cell)];
}

function row(...cells: HTMLTableCellElement[]): HTMLTableRowElement {
  const tr = document.createElement("tr");
  tr.append(...cells);
  return tr;
}

function checkbox(
  target: { name: string; enabled: boolean },
  edits: Edits,
): HTMLTableCellElement {
  const cell = document.createElement("td");
  const input = document.createElement("input");
  input.type = "checkbox";
  input.checked = target.enabled;
  input.setAttribute("aria-label", `Enable ${target.name}`);
  input.addEventListener("change", () => edits.toggle(input.checked));
  cell.appendChild(input);
  return cell;
}

function remove(target: { name: string }, edits: Edits): HTMLTableCellElement {
  const cell = document.createElement("td");
  const button = document.createElement("button");
  button.type = "button";
  button.className = "small ghost";
  button.textContent = "✕";
  button.setAttribute("aria-label", `Remove ${target.name}`);
  button.addEventListener("click", () => edits.remove());
  cell.appendChild(button);
  return cell;
}
