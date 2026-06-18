//! System tray icon (StatusNotifierItem) providing one-click capture actions.
//!
//! Built on `ksni` (pure-Rust SNI over D-Bus). Each menu item launches the
//! OpenShotX CLI as a child process, reusing all existing capture/record/OCR
//! logic and isolating the portal + GTK overlay from the tray's event loop.
//!
//! Requires an SNI host on the session bus (KDE natively; GNOME via the
//! AppIndicator extension). With no host the daemon runs but shows no icon.

use ksni::menu::StandardItem;
use ksni::{MenuItem, Tray, TrayMethods};
use std::process::Command;
use tokio::sync::mpsc;

/// The tray menu state. Holds a shutdown signal fired by the Quit item.
struct OpenShotXTray {
    shutdown: mpsc::Sender<()>,
}

/// Spawn the OpenShotX binary with `args`, detached. Logs failures; never panics.
fn spawn_action(args: &[&str]) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("tray: cannot resolve current executable: {}", e);
            return;
        }
    };
    if let Err(e) = Command::new(&exe).args(args).spawn() {
        eprintln!("tray: failed to launch {:?} {:?}: {}", exe, args, e);
    }
}

/// Build a standard menu item whose activation spawns `args`.
fn action_item(label: &str, args: &'static [&'static str]) -> MenuItem<OpenShotXTray> {
    StandardItem {
        label: label.into(),
        activate: Box::new(move |_: &mut OpenShotXTray| spawn_action(args)),
        ..Default::default()
    }
    .into()
}

impl Tray for OpenShotXTray {
    fn id(&self) -> String {
        "openshotx".into()
    }

    fn title(&self) -> String {
        "OpenShotX".into()
    }

    fn icon_name(&self) -> String {
        "openshotx".into()
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            action_item("Capture Area", &["capture", "area", "--notify"]),
            action_item("Capture Screen", &["capture", "screen", "--notify"]),
            action_item("Capture Window", &["capture", "window", "--notify"]),
            action_item("Record Screen", &["record", "screen", "--notify"]),
            action_item("Capture & OCR Text", &["capture", "area", "--ocr", "--notify"]),
            MenuItem::Separator,
            action_item("Settings…", &["config"]),
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|this: &mut OpenShotXTray| {
                    let _ = this.shutdown.try_send(());
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// How many times to retry registering with the StatusNotifierWatcher.
/// At login the host (e.g. GNOME's AppIndicator extension) may not be up yet,
/// so autostart needs to tolerate a brief race.
const MAX_REGISTER_ATTEMPTS: u32 = 10;
const RETRY_DELAY_SECS: u64 = 2;

/// Run the tray daemon until the user selects Quit (or the SNI service dies).
pub async fn run_tray() {
    let (tx, mut rx) = mpsc::channel::<()>(1);

    let mut handle = None;
    for attempt in 1..=MAX_REGISTER_ATTEMPTS {
        let tray = OpenShotXTray { shutdown: tx.clone() };
        match tray.spawn().await {
            Ok(h) => {
                handle = Some(h);
                break;
            }
            Err(e) if attempt < MAX_REGISTER_ATTEMPTS => {
                eprintln!(
                    "tray: no StatusNotifier host yet ({}); retry {}/{} in {}s",
                    e, attempt, MAX_REGISTER_ATTEMPTS, RETRY_DELAY_SECS
                );
                tokio::time::sleep(std::time::Duration::from_secs(RETRY_DELAY_SECS)).await;
            }
            Err(e) => {
                eprintln!(
                    "Failed to start tray after {} attempts (is a StatusNotifier host \
                     running? On GNOME, enable the AppIndicator extension): {}",
                    MAX_REGISTER_ATTEMPTS, e
                );
                std::process::exit(1);
            }
        }
    }
    let handle = handle.expect("tray handle set on success");

    println!("OpenShotX tray running. Select Quit from the menu to exit.");

    // Park until Quit is chosen or the tray service shuts down on its own.
    tokio::select! {
        _ = rx.recv() => {}
        _ = handle.shutdown() => {}
    }
}
