//! Small always-on-top preview window shown after a screenshot capture: a
//! thumbnail of what was just saved, plus Copy/Open/Close actions, that
//! auto-dismisses after a configurable delay (paused while hovered).
//!
//! Runs its own `gtk4::Application` main loop (mirroring `overlay.rs`'s and
//! `recording_hud.rs`'s proven working pattern -- a bare `gtk4::Window` with
//! no owning `Application` was observed closing itself almost immediately
//! on this session), on the calling process's own thread. Callers show the
//! preview synchronously after saving, right before the CLI process would
//! otherwise just exit.

use crate::capture::{copy_image_to_clipboard, open_in_editor};
use gtk4::{gdk, glib, prelude::*};
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

const WINDOW_WIDTH: i32 = 220;
const WINDOW_HEIGHT: i32 = 190;
const THUMBNAIL_HEIGHT: i32 = 130;
const EDGE_MARGIN: i32 = 24;

/// Show the post-capture preview window for `path`, blocking until it's
/// dismissed (auto-close timeout, Close button, ESC, or window close).
/// `auto_close_seconds` of `0` disables the auto-dismiss timer entirely
/// (stays open until manually dismissed). `editor` is the configured
/// editor command (see `capture::open_in_editor`) used by the Open button.
pub fn show(path: &Path, editor: &str, auto_close_seconds: u32) {
    let _gdk_backend_guard = crate::overlay::force_x11_backend();

    // NON_UNIQUE: one preview per capture; must never hand off to some
    // other still-registered instance via D-Bus activation (same
    // reasoning as the selection overlay's and recording HUD's app ids).
    let app = gtk4::Application::builder()
        .application_id("com.openshotx.preview")
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();

    let path = path.to_path_buf();
    let editor = editor.to_string();
    app.connect_activate(move |app| {
        build_preview(app, &path, &editor, auto_close_seconds);
    });

    let _ = app.run_with_args::<String>(&[]);
}

fn build_preview(app: &gtk4::Application, path: &Path, editor: &str, auto_close_seconds: u32) {
    let window = gtk4::ApplicationWindow::builder()
        .application(app)
        .title("Screenshot")
        .decorated(false)
        .resizable(false)
        .default_width(WINDOW_WIDTH)
        .default_height(WINDOW_HEIGHT)
        .build();
    window.add_css_class("openshotx-preview");

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    outer.set_margin_top(10);
    outer.set_margin_bottom(10);
    outer.set_margin_start(10);
    outer.set_margin_end(10);

    let picture = gtk4::Picture::for_filename(path);
    picture.set_content_fit(gtk4::ContentFit::Contain);
    picture.set_size_request(WINDOW_WIDTH - 20, THUMBNAIL_HEIGHT);
    picture.set_can_shrink(true);

    let button_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    button_row.set_halign(gtk4::Align::End);
    let copy_button = gtk4::Button::with_label("Copy");
    let open_button = gtk4::Button::with_label("Open");
    let close_button = gtk4::Button::with_label("Close");
    button_row.append(&copy_button);
    button_row.append(&open_button);
    button_row.append(&close_button);

    outer.append(&picture);
    outer.append(&button_row);

    // WindowHandle so the user can drag the preview elsewhere by clicking
    // any non-button part of it, same technique the recording HUD uses.
    let handle = gtk4::WindowHandle::new();
    handle.set_child(Some(&outer));
    window.set_child(Some(&handle));

    copy_button.connect_clicked({
        let path = path.to_path_buf();
        move |_| {
            if let Err(e) = copy_image_to_clipboard(&path) {
                eprintln!("Warning: Failed to copy image to clipboard: {}", e);
            }
        }
    });

    open_button.connect_clicked({
        let path = path.to_path_buf();
        let editor = editor.to_string();
        let window = window.clone();
        move |_| {
            if let Err(e) = open_in_editor(&path, &editor) {
                eprintln!("Warning: Failed to open screenshot: {}", e);
            }
            window.close();
        }
    });

    close_button.connect_clicked({
        let window = window.clone();
        move |_| window.close()
    });

    let key_controller = gtk4::EventControllerKey::new();
    key_controller.connect_key_pressed({
        let window = window.clone();
        move |_, key, _, _| {
            if key == gdk::Key::Escape {
                window.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    if auto_close_seconds > 0 {
        install_auto_close(&window, auto_close_seconds);
    }

    position_bottom_right(&window);
    window.present();
    crate::overlay::request_always_on_top(&window);
}

/// Auto-dismiss `window` after `seconds` of not being hovered, ticking once
/// per second so hovering resets nothing already elapsed but simply pauses
/// the countdown -- the user gets the full delay back only once the
/// pointer actually leaves, not on every tick while it's still inside.
fn install_auto_close(window: &gtk4::ApplicationWindow, seconds: u32) {
    let hovered = Rc::new(Cell::new(false));
    let motion = gtk4::EventControllerMotion::new();
    motion.connect_enter({
        let hovered = hovered.clone();
        move |_, _, _| hovered.set(true)
    });
    motion.connect_leave({
        let hovered = hovered.clone();
        move |_| hovered.set(false)
    });
    window.add_controller(motion);

    let remaining = Rc::new(Cell::new(seconds));
    glib::source::timeout_add_local(Duration::from_secs(1), {
        let window_weak = window.downgrade();
        let hovered = hovered.clone();
        let remaining = remaining.clone();
        move || {
            if hovered.get() {
                return glib::ControlFlow::Continue;
            }
            let left = remaining.get().saturating_sub(1);
            remaining.set(left);
            if left == 0 {
                if let Some(window) = window_weak.upgrade() {
                    window.close();
                }
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        }
    });
}

/// Position the preview near the bottom-right corner of the primary
/// monitor. GTK4 dropped programmatic window positioning on Wayland
/// entirely (the security model treats absolute placement as a compositor
/// decision), so this only takes effect on the X11/XWayland surface
/// `force_x11_backend` guarantees this window has -- set via the
/// `default_width`/`default_height` the window was already built with and
/// an X11 `ConfigureWindow` request over the same connection
/// `request_always_on_top` uses. Best-effort: if it fails, the window
/// still opens wherever the window manager places it.
fn position_bottom_right(window: &gtk4::ApplicationWindow) {
    let Some((screen_w, screen_h)) = crate::overlay::primary_monitor_geometry() else { return };
    let x = (screen_w - WINDOW_WIDTH as f64 - EDGE_MARGIN as f64).max(0.0) as i32;
    let y = (screen_h - WINDOW_HEIGHT as f64 - EDGE_MARGIN as f64).max(0.0) as i32;

    // Actual placement happens once the X11 surface exists; realize it
    // first (present() below triggers this too, but doing it here lets us
    // move the window before its first paint so there's no visible jump).
    WidgetExt::realize(window);
    let Some(surface) = window.surface() else { return };
    let Some(x11_surface) = surface.downcast_ref::<gdk4_x11::X11Surface>() else { return };
    let xid = x11_surface.xid() as u32;

    let result: Result<(), Box<dyn std::error::Error>> = (|| {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt};

        let (conn, _screen_num) = x11rb::connect(None)?;
        conn.configure_window(xid, &ConfigureWindowAux::new().x(x).y(y))?;
        conn.flush()?;
        Ok(())
    })();

    if let Err(e) = result {
        eprintln!("Warning: failed to position preview window: {}", e);
    }
}
