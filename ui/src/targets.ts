/** The profile editor: which programs and services Quiet Mode touches. */
import {
  api,
  errorMessage,
  type Capabilities,
  type Os,
  type ProcessAction,
  type ProcessRow,
  type Profile,
} from "./bridge.ts";
import { showDialog, toast } from "./dialog.ts";
import { formatBytes } from "./format.ts";
import {
  addKeepAlive,
  addProcess,
  addService,
  normalizeName,
  removeKeepAlive,
  removeProcess,
  removeService,
  rebase,
  sameProfile,
  setProcess,
  setService,
  type EditResult,
} from "./profile-edit.ts";

function byId<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing #${id}`);
  return element as T;
}

export interface TargetsHost {
  save(profile: Profile): Promise<void>;
  defaults(): Promise<Profile>;
  /** Whether the list holds edits that are not saved. */
  unsaved(unsaved: boolean): void;
}

/** How long a row that was just added stays highlighted. */
const FRESH_MS = 2500;

/** Programs and services on the list, which protection takes off it. */
function listed(profile: Profile): number {
  return profile.processes.length + profile.services.length;
}

export class Targets {
  private saved: Profile;
  private working: Profile;
  private running = new Map<string, ProcessRow>();

  constructor(
    initial: Profile,
    private readonly host: TargetsHost,
  ) {
    this.saved = initial;
    this.working = structuredClone(initial);
    byId<HTMLFormElement>("process-add").addEventListener("submit", (event) => {
      event.preventDefault();
      const name = byId<HTMLInputElement>("process-name");
      const action = byId<HTMLSelectElement>("process-action")
        .value as ProcessAction;
      this.apply(
        addProcess(this.working, name.value, action),
        "process-targets",
        () => (name.value = ""),
      );
    });
    byId<HTMLFormElement>("service-add").addEventListener("submit", (event) => {
      event.preventDefault();
      const name = byId<HTMLInputElement>("service-name");
      this.apply(
        addService(this.working, name.value),
        "service-targets",
        () => (name.value = ""),
      );
    });
    byId<HTMLFormElement>("keep-add").addEventListener("submit", (event) => {
      event.preventDefault();
      const name = byId<HTMLInputElement>("keep-name");
      const shown = name.value.trim();
      const before = this.working;
      const result = addKeepAlive(before, name.value);
      this.apply(result, "keep-alive", () => {
        name.value = "";
        // Protection wins over parking, so the row is gone: say so.
        if (result.ok && listed(result.profile) < listed(before))
          toast(`${shown} is protected now, so it was taken off the park list`);
      });
    });
    byId<HTMLInputElement>("opt-power").addEventListener("change", (event) => {
      this.working = {
        ...this.working,
        power: (event.target as HTMLInputElement).checked
          ? "performance"
          : "leave",
      };
      this.updateStatus();
    });
    byId<HTMLInputElement>("opt-purge").addEventListener("change", (event) => {
      this.working = {
        ...this.working,
        purge_memory: (event.target as HTMLInputElement).checked,
      };
      this.updateStatus();
    });
    byId("targets-save").addEventListener("click", () => void this.save());
    byId("targets-reset").addEventListener("click", () => void this.reset());
    this.render();
  }

  describe(caps: Capabilities, os: Os): void {
    const hint = byId("service-hint");
    // Rust refuses these when the list is saved; say so before that.
    const essential = " Essential ones (sound, network, security) are refused.";
    if (os === "linux")
      hint.textContent =
        "systemd units; prefix user units with user: (e.g. user:tracker-miner-fs-3)." +
        essential;
    else if (os === "mac_os")
      hint.textContent =
        "launchd agent labels, e.g. com.microsoft.update.agent." + essential;
    else if (!caps.services)
      hint.textContent = "Stopping services needs administrator rights.";
    else
      hint.textContent =
        "Windows service names, as shown in services.msc." + essential;
  }

  setProfile(profile: Profile): void {
    // Unsaved edits survive a save made elsewhere (Scan adding its finds).
    this.working = sameProfile(this.saved, this.working)
      ? structuredClone(profile)
      : rebase(this.working, this.saved, profile);
    this.saved = profile;
    this.render();
  }

  async refreshRunning(): Promise<void> {
    try {
      const rows = await api.listProcesses();
      this.running = new Map(rows.map((row) => [normalizeName(row.name), row]));
      const list = byId<HTMLDataListElement>("running-processes");
      list.replaceChildren(
        ...rows.slice(0, 200).map((row) => {
          const option = document.createElement("option");
          option.value = row.name;
          option.label = `${formatBytes(row.memory_bytes)} · ${row.instances} running`;
          return option;
        }),
      );
      this.render();
    } catch (error) {
      toast(`Could not list processes: ${errorMessage(error)}`, true);
    }
  }

  private apply(result: EditResult, list: string, onOk: () => void): void {
    if (!result.ok) {
      toast(result.reason, true);
      return;
    }
    this.working = result.profile;
    onOk();
    this.render();
    this.reveal(list);
  }

  /** The row just added goes at the bottom of a long list: bring it into view. */
  private reveal(list: string): void {
    const row = byId(list).lastElementChild;
    if (!row) return;
    row.classList.add("fresh");
    row.scrollIntoView({ block: "nearest" });
    window.setTimeout(() => row.classList.remove("fresh"), FRESH_MS);
  }

  /** After a row goes, focus moves to the one that took its place. */
  private removed(list: string, index: number, fallback: string): void {
    this.render();
    const rows = byId(list).children;
    const next = rows[Math.min(index, rows.length - 1)];
    const control = next?.querySelector<HTMLElement>("button");
    (control ?? byId(fallback)).focus();
  }

  private async save(): Promise<void> {
    // What was sent is what is saved: an edit made during the round trip
    // stays unsaved rather than being marked saved.
    const sent = structuredClone(this.working);
    byId<HTMLButtonElement>("targets-save").disabled = true;
    try {
      await this.host.save(sent);
      this.saved = sent;
      toast("Park list saved");
    } catch (error) {
      toast(errorMessage(error), true);
    } finally {
      this.render();
    }
  }

  private async reset(): Promise<void> {
    const choice = await showDialog({
      title: "Restore the default park list?",
      body: "Your own additions will be removed. Nothing is saved until you press Save changes.",
      buttons: [
        { label: "Restore defaults", value: "yes", primary: true },
        { label: "Cancel", value: "no" },
      ],
    });
    if (choice !== "yes") return;
    try {
      this.working = await this.host.defaults();
      this.render();
    } catch (error) {
      toast(errorMessage(error), true);
    }
  }

  /**
   * The save button, the unsaved note and the two options. A tick or a choice
   * in a row changes only these: rebuilding the row would throw away the
   * keyboard focus that was just used to make it.
   */
  private updateStatus(): void {
    const dirty = !sameProfile(this.saved, this.working);
    byId<HTMLButtonElement>("targets-save").disabled = !dirty;
    byId("targets-status").textContent = dirty ? "Unsaved changes" : "";
    byId<HTMLInputElement>("opt-power").checked =
      this.working.power === "performance";
    byId<HTMLInputElement>("opt-purge").checked = this.working.purge_memory;
    this.host.unsaved(dirty);
  }

  private render(): void {
    this.updateStatus();

    const processes = byId<HTMLTableSectionElement>("process-targets");
    processes.replaceChildren(
      ...this.working.processes.map((target, index) => {
        const row = document.createElement("tr");
        const enabled = this.checkbox(
          target.name,
          target.enabled,
          (checked) => {
            this.working = setProcess(this.working, index, {
              enabled: checked,
            });
            this.updateStatus();
          },
        );
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
        select.addEventListener("change", () => {
          this.working = setProcess(this.working, index, {
            action: select.value as ProcessAction,
          });
          this.updateStatus();
        });
        action.appendChild(select);
        const now = document.createElement("td");
        const live = this.running.get(normalizeName(target.name));
        const state = document.createElement("span");
        state.className = live ? "state running" : "state";
        state.textContent = live
          ? `${live.instances} running · ${formatBytes(live.memory_bytes)}`
          : "not running";
        now.appendChild(state);
        row.append(
          enabled,
          name,
          action,
          now,
          this.remove(`Remove ${target.name}`, () => {
            this.working = removeProcess(this.working, index);
            this.removed("process-targets", index, "process-name");
          }),
        );
        return row;
      }),
    );

    const services = byId<HTMLTableSectionElement>("service-targets");
    services.replaceChildren(
      ...this.working.services.map((target, index) => {
        const row = document.createElement("tr");
        const enabled = this.checkbox(
          target.name,
          target.enabled,
          (checked) => {
            this.working = setService(this.working, index, checked);
            this.updateStatus();
          },
        );
        const name = document.createElement("td");
        name.className = "name";
        name.textContent = target.name;
        row.append(
          enabled,
          name,
          this.remove(`Remove ${target.name}`, () => {
            this.working = removeService(this.working, index);
            this.removed("service-targets", index, "service-name");
          }),
        );
        return row;
      }),
    );

    const keep = byId<HTMLUListElement>("keep-alive");
    keep.replaceChildren(
      ...this.working.keep_alive.map((name, index) => {
        const item = document.createElement("li");
        item.textContent = name;
        const button = document.createElement("button");
        button.type = "button";
        button.textContent = "✕";
        button.setAttribute("aria-label", `Stop protecting ${name}`);
        button.addEventListener("click", () => {
          this.working = removeKeepAlive(this.working, index);
          this.removed("keep-alive", index, "keep-name");
        });
        item.appendChild(button);
        return item;
      }),
    );
  }

  private checkbox(
    name: string,
    checked: boolean,
    onChange: (checked: boolean) => void,
  ): HTMLTableCellElement {
    const cell = document.createElement("td");
    const input = document.createElement("input");
    input.type = "checkbox";
    input.checked = checked;
    input.setAttribute("aria-label", `Enable ${name}`);
    input.addEventListener("change", () => onChange(input.checked));
    cell.appendChild(input);
    return cell;
  }

  private remove(label: string, onClick: () => void): HTMLTableCellElement {
    const cell = document.createElement("td");
    const button = document.createElement("button");
    button.type = "button";
    button.className = "small ghost";
    button.textContent = "✕";
    button.setAttribute("aria-label", label);
    button.addEventListener("click", onClick);
    cell.appendChild(button);
    return cell;
  }
}
