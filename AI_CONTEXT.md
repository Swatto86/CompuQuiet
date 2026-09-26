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

- `src-tauri/src/tray.rs` — tray menu handled entirely in Rust (`dispatch_menu`);
  Quit restores per settings then `app.exit(0)` (no webview dependency)
- `src-tauri/src/commands.rs` — IPC; `simulate_tray_menu` (fake-platform only) for e2e
- `crates/cq-core/src/store.rs` — data dir `CompuQuiet`; migrates legacy `ComputeQuiet`;
  env `COMPUQUIET_DATA_DIR` (legacy `COMPUTEQUIET_DATA_DIR`)
- `ui/index.html` — Home / Find savings / What to park / Settings (About folded in)

## Data Flow

Snapshot → plan → execute + journal → restore newest-first. Scan recommendations
auto-apply low-risk only when `auto_scan` is on.

## Recent Context & Decisions

- 2026-09-26: Renamed product to CompuQuiet (crate `compuquiet`, id `co.swatto.compuquiet`).
- 2026-09-26: Tray right-click menu fixed — all actions run in Rust; Quit no longer
  depends on a hidden webview receiving `confirm-quit`.
- 2026-09-26: UI copy/tabs clarified; primary action "Free up this PC".
- Fresh GitHub repo `Swatto86/CompuQuiet` + site update on swatbox pending after
  local install handoff.
