import { strict as assert } from "node:assert";
import { test } from "node:test";

import type { Capabilities, ServiceRow } from "./bridge.ts";
import {
  awakeHint,
  matchesFilter,
  offered,
  pickerLabel,
  serviceHint,
  stateText,
} from "./park-list.ts";

const caps: Capabilities = {
  services: true,
  power: true,
  memory_purge: true,
  keep_awake: true,
  elevated: true,
  can_elevate: false,
};

function service(
  name: string,
  state: ServiceRow["state"],
  essential = false,
): ServiceRow {
  return { name, display_name: `${name} display`, state, essential };
}

test("the filter keeps a row that holds every word, in any case and order", () => {
  assert.ok(matchesFilter("", "Dropbox.exe"), "nothing typed keeps all");
  assert.ok(matchesFilter("   ", "Dropbox.exe"));
  assert.ok(matchesFilter("drop", "Dropbox.exe"));
  assert.ok(matchesFilter("EXE box", "Dropbox.exe"));
  assert.ok(!matchesFilter("dropbox slack", "Dropbox.exe"));
  assert.ok(!matchesFilter("onedrive", "Dropbox.exe"));
});

test("a service is found by the name the machine gives it as well as its own", () => {
  assert.ok(matchesFilter("print", "Spooler", "Print Spooler"));
  assert.ok(matchesFilter("print spool", "Spooler", "Print Spooler"));
  assert.ok(!matchesFilter("print", "Spooler", undefined));
  assert.ok(!matchesFilter("print", "Spooler", ""));
});

test("a picker entry says what the machine calls the service and whether it runs", () => {
  assert.equal(
    pickerLabel(service("Spooler", "running")),
    "Spooler display · running",
  );
  assert.equal(stateText("stopped"), "stopped");
  assert.equal(stateText("transitioning"), "starting or stopping");
  assert.equal(stateText("not_installed"), "not installed");
});

test("the picker leaves out the services Quiet Mode never stops", () => {
  const rows = [
    service("Spooler", "running"),
    service("AudioSrv", "running", true),
    service("Fax", "stopped"),
  ];
  assert.deepEqual(
    offered(rows).map((row) => row.name),
    ["Spooler", "Fax"],
  );
  assert.equal(rows.length, 3, "the list itself is not changed");
});

test("the service hint names the kind of name this system uses", () => {
  assert.match(serviceHint(caps, "windows"), /services\.msc/);
  assert.match(serviceHint(caps, "linux"), /user:/);
  assert.match(serviceHint(caps, "mac_os"), /launchd/);
  assert.match(
    serviceHint({ ...caps, services: false }, "windows"),
    /administrator/,
  );
  // Where names are checked by another system, administrator rights do not matter.
  assert.match(serviceHint({ ...caps, services: false }, "linux"), /systemd/);
});

test("the awake hint says whether this system can hold off sleep", () => {
  assert.match(awakeHint(caps), /Stops sleep/);
  assert.match(awakeHint({ ...caps, keep_awake: false }), /Not available/);
});
