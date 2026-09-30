/**
 * What Home says when Quiet Mode cannot finish or start, and what the
 * confirmation names before anything is given up. Plain functions over the
 * engine state, so the wording is testable without a page.
 */
import type { EngineState } from "./bridge.ts";

export type RecoveryKind = "stuck" | "journal";

export interface RecoveryNotice {
  kind: RecoveryKind;
  title: string;
  intro: string;
  items: string[];
  action: string;
}

export interface RecoveryConfirmation {
  title: string;
  body: string;
  confirm: string;
}

/** `null` when nothing is stuck. Entries that failed to restore come first. */
export function recoveryNotice(state: EngineState): RecoveryNotice | null {
  if (state.unrestored.length > 0) {
    return {
      kind: "stuck",
      title: "Some changes could not be put back",
      intro:
        "Put everything back tries these again each time. If they can never work (a service that has been disabled since, say), give up on them to end Quiet Mode.",
      items: state.unrestored.map((entry) =>
        entry.error ? `${entry.label}: ${entry.error}` : entry.label,
      ),
      action: "Give up on these…",
    };
  }
  if (state.startup_error !== null) {
    return {
      kind: "journal",
      title: "The record of an earlier Quiet Mode cannot be read",
      intro:
        "CompuQuiet cannot put back what that session parked, and will not start Quiet Mode until this is dealt with. If a newer CompuQuiet wrote it, update instead; otherwise set the record aside.",
      items: [state.startup_error],
      action: "Set the record aside…",
    };
  }
  return null;
}

/** The dialog shown before giving up: every entry named, with what it leaves. */
export function recoveryConfirmation(
  kind: RecoveryKind,
  state: EngineState,
): RecoveryConfirmation {
  if (kind === "stuck") {
    const lines = state.unrestored.map(
      (entry) => `• ${entry.label}: ${entry.consequence}`,
    );
    return {
      title: "Give up on putting these back?",
      body: [
        "Quiet Mode ends, and this is what stays as it is:",
        ...lines,
        "The record is kept as journal.json.bad in the data folder, so it is not lost.",
      ].join("\n"),
      confirm: "Give up on these",
    };
  }
  return {
    title: "Set the record aside?",
    body: "Anything that Quiet Mode session left stopped, frozen or closed stays that way until you start it again yourself. The record is kept as journal.json.bad in the data folder, so it is not lost.",
    confirm: "Set it aside",
  };
}
