/** The profile editor: which programs and services Quiet Mode touches. */
import {
  errorMessage,
  type Capabilities,
  type Os,
  type Profile,
} from "./bridge.ts";
import { showDialog, toast } from "./dialog.ts";
import {
  awakeHint,
  matchesFilter,
  serviceHint,
  slowHint,
} from "./park-list.ts";
import { orNote, processRow, serviceRow } from "./park-rows.ts";
import { Pickers } from "./pickers.ts";
import {
  addKeepAlive,
  addProcess,
  addService,
  normalizeName,
  removeKeepAlive,
  removeProcess,
  removeService,
  rebase,
  restoreDefaults,
  sameProfile,
  setProcess,
  setService,
  type EditResult,
  type Handling,
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
  private readonly pickers = new Pickers();
  /** What the filter box holds: only matching rows are drawn. */
  private filter = "";
  /** More than one profile, so Never touch is something to say about a removal. */
  private shared = false;

  constructor(
    initial: Profile,
    private readonly host: TargetsHost,
  ) {
    this.saved = initial;
    this.working = structuredClone(initial);
    byId<HTMLInputElement>("park-filter").addEventListener("input", (event) => {
      this.filter = (event.target as HTMLInputElement).value;
      this.render();
    });
    byId<HTMLFormElement>("process-add").addEventListener("submit", (event) => {
      event.preventDefault();
      const name = byId<HTMLInputElement>("process-name");
      const handling = byId<HTMLSelectElement>("process-action")
        .value as Handling;
      this.apply(
        addProcess(this.working, name.value, handling),
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
    byId<HTMLInputElement>("opt-unload").addEventListener("change", (event) => {
      this.working = {
        ...this.working,
        unload_ai_models: (event.target as HTMLInputElement).checked,
      };
      this.updateStatus();
    });
    byId<HTMLInputElement>("opt-awake").addEventListener("change", (event) => {
      this.working = {
        ...this.working,
        keep_awake: (event.target as HTMLInputElement).checked,
      };
      this.updateStatus();
    });
    byId("targets-save").addEventListener("click", () => void this.save());
    byId("targets-reset").addEventListener("click", () => void this.reset());
    this.render();
  }

  describe(caps: Capabilities, os: Os): void {
    byId("service-hint").textContent = serviceHint(caps, os);
    // One already ticked can still be unticked; one that cannot work cannot be ticked.
    const awake = byId<HTMLInputElement>("opt-awake");
    awake.disabled = !caps.keep_awake && !awake.checked;
    byId("awake-hint").textContent = awakeHint(caps);
    const slow = byId("slow-hint");
    slow.textContent = slowHint(caps);
    slow.hidden = slow.textContent === "";
  }

  /** How many profiles there are. */
  setProfileCount(count: number): void {
    this.shared = count > 1;
  }

  /** Another profile's lists: what was being edited belongs to the one left. */
  replace(profile: Profile): void {
    this.saved = profile;
    this.working = structuredClone(profile);
    this.render();
  }

  setProfile(profile: Profile): void {
    // Unsaved edits survive a save made elsewhere (Scan adding its finds).
    this.working = sameProfile(this.saved, this.working)
      ? structuredClone(profile)
      : rebase(this.working, this.saved, profile);
    this.saved = profile;
    this.render();
  }

  /** Ask the machine what runs and what services it has, for the pickers and the rows. */
  async refreshRunning(): Promise<void> {
    await this.pickers.refresh();
    this.render();
  }

  private apply(result: EditResult, list: string, onOk: () => void): void {
    if (!result.ok) {
      toast(result.reason, true);
      return;
    }
    this.working = result.profile;
    onOk();
    // What was just added must not be hidden by what was typed before.
    this.filter = "";
    byId<HTMLInputElement>("park-filter").value = "";
    this.render();
    this.reveal(list);
  }

  /**
   * A removed row goes under Never touch, which every profile shares, so it
   * leaves the others' lists too. Said, because that is not what removing a
   * row from one list of several looks like.
   */
  private neverTouchNote(name: string): void {
    if (this.shared)
      toast(
        `${name} is under Never touch now, in every profile. To stop parking it in this profile only, untick it instead.`,
      );
  }

  /** The row just added goes at the bottom of a long list: bring it into view. */
  private reveal(list: string): void {
    const row = byId(list).lastElementChild;
    if (!row) return;
    row.classList.add("fresh");
    row.scrollIntoView({ block: "nearest" });
    window.setTimeout(() => row.classList.remove("fresh"), FRESH_MS);
  }

  /** After a row goes, focus moves to the one that took its place; `index` counts the rows drawn. */
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
      body: "Your own additions will be removed. Never touch is kept, and what is on it stays off the list. Nothing is saved until you press Save changes.",
      buttons: [
        { label: "Restore defaults", value: "yes", primary: true },
        { label: "Cancel", value: "no" },
      ],
    });
    if (choice !== "yes") return;
    try {
      this.working = restoreDefaults(this.working, await this.host.defaults());
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
    byId<HTMLInputElement>("opt-awake").checked = this.working.keep_awake;
    byId<HTMLInputElement>("opt-unload").checked =
      this.working.unload_ai_models;
    this.host.unsaved(dirty);
  }

  private render(): void {
    this.updateStatus();

    const programs = this.filtered(this.working.processes);
    byId<HTMLTableSectionElement>("process-targets").replaceChildren(
      ...orNote(
        programs.map(([target, index], place) =>
          processRow(
            target,
            this.pickers.running.get(normalizeName(target.name)),
            {
              toggle: (enabled) => {
                this.working = setProcess(this.working, index, { enabled });
                this.updateStatus();
              },
              act: (handling) => {
                this.working = setProcess(this.working, index, { handling });
                this.updateStatus();
              },
              remove: () => {
                this.working = removeProcess(this.working, index);
                this.removed("process-targets", place, "process-name");
                this.neverTouchNote(target.name);
              },
            },
          ),
        ),
        this.working.processes.length,
        5,
      ),
    );

    const stopped = this.filtered(this.working.services, (name) =>
      this.pickers.displayName(name),
    );
    byId<HTMLTableSectionElement>("service-targets").replaceChildren(
      ...orNote(
        stopped.map(([target, index], place) =>
          serviceRow(target, this.pickers.displayName(target.name), {
            toggle: (enabled) => {
              this.working = setService(this.working, index, enabled);
              this.updateStatus();
            },
            remove: () => {
              this.working = removeService(this.working, index);
              this.removed("service-targets", place, "service-name");
              this.neverTouchNote(target.name);
            },
          }),
        ),
        this.working.services.length,
        3,
      ),
    );
    byId("park-filter-status").textContent =
      this.filter.trim() === ""
        ? ""
        : `Showing ${programs.length} of ${this.working.processes.length} programs and ${stopped.length} of ${this.working.services.length} services`;

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

  /** What the filter leaves of a list: each target with its place in the whole list. */
  private filtered<T extends { name: string }>(
    list: T[],
    display: (name: string) => string = () => "",
  ): [T, number][] {
    const kept: [T, number][] = [];
    list.forEach((target, index) => {
      if (matchesFilter(this.filter, target.name, display(target.name)))
        kept.push([target, index]);
    });
    return kept;
  }
}
