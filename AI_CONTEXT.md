# CompuQuiet — agent context

## System Overview

CompuQuiet (formerly ComputeQuiet) parks background processes, services and
power plans so games and local AI get the machine, then restores everything
from an undo journal. Tray-resident Tauri 2 desktop app; unelevated by default.

## Tech Stack & Architecture

- Rust workspace: `crates/cq-core` (domain), `crates/cq-platform` (OS adapters + fake)
- Shell: `src-tauri` (commands, engine, tray, autostart)
- UI: vanilla TypeScript + Vite in `ui/`
- E2E: WebdriverIO + `tauri-driver` against fake-platform debug binary
- Gates: `scripts/fastcheck.ps1` / `.sh`, `scripts/verify.ps1` / `.sh`

## Component Map

- `src-tauri/src/update.rs` — `tauri-plugin-updater` checks GitHub `latest.json` when idle; `scripts/latest-json.mjs` builds that manifest from the signed release bundles
- `src-tauri/src/tray.rs` — tray menu handled entirely in Rust (`dispatch_menu`);
  right-click opens the menu from the event loop (`show_menu`) so Windows
  accepts the click; Quit restores per settings then `app.exit(0)` (no webview)
- `src-tauri/src/commands.rs` — IPC; `simulate_tray_menu` (fake-platform only) for e2e
- `crates/cq-core/src/store.rs` — data dir `CompuQuiet`; migrates legacy `ComputeQuiet`;
  env `COMPUQUIET_DATA_DIR` (legacy `COMPUTEQUIET_DATA_DIR`)
- `ui/index.html` — Home / Scan / Park list / Settings. Home states what the
  button will do (`homePlan` in `ui/src/format.ts`) before the user leaves the page.

## Data Flow

Snapshot → plan → execute + journal → restore newest-first. Scan recommendations
auto-apply low-risk only when `auto_scan` is on.

## Recent Context & Decisions

- 2026-09-27: Auto-update. Idle releases install from `releases/latest/download/latest.json`. Signing key is the `TAURI_SIGNING_PRIVATE_KEY` secret; public key is in `tauri.conf.json`.
- 2026-09-26: Release 1.1.3. Tag `v1.1.3` publishes installers after `verify` is green on that commit.
- 2026-09-26: Tray menu opens from the event loop (`show_menu`) so Windows
  accepts item clicks; Linux re-applies the menu after an icon change.
  Home screen states what one press will do; Scan and Settings copy is plainer.
- 2026-09-26: Renamed product to CompuQuiet (crate `compuquiet`, id `co.swatto.compuquiet`).
- 2026-09-26: Tray right-click menu fixed — all actions run in Rust; Quit no longer
  depends on a hidden webview receiving `confirm-quit`.
- 2026-09-26: UI copy/tabs clarified; primary action "Free up this PC".
- Fresh GitHub repo `Swatto86/CompuQuiet` + site update on swatbox pending after
  local install handoff.
