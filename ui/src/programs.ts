/** The names of what is running, for a box that takes a program's name. */
import { api, errorMessage } from "./bridge.ts";
import { toast } from "./dialog.ts";
import { formatBytes } from "./format.ts";

const MOST_SHOWN = 200;

export async function fillRunning(list: HTMLDataListElement): Promise<void> {
  try {
    const rows = await api.listProcesses();
    list.replaceChildren(
      ...rows.slice(0, MOST_SHOWN).map((row) => {
        const option = document.createElement("option");
        option.value = row.name;
        option.label = `${formatBytes(row.memory_bytes)} · ${row.instances} running`;
        return option;
      }),
    );
  } catch (error) {
    toast(`Could not list processes: ${errorMessage(error)}`, true);
  }
}
