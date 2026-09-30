/**
 * Choosing, adding, renaming and deleting profiles: the choice on Home and the
 * card at the top of the Park list. The engine does the work and checks the
 * names; this only asks, and says why it was refused.
 */
import {
  api,
  errorMessage,
  type EngineState,
  type Settings,
} from "./bridge.ts";
import { showDialog, toast } from "./dialog.ts";
import { byId } from "./dom.ts";
import { checkProfileName } from "./profile-edit.ts";

export interface ProfilesHost {
  /** The Park list holds edits that are not saved. */
  unsaved(): boolean;
  /**
   * The settings a change returned, to take up in place of the page's own.
   * `keepEdits` when the profile in use kept its lists, as a rename does.
   */
  changed(settings: Settings, keepEdits: boolean): Promise<void>;
}

export class ProfilesView {
  private state: EngineState | null = null;
  private working = false;

  constructor(private readonly host: ProfilesHost) {
    for (const id of ["profile-choice", "profile-edit-choice"]) {
      byId<HTMLSelectElement>(id).addEventListener("change", (event) => {
        void this.choose((event.target as HTMLSelectElement).value);
      });
    }
    byId<HTMLFormElement>("profile-add").addEventListener("submit", (event) => {
      event.preventDefault();
      void this.add();
    });
    byId("profile-rename").addEventListener("click", () => void this.rename());
    byId("profile-delete").addEventListener("click", () => void this.remove());
  }

  /** Show the profiles the engine has; none can change while Quiet Mode is on. */
  render(engine: EngineState): void {
    this.state = engine;
    const many = engine.profiles.length > 1;
    const locked = engine.quiet || engine.busy;
    for (const id of ["profile-choice", "profile-edit-choice"]) {
      const select = byId<HTMLSelectElement>(id);
      const names = engine.profiles.join("\n");
      if (select.dataset["names"] !== names) {
        select.dataset["names"] = names;
        select.replaceChildren(
          ...engine.profiles.map((name) => new Option(name, name)),
        );
      }
      select.value = engine.profile;
      select.disabled = locked;
    }
    // Home offers the choice only where there is one.
    byId("profile-bar").hidden = !many;
    byId("profile-note").textContent =
      many && locked ? "Put everything back to choose another." : "";
    byId("profile-lock").hidden = !locked;
    byId<HTMLButtonElement>("profile-delete").disabled = locked || !many;
    byId<HTMLInputElement>("profile-name").disabled = locked;
    byId<HTMLSelectElement>("profile-start").disabled = locked;
    // Add and Rename, which share the name.
    for (const button of byId("profile-add").querySelectorAll("button"))
      button.disabled = locked;
  }

  private async choose(name: string): Promise<void> {
    const state = this.state;
    if (!state || name === state.profile) return;
    if (!(await this.discardEdits(`Switching to ${name}`))) {
      this.render(state);
      return;
    }
    await this.change(() => api.switchProfile(name), false, `Using ${name}`);
  }

  private async add(): Promise<void> {
    const state = this.state;
    if (!state) return;
    const input = byId<HTMLInputElement>("profile-name");
    const name = input.value.trim();
    const reason = checkProfileName(name, state.profiles);
    if (reason) return toast(reason, true);
    const copy = byId<HTMLSelectElement>("profile-start").value === "copy";
    if (!(await this.discardEdits("Adding a profile"))) return;
    const added = await this.change(
      () => api.addProfile(name, copy),
      false,
      `Added ${name}, and using it`,
    );
    if (added) input.value = "";
  }

  private async rename(): Promise<void> {
    const state = this.state;
    if (!state) return;
    const input = byId<HTMLInputElement>("profile-name");
    const name = input.value.trim();
    const reason = checkProfileName(name, state.profiles, state.profile);
    if (reason) return toast(reason, true);
    const renamed = await this.change(
      () => api.renameProfile(state.profile, name),
      true,
      `${state.profile} is now called ${name}`,
    );
    if (renamed) input.value = "";
  }

  private async remove(): Promise<void> {
    const state = this.state;
    if (!state || state.profiles.length < 2) return;
    const choice = await showDialog({
      title: `Delete ${state.profile}?`,
      body: "Its park list and options are deleted, and so is any program's choice to start it. Another profile takes its place. Never touch stays as it is.",
      buttons: [
        { label: "Delete", value: "yes", danger: true },
        { label: "Cancel", value: "no", primary: true },
      ],
      cancel: "no",
    });
    if (choice !== "yes") return;
    await this.change(
      () => api.deleteProfile(state.profile),
      false,
      `Deleted ${state.profile}`,
    );
  }

  /** Unsaved Park list edits belong to the profile being left: say so first. */
  private async discardEdits(doing: string): Promise<boolean> {
    if (!this.host.unsaved()) return true;
    const choice = await showDialog({
      title: "Discard unsaved changes?",
      body: `${doing} drops the Park list changes you have not saved.`,
      buttons: [
        { label: "Discard changes", value: "yes", danger: true },
        { label: "Cancel", value: "no", primary: true },
      ],
      cancel: "no",
    });
    return choice === "yes";
  }

  private async change(
    work: () => Promise<Settings>,
    keepEdits: boolean,
    done: string,
  ): Promise<boolean> {
    if (this.working) return false;
    this.working = true;
    try {
      await this.host.changed(await work(), keepEdits);
      toast(done);
      return true;
    } catch (error) {
      toast(errorMessage(error), true);
      if (this.state) this.render(this.state);
      return false;
    } finally {
      this.working = false;
    }
  }
}
