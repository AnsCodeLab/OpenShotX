//! System tray icon (StatusNotifierItem) providing one-click capture actions.
//!
//! Built on `ksni` (pure-Rust SNI over D-Bus). Capture/record items launch the
//! OpenShotX CLI as child processes (reusing all existing logic). "Settings…"
//! and "Quit" are routed to the GTK side via an [`async_channel`] so a single
//! process owns both the tray and the settings window (see [`crate::gui`]).
//!
//! Requires an SNI host on the session bus (KDE natively; GNOME via the
//! AppIndicator extension). With no host the daemon runs but shows no icon.

use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, Tray, TrayMethods};
use std::process::Command;
use std::sync::OnceLock;

/// PNG used for the tray pixmap, rendered from `data/openshotx.svg`.
const ICON_PNG: &[u8] = include_bytes!("../../data/openshotx-tray.png");

/// Decode the embedded PNG into an SNI [`Icon`] (ARGB32, network byte order).
/// Cached after first use. Returns empty on decode failure (falls back to name).
fn icon_pixmaps() -> &'static Vec<Icon> {
    static ICON: OnceLock<Vec<Icon>> = OnceLock::new();
    ICON.get_or_init(|| {
        match image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png) {
            Ok(img) => {
                let (width, height) = (img.width() as i32, img.height() as i32);
                let mut data = img.into_rgba8().into_vec();
                // RGBA -> ARGB by rotating each pixel's bytes right by one.
                for px in data.chunks_exact_mut(4) {
                    px.rotate_right(1);
                }
                vec![Icon { width, height, data }]
            }
            Err(e) => {
                eprintln!("tray: failed to decode embedded icon: {}", e);
                Vec::new()
            }
        }
    })
}

/// Messages the tray (background thread) sends to the GTK main thread.
#[derive(Debug, Clone, Copy)]
pub enum TrayMsg {
    /// The tray icon registered with a host and is now visible.
    Registered,
    /// Show / focus the settings window.
    ShowSettings,
    /// Quit the whole application.
    Quit,
}

/// The tray menu state. Holds a channel to the GTK main thread and, while a
/// screen recording is running, that child process so it can be stopped.
struct OpenShotXTray {
    tx: async_channel::Sender<TrayMsg>,
    recording: Option<std::process::Child>,
}

impl OpenShotXTray {
    /// Start a recording of the given type as a child process, tracking it so
    /// it can be stopped from the menu. On Wayland this triggers the portal
    /// share dialog. `record_type` should be `"screen"`, `"area"`, or
    /// `"window"`.
    fn start_recording(&mut self, record_type: &str) {
        // Reap any previous child that already exited on its own.
        if let Some(child) = self.recording.as_mut() {
            if matches!(child.try_wait(), Ok(Some(_))) {
                self.recording = None;
            } else {
                return; // already recording
            }
        }
        let args = vec!["record", record_type, "--notify"];
        match Command::new("openshotx").args(&args).spawn() {
            Ok(child) => self.recording = Some(child),
            Err(e) => eprintln!("tray: failed to start recording: {}", e),
        }
    }

    /// Stop the running recording by sending SIGINT (the record command only
    /// finalizes the file cleanly on SIGINT), then reap it off-thread so the
    /// menu callback never blocks while GStreamer flushes end-of-stream.
    fn stop_recording(&mut self) {
        if let Some(child) = self.recording.take() {
            let pid = child.id() as libc::pid_t;
            // SAFETY: sending a signal to a pid we own; errors are ignored.
            unsafe { libc::kill(pid, libc::SIGINT) };
            std::thread::spawn(move || {
                let mut child = child;
                let _ = child.wait();
            });
        }
    }

}

/// Spawn the OpenShotX binary with `args`, detached. Logs failures; never panics.
fn spawn_action(args: &[&str]) {
    if let Err(e) = Command::new("openshotx").args(args).spawn() {
        eprintln!("tray: failed to launch openshotx {:?}: {}", args, e);
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

/// Build a menu item that sends `msg` to the GTK main thread.
fn msg_item(label: &str, msg: TrayMsg) -> MenuItem<OpenShotXTray> {
    StandardItem {
        label: label.into(),
        activate: Box::new(move |this: &mut OpenShotXTray| {
            let _ = this.tx.try_send(msg);
        }),
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

    fn icon_pixmap(&self) -> Vec<Icon> {
        icon_pixmaps().clone()
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        // While recording: show only the stop button.
        // Otherwise: show three recording options.
        let recording_items: Vec<MenuItem<Self>> = if self.recording.is_some() {
            vec![
                StandardItem {
                    label: "⏹ Stop Recording".into(),
                    activate: Box::new(|this: &mut OpenShotXTray| this.stop_recording()),
                    ..Default::default()
                }
                .into(),
            ]
        } else {
            vec![
                StandardItem {
                    label: "Record Screen".into(),
                    activate: Box::new(|this: &mut OpenShotXTray| this.start_recording("screen")),
                    ..Default::default()
                }
                .into(),
                action_item("Record Region", &["record", "area", "--notify"]),
                action_item("Record Window", &["record", "window", "--notify"]),
            ]
        };

        let mut items: Vec<MenuItem<Self>> = vec![
            action_item("Capture Area", &["capture", "area", "--notify"]),
            action_item("Capture Screen", &["capture", "screen", "--notify"]),
            action_item("Capture Window", &["capture", "window", "--notify"]),
            action_item("Capture & OCR Text", &["capture", "area", "--ocr", "--notify"]),
            MenuItem::Separator,
        ];
        items.extend(recording_items);
        items.push(MenuItem::Separator);
        items.push(msg_item("Settings…", TrayMsg::ShowSettings));
        items.push(msg_item("Quit", TrayMsg::Quit));
        items
    }
}

/// How many times to retry registering with the StatusNotifierWatcher.
/// At login the host (e.g. GNOME's AppIndicator extension) may not be up yet,
/// so autostart needs to tolerate a brief race.
const MAX_REGISTER_ATTEMPTS: u32 = 10;
const RETRY_DELAY_SECS: u64 = 2;

/// Spawn the ksni tray on a dedicated thread with its own current-thread Tokio
/// runtime. The thread lives until the process exits. Menu activations send
/// [`TrayMsg`]s through `tx`.
pub fn spawn_tray_thread(tx: async_channel::Sender<TrayMsg>) {
    let builder = std::thread::Builder::new().name("openshotx-tray".into());
    let spawn_result = builder.spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("tray: failed to start runtime: {}", e);
                return;
            }
        };
        rt.block_on(async move {
            let mut handle = None;
            for attempt in 1..=MAX_REGISTER_ATTEMPTS {
                let tray = OpenShotXTray { tx: tx.clone(), recording: None };
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
                            "tray: failed to register after {} attempts (is a StatusNotifier \
                             host running? On GNOME, enable the AppIndicator extension): {}",
                            MAX_REGISTER_ATTEMPTS, e
                        );
                        return;
                    }
                }
            }
            if let Some(_handle) = handle {
                eprintln!("tray: registered StatusNotifierItem with the watcher");
                // Tell the GTK side the icon is live, so "minimize to tray" is safe.
                let _ = tx.try_send(TrayMsg::Registered);
                // Keep _handle alive until the runtime shuts down (process exit).
                // Calling handle.shutdown() would actively stop the tray — don't do that.
                std::future::pending::<()>().await;
            }
        });
    });
    if let Err(e) = spawn_result {
        eprintln!("tray: failed to spawn tray thread: {}", e);
    }
}
