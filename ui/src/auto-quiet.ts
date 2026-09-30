/** Settings for the programs that start Quiet Mode, and the still-on reminder. */
import type { AutoQuiet, Settings } from "./bridge.ts";
import { toast } from "./dialog.ts";
import { byId } from "./dom.ts";
import {
  addProgram,
  profileNames,
  removeTrigger,
  setTriggerProfile,
} from "./profile-edit.ts";
import { fillRunning } from "./programs.ts";

export interface AutoQuietHost {
  current(): Settings;
  persist(settings: Settings): Promise<void>;
}

export class AutoQuietView {
  constructor(private readonly host: AutoQuietHost) {
    byId<HTMLInputElement>("set-auto-quiet").addEventListener(
      "change",
      (event) => {
        const enabled = (event.target as HTMLInputElement).checked;
        void this.change((auto) => ({ ...auto, enabled }));
      },
    );
    byId<HTMLSelectElement>("set-still-on").addEventListener(
      "change",
      (event) => {
        const hours = Number((event.target as HTMLSelectElement).value);
        void this.host.persist({
          ...this.host.current(),
          still_on_hours: hours,
        });
      },
    );
    const name = byId<HTMLInputElement>("auto-quiet-name");
    name.addEventListener(
      "focus",
      () => void fillRunning(byId<HTMLDataListElement>("auto-quiet-programs")),
    );
    byId<HTMLFormElement>("auto-quiet-add").addEventListener(
      "submit",
      (event) => {
        event.preventDefault();
        const result = addProgram(
          this.host.current().auto_quiet.programs,
          name.value,
        );
        if (!result.ok) {
          toast(result.reason, true);
          return;
        }
        name.value = "";
        void this.change((auto) => ({ ...auto, programs: result.programs }));
      },
    );
  }

  render(settings: Settings): void {
    byId<HTMLInputElement>("set-auto-quiet").checked =
      settings.auto_quiet.enabled;
    const still = byId<HTMLSelectElement>("set-still-on");
    const hours = String(settings.still_on_hours);
    // A value written by hand is still shown as it is.
    if (!Array.from(still.options).some((option) => option.value === hours))
      still.add(new Option(`${hours} hours`, hours));
    still.value = hours;
    const names = profileNames(settings);
    byId("auto-quiet-list").replaceChildren(
      ...settings.auto_quiet.programs.map((program) => {
        const item = document.createElement("li");
        item.textContent = program;
        // Only where there is a choice to make.
        if (names.length > 1)
          item.appendChild(this.chooser(settings, program, names));
        const remove = document.createElement("button");
        remove.type = "button";
        remove.textContent = "✕";
        remove.setAttribute(
          "aria-label",
          `Stop starting Quiet Mode for ${program}`,
        );
        remove.addEventListener("click", () => {
          void this.change((auto) => removeTrigger(auto, program));
        });
        item.appendChild(remove);
        return item;
      }),
    );
  }

  /** Which profile `program` starts: the one in use, or one of the saved. */
  private chooser(
    settings: Settings,
    program: string,
    names: string[],
  ): HTMLSelectElement {
    const select = document.createElement("select");
    select.setAttribute("aria-label", `Profile to start for ${program}`);
    select.add(new Option("Profile in use", ""));
    for (const name of names) select.add(new Option(name, name));
    const chosen = settings.auto_quiet.profiles?.[program] ?? "";
    select.value = names.includes(chosen) ? chosen : "";
    select.addEventListener("change", () => {
      void this.change((auto) =>
        setTriggerProfile(auto, program, select.value),
      );
    });
    return select;
  }

  /** Save the edited list; the chips follow what was kept, or put back. */
  private async change(edit: (auto: AutoQuiet) => AutoQuiet): Promise<void> {
    const settings = this.host.current();
    await this.host.persist({
      ...settings,
      auto_quiet: edit(settings.auto_quiet),
    });
    this.render(this.host.current());
  }
}
