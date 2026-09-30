/**
 * What the Park list's two pickers know about the machine: the programs
 * running now and the services it has. They fill the suggestions under the
 * name fields and tell a row what its target is.
 */
import {
  api,
  errorMessage,
  type ProcessRow,
  type ServiceRow,
} from "./bridge.ts";
import { toast } from "./dialog.ts";
import { byId } from "./dom.ts";
import { formatBytes } from "./format.ts";
import { offered, pickerLabel } from "./park-list.ts";
import { normalizeName } from "./profile-edit.ts";

/** The most suggestions a field shows: a list of thousands helps nobody. */
const MOST_SUGGESTED = 200;

export class Pickers {
  /** Running programs, by name as profiles compare them. */
  readonly running = new Map<string, ProcessRow>();
  private services = new Map<string, ServiceRow>();

  /** Ask the machine again. A list that cannot be read is reported and left as it was. */
  async refresh(): Promise<void> {
    await Promise.all([this.refreshPrograms(), this.refreshServices()]);
  }

  /** What the machine calls a service, when it has said. */
  displayName(service: string): string {
    return this.services.get(service.trim().toLowerCase())?.display_name ?? "";
  }

  private async refreshPrograms(): Promise<void> {
    try {
      const rows = await api.listProcesses();
      this.running.clear();
      for (const row of rows) this.running.set(normalizeName(row.name), row);
      suggest(
        "running-processes",
        rows
          .slice(0, MOST_SUGGESTED)
          .map((row): [string, string] => [
            row.name,
            `${formatBytes(row.memory_bytes)} · ${row.instances} running`,
          ]),
      );
    } catch (error) {
      toast(`Could not list processes: ${errorMessage(error)}`, true);
    }
  }

  private async refreshServices(): Promise<void> {
    try {
      const rows = await api.listServices();
      this.services = new Map(rows.map((row) => [row.name.toLowerCase(), row]));
      suggest(
        "running-services",
        offered(rows).map((row): [string, string] => [
          row.name,
          pickerLabel(row),
        ]),
      );
    } catch (error) {
      toast(`Could not list services: ${errorMessage(error)}`, true);
    }
  }
}

/** Replace the suggestions of a `<datalist>` with (value, label) pairs. */
function suggest(id: string, entries: [string, string][]): void {
  byId<HTMLDataListElement>(id).replaceChildren(
    ...entries.map(([value, label]) => {
      const option = document.createElement("option");
      option.value = value;
      option.label = label;
      return option;
    }),
  );
}
