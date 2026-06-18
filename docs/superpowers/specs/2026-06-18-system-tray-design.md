# System Tray for OpenShotX — Design

**Date:** 2026-06-18
**Status:** Approved (pending spec review)

## Goal

Add a system-tray icon that gives one-click access to OpenShotX's capture/record
actions without a terminal or global hotkey. The tray runs as a resident daemon,
autostarts on login, and is enabled/disabled from the Settings GUI.

## Target environment

Verified on the development machine: GNOME Shell 50.2 (Wayland), with the
AppIndicator extension (`appindicatorsupport@rgcjonas.gmail.com`) installed and
`org.kde.StatusNotifierWatcher` live on the session bus. The tray uses the
StatusNotifierItem (SNI) protocol, so it also works on KDE and any desktop with
an SNI host. On a desktop with no SNI host the daemon still runs but shows no
icon (documented limitation, not an error).

## Approach

Use the **`ksni`** crate — a pure-Rust StatusNotifierItem implementation over
D-Bus. Chosen over `libayatana-appindicator` (GTK3 C library, conflicts with the
GTK4 settings GUI and adds a system dependency) and a hand-rolled zbus SNI (large
amount of code ksni already provides). `ksni` fits the existing zbus/tokio stack
and reuses the `openshotx` icon already installed in the hicolor icon theme.

**Action model:** every menu item launches the existing CLI as a child process
via `std::process::Command::new(current_exe).args(...)`. This reuses all capture/
record/OCR logic with zero duplication and isolates the portal + GTK overlay in a
separate process, so the tray's D-Bus event loop never blocks or conflicts.

## Components

### 1. `openshotx tray` subcommand (`src/tray/mod.rs`)
- `run_tray()` builds the SNI item (icon name `openshotx`, tooltip "OpenShotX")
  and runs the ksni service, parking the main thread until **Quit**.
- Menu:
  - Capture Area  → `capture area --notify`
  - Capture Screen → `capture screen --notify`
  - Capture Window → `capture window --notify`
  - Record Screen → `record screen --notify`
  - Capture & OCR Text → `capture area --ocr --notify`
  - — separator —
  - Settings… → `config`
  - Quit → stops the service and exits
- Each item spawns `current_exe` with the arguments above (detached); failures to
  spawn are logged to stderr, never panic.

### 2. `--notify` flag (`src/main.rs`, capture + record paths)
- On a successful capture/record, spawn `notify-send "OpenShotX" "<message>"`:
  - screenshot → `Saved to <path>`
  - `--ocr` → `Text copied to clipboard`
  - recording → `Recording saved to <path>`
- Reusable: also gives the F6/Shift+F6 hotkeys feedback, which they lack today.
- Missing `notify-send` is a silent no-op (graceful fallback).

### 3. Autostart module (`src/autostart.rs`)
- Manages `~/.config/autostart/openshotx-tray.desktop`. `Exec` is the absolute
  path to the running binary (`std::env::current_exe()`) plus `tray`, so it works
  regardless of the login-session `PATH`; `X-GNOME-Autostart-enabled=true`.
- API: `enable() -> io::Result<()>`, `disable() -> io::Result<()>`,
  `is_enabled() -> bool`, plus an internal `autostart_path()` overridable for
  tests. The **file's existence is the source of truth**; the config bool mirrors
  it.

### 4. Config (`src/config.rs`)
- New `[tray]` section: `TrayConfig { autostart: bool }`, default `false`, wired
  into `Config` with serde defaults like the other sections.

### 5. Settings GUI (`src/gui/general_tab.rs`)
- A switch row **"Start tray icon on login"**.
- Initial state from `autostart::is_enabled()`.
- Toggling it calls `autostart::enable()/disable()` immediately and updates
  `config.tray.autostart` (persisted on Save). Immediate file write means the
  setting takes effect even if the user closes without Save.

### 6. Wiring
- `src/lib.rs`: expose `tray` and `autostart` modules.
- `src/main.rs`: dispatch `"tray" => tray::run_tray()`; document `tray` and
  `--notify` in `print_usage()`.
- `Cargo.toml`: add `ksni`.
- `data/`: keep a reference `openshotx-tray.desktop` template; README documents
  the tray and the autostart toggle.

## Data flow

```
login ─▶ autostart .desktop ─▶ `openshotx tray` (ksni daemon)
                                     │  menu click
                                     ▼
                       Command::new(current_exe) "capture area --notify"
                                     │ (child process)
                                     ▼
            existing capture pipeline ─▶ save + clipboard ─▶ notify-send
```

Settings GUI toggle ─▶ `autostart::enable()/disable()` ─▶ writes/removes
`~/.config/autostart/openshotx-tray.desktop`.

## Error handling

- Tray child-process spawn failures: log to stderr, daemon keeps running.
- No SNI host present: daemon runs without a visible icon (documented).
- Autostart file write failure: surfaced to the GUI as an error log; switch
  reverts to actual `is_enabled()` state.
- `notify-send` absent: no-op.

## Testing

- **Unit:** `autostart::{enable, disable, is_enabled}` against a temp dir via the
  overridable path; `TrayConfig` serde round-trip (load with/without `[tray]`).
- **Manual:** run `openshotx tray`, confirm the icon appears; exercise each menu
  item (Capture Area/Screen/Window, Record, Capture & OCR, Settings, Quit);
  toggle the GUI switch and confirm the autostart file appears/disappears and
  survives logout/login.

## Out of scope (future)

Recent-screenshots submenu, inline cursor/format toggles in the menu, upload
actions. These map to the earlier "rich menu" (option C) and are deferred.
