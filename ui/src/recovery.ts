/**
 * The Home panel for a stuck Quiet Mode: a restore that keeps failing on the
 * same entry, or a record that cannot be read. Giving up is always confirmed,
 * naming what it leaves behind; the engine keeps the record as a `.bad` file.
 */
import { api, errorMessage, type EngineState } from "./bridge.ts";
import { showDialog, toast } from "./dialog.ts";
import {
  recoveryConfirmation,
  recoveryNotice,
  type RecoveryKind,
} from "./recovery-text.ts";

function byId<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing #${id}`);
  return element as T;
}

export class Recovery {
  private readonly panel = byId("recovery");
  private readonly title = byId("recovery-title");
  private readonly intro = byId("recovery-intro");
  private readonly list = byId<HTMLUListElement>("recovery-list");
  private readonly action = byId<HTMLButtonElement>("recovery-action");
  private readonly onChange: (state: EngineState) => void;
  private state: EngineState | null = null;
  private kind: RecoveryKind | null = null;

  /** `onChange` receives the engine state after something was given up. */
  constructor(onChange: (state: EngineState) => void) {
    this.onChange = onChange;
    this.action.addEventListener("click", () => void this.giveUp());
  }

  render(state: EngineState): void {
    this.state = state;
    const notice = recoveryNotice(state);
    this.kind = notice?.kind ?? null;
    this.panel.hidden = notice === null;
    if (notice === null) return;
    this.title.textContent = notice.title;
    this.intro.textContent = notice.intro;
    this.action.textContent = notice.action;
    this.list.replaceChildren(
      ...notice.items.map((text) => {
        const item = document.createElement("li");
        item.textContent = text;
        return item;
      }),
    );
  }

  private async giveUp(): Promise<void> {
    const { state, kind } = this;
    if (state === null || kind === null) return;
    const confirmation = recoveryConfirmation(kind, state);
    const choice = await showDialog({
      title: confirmation.title,
      body: confirmation.body,
      buttons: [
        { label: confirmation.confirm, value: "confirm", danger: true },
        { label: "Cancel", value: "cancel", primary: true },
      ],
      cancel: "cancel",
    });
    if (choice !== "confirm") return;
    try {
      if (kind === "stuck") {
        this.onChange(await api.giveUpRestoring());
        toast("Quiet Mode ended. The record is kept as journal.json.bad.");
      } else {
        await api.setAsideJournal();
        this.onChange(await api.getState());
        toast("The record is set aside as journal.json.bad.");
      }
    } catch (error) {
      toast(errorMessage(error), true);
      this.onChange(await api.getState());
    }
  }
}
