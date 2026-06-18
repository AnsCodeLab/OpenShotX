mod general_tab;
mod capture_tab;
mod recording_tab;
mod hotkeys_tab;

use crate::config::Config;
use crate::tray::{self, TrayMsg};
use gtk4::{self as gtk, prelude::*};
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Open the settings window as a standalone, transient dialog (`openshotx config`).
/// Closing the window quits the application.
pub fn run_settings(initial_config: Config) {
    let app = adw::Application::builder()
        .application_id("io.github.anscodelab.openshotx")
        .build();

    app.connect_activate(move |app| {
        let config = Rc::new(RefCell::new(initial_config.clone()));
        // No tray in standalone settings mode; flag is unused (tray_mode = false).
        let window = build_window(app, config, false, Rc::new(Cell::new(false)));
        window.present();
    });

    app.run_with_args::<String>(&[]);
}

/// Run the unified tray application (`openshotx tray`): a system-tray icon plus
/// a settings window that hides to the tray instead of quitting. A single
/// process owns both, so the tray's "Settings…" re-shows the same window.
///
/// `start_hidden` controls the initial window state: a manual launch shows the
/// window, while login autostart (`--hidden`) starts in the tray only.
pub fn run_tray_app(initial_config: Config, start_hidden: bool) {
    let app = adw::Application::builder()
        .application_id("io.github.anscodelab.openshotx.tray")
        .build();

    app.connect_activate(move |app| {
        // Single-instance: a second launch (e.g. clicking the app icon again)
        // re-triggers activate on the running process. Re-show the existing
        // window instead of building another window + tray thread. Use
        // windows() (not active_window(), which is None while hidden/minimized).
        if let Some(win) = app.windows().into_iter().next() {
            win.set_visible(true);
            win.unminimize();
            win.present();
            return;
        }

        let config = Rc::new(RefCell::new(initial_config.clone()));
        // Shared flag: true once the tray icon is actually registered/visible.
        let tray_available = Rc::new(Cell::new(false));
        let window = build_window(app, config, true, tray_available.clone());
        if !start_hidden {
            window.present();
        }

        // Keep the app alive even with no visible window.
        let hold = app.hold();

        let (tx, rx) = async_channel::unbounded::<TrayMsg>();
        tray::spawn_tray_thread(tx);

        let app_for_loop = app.clone();
        let win_for_loop = window.clone();
        gtk::glib::spawn_future_local(async move {
            let _hold = hold; // dropped when the loop ends
            while let Ok(msg) = rx.recv().await {
                match msg {
                    TrayMsg::Registered => {
                        tray_available.set(true);
                    }
                    TrayMsg::ShowSettings => {
                        win_for_loop.set_visible(true);
                        win_for_loop.present();
                    }
                    TrayMsg::Quit => {
                        app_for_loop.quit();
                        break;
                    }
                }
            }
        });
    });

    app.run_with_args::<String>(&[]);
}

/// Build the settings window.
///
/// In `tray_mode` the window starts hidden, gains a "Minimize to Tray" button,
/// and hiding (Cancel / Save / close / minimize) keeps the process running in
/// the tray instead of quitting. Otherwise it behaves as a normal dialog whose
/// close quits the app.
fn build_window(
    app: &adw::Application,
    config: Rc<RefCell<Config>>,
    tray_mode: bool,
    tray_available: Rc<Cell<bool>>,
) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("openshotx Settings")
        .default_width(560)
        .default_height(500)
        .resizable(false)
        .build();

    let header = adw::HeaderBar::new();
    let cancel_btn = gtk::Button::with_label("Cancel");
    let save_btn = gtk::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");
    header.pack_start(&cancel_btn);
    header.pack_end(&save_btn);

    // Send the window to the background: hide into the tray when the icon is
    // actually live, otherwise just minimize so it stays recoverable from the
    // overview (avoids losing the window when no SNI host is showing the icon).
    let to_background = {
        let w = window.clone();
        let avail = tray_available.clone();
        move || {
            if avail.get() {
                w.set_visible(false);
            } else {
                w.minimize();
            }
        }
    };

    // Use the Adwaita header bar AS the window titlebar; otherwise the window's
    // default titlebar and this header both render, duplicating the title.
    window.set_titlebar(Some(&header));

    let notebook = gtk::Notebook::new();
    notebook.set_show_border(false);
    notebook.set_vexpand(true);

    // In tray mode, the General tab gets a "Minimize to tray" button (kept out of
    // the header to avoid confusion with the window's own minimize control).
    let minimize_cb: Option<Rc<dyn Fn()>> = if tray_mode {
        Some(Rc::new(to_background.clone()))
    } else {
        None
    };

    general_tab::append_to(&notebook, &config, minimize_cb);
    capture_tab::append_to(&notebook, &config);
    recording_tab::append_to(&notebook, &config);
    hotkeys_tab::append_to(&notebook, &config);

    window.set_child(Some(&notebook));

    // In tray mode Cancel/Save send the window to the background; otherwise they
    // close it (and the app quits).
    let dismiss = {
        let w = window.clone();
        let bg = to_background.clone();
        move || {
            if tray_mode {
                bg();
            } else {
                w.close();
            }
        }
    };

    let d = dismiss.clone();
    cancel_btn.connect_clicked(move |_| d());

    let cfg = config.clone();
    let d = dismiss.clone();
    save_btn.connect_clicked(move |_| {
        if let Err(e) = cfg.borrow().save() {
            eprintln!("Failed to save config: {}", e);
        }
        d();
    });

    // In tray mode, the window-manager close button sends to the background
    // (hide to tray, or minimize if no icon is showing) instead of quitting.
    if tray_mode {
        let bg = to_background.clone();
        window.connect_close_request(move |_| {
            bg();
            gtk::glib::Propagation::Stop
        });
    }

    window
}
