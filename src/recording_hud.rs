//! Small always-on-top, draggable HUD shown while a `record screen`/`area`/
//! `window` (MP4/WebM) recording is running: a live elapsed timer,
//! Pause/Resume, and Stop. Runs its own `gtk4::Application` main loop for
//! the recording's lifetime, driving both GStreamer bus polling and the
//! timer tick from a `glib::timeout_add_local` -- this replaces the tokio
//! Ctrl+C/bus-poll loop `start_recording` otherwise uses when the HUD can't
//! be shown. A `glib::unix_signal_add_local` SIGINT handler is still
//! installed so a terminal-launched `record screen` keeps responding to
//! Ctrl+C exactly as before, HUD or not.
//!
//! Structured as a `gtk4::Application` (mirroring `overlay.rs`'s proven
//! working pattern), not a bare `gtk4::Window` driven by a manually-created
//! `glib::MainLoop`: a top-level window with no owning `Application` was
//! observed closing itself almost immediately on this session (mutter/
//! XWayland apparently doesn't keep such a window alive/interactive) --
//! matches how `select_area()`'s overlay is built, which does not have
//! that problem.

use crate::recording::RecordError;
use gdk4_x11::X11Surface;
use gst::prelude::*;
use gstreamer as gst;
use gtk4::glib;
use gtk4::glib::object::Cast;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Why the HUD's main loop ended.
enum HudOutcome {
    /// User clicked Stop, closed the window, or SIGINT was received, and
    /// the pipeline reached EOS (or the wait for it timed out).
    Stopped,
    /// The pipeline reported an error on the bus before it could stop.
    PipelineError(String),
}

/// Mutable state shared between the HUD's button callbacks, its SIGINT
/// handler, and its periodic tick -- all running on the same GLib main
/// loop thread, so plain `Cell`/`RefCell` (no locking needed).
struct HudState {
    pipeline: gst::Pipeline,
    paused: Cell<bool>,
    start: Instant,
    /// Total time spent paused across all completed pause intervals.
    paused_accum: Cell<Duration>,
    /// When the current pause began, if paused right now.
    paused_since: Cell<Option<Instant>>,
    stopping: Cell<bool>,
    stop_deadline: Cell<Option<Instant>>,
}

impl HudState {
    /// Wall-clock recording time so far, excluding time spent paused.
    fn elapsed(&self) -> Duration {
        let raw = self.start.elapsed();
        let mut paused_total = self.paused_accum.get();
        if let Some(since) = self.paused_since.get() {
            paused_total += since.elapsed();
        }
        raw.saturating_sub(paused_total)
    }

    /// Resume if currently paused (EOS needs a running pipeline to
    /// propagate), then send EOS and arm a deadline in case it never
    /// arrives on the bus. Idempotent: a second call while already
    /// stopping does nothing.
    fn begin_stop(&self) {
        if self.stopping.replace(true) {
            return;
        }
        self.resume_if_paused();
        self.pipeline.send_event(gst::event::Eos::new());
        self.stop_deadline.set(Some(Instant::now() + Duration::from_secs(5)));
    }

    fn resume_if_paused(&self) {
        if !self.paused.get() {
            return;
        }
        let _ = self.pipeline.set_state(gst::State::Playing);
        if let Some(since) = self.paused_since.take() {
            self.paused_accum.set(self.paused_accum.get() + since.elapsed());
        }
        self.paused.set(false);
    }

    /// Toggle paused/playing. Returns the new paused state. A no-op (stays
    /// playing) once stopping has begun.
    fn toggle_pause(&self) -> bool {
        if self.stopping.get() {
            return false;
        }
        if self.paused.get() {
            self.resume_if_paused();
        } else {
            let _ = self.pipeline.set_state(gst::State::Paused);
            self.paused_since.set(Some(Instant::now()));
            self.paused.set(true);
        }
        self.paused.get()
    }
}

fn format_elapsed(d: Duration) -> String {
    let total_secs = d.as_secs();
    format!("{:02}:{:02}", total_secs / 60, total_secs % 60)
}

/// Best-effort: ask the window manager to keep `window` above other
/// windows. GTK4 dropped the toolkit-level "always on top" API (`gtk4::Window`
/// has no `set_keep_above`) -- Wayland's security model deliberately
/// disallows arbitrary clients from doing this, and GTK4 removed the
/// cross-platform API along with it. This sends the standard EWMH
/// `_NET_WM_STATE_ABOVE` ClientMessage directly, over the same XWayland
/// connection `force_x11_backend` already relies on for this window to
/// exist as a real X11 window at all. Silently does nothing if the surface
/// isn't an X11 surface or the message can't be sent: the HUD still works,
/// it just might not stay on top of everything.
fn request_always_on_top(window: &gtk4::ApplicationWindow) {
    let Some(surface) = window.surface() else { return };
    let Some(x11_surface) = surface.downcast_ref::<X11Surface>() else { return };
    let xid = x11_surface.xid() as u32;

    let result: Result<(), Box<dyn std::error::Error>> = (|| {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt, EventMask};

        let (conn, screen_num) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen_num].root;
        let wm_state = conn.intern_atom(false, b"_NET_WM_STATE")?.reply()?.atom;
        let wm_state_above = conn.intern_atom(false, b"_NET_WM_STATE_ABOVE")?.reply()?.atom;

        // EWMH _NET_WM_STATE client message: data[0]=1 (_NET_WM_STATE_ADD),
        // data[1]=the state atom to add, data[3]=1 (source indication:
        // normal application).
        let event = ClientMessageEvent::new(32, xid, wm_state, [1u32, wm_state_above, 0, 1, 0]);
        conn.send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )?;
        conn.flush()?;
        Ok(())
    })();

    if let Err(e) = result {
        eprintln!("Warning: failed to keep recording HUD on top: {}", e);
    }
}

/// Build and show the HUD window on `app`, wiring its buttons, SIGINT
/// handler, and periodic tick against `pipeline`. Ends `app`'s main loop
/// (via `app.quit()`) once stopped, recording the outcome into
/// `outcome_slot` for `run()` to read after `app.run_with_args()` returns.
fn build_hud(app: &gtk4::Application, pipeline: gst::Pipeline, outcome_slot: Rc<RefCell<Option<HudOutcome>>>) {
    let window = gtk4::ApplicationWindow::builder()
        .application(app)
        .title("Recording")
        .decorated(false)
        .resizable(false)
        .default_width(220)
        .default_height(44)
        .build();

    let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    container.set_margin_top(8);
    container.set_margin_bottom(8);
    container.set_margin_start(12);
    container.set_margin_end(12);

    let dot = gtk4::DrawingArea::new();
    dot.set_content_width(10);
    dot.set_content_height(10);
    dot.set_valign(gtk4::Align::Center);
    dot.set_draw_func(|_, cr, w, h| {
        cr.set_source_rgb(0.90, 0.20, 0.20);
        cr.arc(w as f64 / 2.0, h as f64 / 2.0, (w.min(h) as f64) / 2.0, 0.0, 2.0 * std::f64::consts::PI);
        let _ = cr.fill();
    });

    let timer_label = gtk4::Label::new(Some("00:00"));
    timer_label.set_valign(gtk4::Align::Center);
    timer_label.set_hexpand(true);
    timer_label.set_halign(gtk4::Align::Start);

    let pause_button = gtk4::Button::with_label("Pause");
    let stop_button = gtk4::Button::with_label("Stop");

    container.append(&dot);
    container.append(&timer_label);
    container.append(&pause_button);
    container.append(&stop_button);

    // Wrap the content in a WindowHandle so the user can drag the HUD
    // anywhere on screen by clicking any non-interactive part of it (the
    // same widget GTK4 headerbars use for exactly this) -- buttons inside
    // still receive their own clicks normally; only the surrounding area
    // acts as a drag handle. This is a user-initiated interactive move
    // (compositor-mediated), which Wayland allows unlike programmatic
    // absolute positioning (GTK4 dropped that API entirely).
    let handle = gtk4::WindowHandle::new();
    handle.set_child(Some(&container));
    window.set_child(Some(&handle));

    let state = Rc::new(HudState {
        pipeline,
        paused: Cell::new(false),
        start: Instant::now(),
        paused_accum: Cell::new(Duration::ZERO),
        paused_since: Cell::new(None),
        stopping: Cell::new(false),
        stop_deadline: Cell::new(None),
    });

    pause_button.connect_clicked({
        let state = state.clone();
        let pause_button = pause_button.clone();
        move |_| {
            let now_paused = state.toggle_pause();
            pause_button.set_label(if now_paused { "Resume" } else { "Pause" });
        }
    });

    stop_button.connect_clicked({
        let state = state.clone();
        move |_| state.begin_stop()
    });

    window.connect_close_request({
        let state = state.clone();
        move |_| {
            state.begin_stop();
            glib::Propagation::Proceed
        }
    });

    glib::source::unix_signal_add_local(libc::SIGINT, {
        let state = state.clone();
        move || {
            state.begin_stop();
            glib::ControlFlow::Continue
        }
    });

    glib::source::timeout_add_local(Duration::from_millis(250), {
        let state = state.clone();
        let timer_label = timer_label.clone();
        let app = app.clone();
        let outcome_slot = outcome_slot.clone();
        move || {
            timer_label.set_text(&format_elapsed(state.elapsed()));

            if let Some(bus) = state.pipeline.bus() {
                for msg in bus.iter_timed(gst::ClockTime::ZERO) {
                    match msg.view() {
                        gst::MessageView::Eos(..) => {
                            *outcome_slot.borrow_mut() = Some(HudOutcome::Stopped);
                            app.quit();
                            return glib::ControlFlow::Break;
                        }
                        gst::MessageView::Error(err) => {
                            *outcome_slot.borrow_mut() =
                                Some(HudOutcome::PipelineError(err.error().to_string()));
                            app.quit();
                            return glib::ControlFlow::Break;
                        }
                        _ => {}
                    }
                }
            }

            if state.stopping.get() {
                if let Some(deadline) = state.stop_deadline.get() {
                    if Instant::now() > deadline {
                        eprintln!("Timeout waiting for EOS. Forcing stop.");
                        *outcome_slot.borrow_mut() = Some(HudOutcome::Stopped);
                        app.quit();
                        return glib::ControlFlow::Break;
                    }
                }
            }

            glib::ControlFlow::Continue
        }
    });

    window.present();
    request_always_on_top(&window);
}

/// Run the recording HUD until the pipeline stops (Stop button, window
/// close, SIGINT, or a pipeline error), driving `pipeline` entirely from
/// this `Application`'s main loop. On `Ok(())`, `pipeline` has already
/// reached EOS (or the wait for it timed out) and is ready for the
/// caller's own `set_state(Null)` cleanup, exactly as after the non-HUD
/// Ctrl+C loop.
pub fn run(pipeline: gst::Pipeline) -> Result<(), RecordError> {
    // Same reasoning as the selection overlay: force X11 so this window
    // (a) reliably exists and stays interactive on a Wayland session and
    // (b) has a real XID for `request_always_on_top` to target.
    let _gdk_backend_guard = crate::overlay::force_x11_backend();

    // NON_UNIQUE: this is one HUD per recording process; it must never
    // hand off to some other still-registered app instance via D-Bus
    // activation (same reasoning as the selection overlay's own app id).
    let app = gtk4::Application::builder()
        .application_id("com.openshotx.recording-hud")
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();

    let outcome_slot: Rc<RefCell<Option<HudOutcome>>> = Rc::new(RefCell::new(None));

    {
        let outcome_slot = outcome_slot.clone();
        app.connect_activate(move |app| {
            build_hud(app, pipeline.clone(), outcome_slot.clone());
        });
    }

    let _ = app.run_with_args::<String>(&[]);

    let outcome = outcome_slot.borrow_mut().take();
    match outcome {
        Some(HudOutcome::PipelineError(msg)) => Err(RecordError::GStreamerError(msg)),
        Some(HudOutcome::Stopped) | None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_elapsed() {
        assert_eq!(format_elapsed(Duration::from_secs(0)), "00:00");
        assert_eq!(format_elapsed(Duration::from_secs(5)), "00:05");
        assert_eq!(format_elapsed(Duration::from_secs(65)), "01:05");
        assert_eq!(format_elapsed(Duration::from_secs(3661)), "61:01");
    }

    #[test]
    fn test_pause_resume_excludes_paused_time_from_elapsed() {
        gst::init().ok();
        let state = HudState {
            pipeline: gst::Pipeline::new(),
            paused: Cell::new(false),
            start: Instant::now(),
            paused_accum: Cell::new(Duration::ZERO),
            paused_since: Cell::new(None),
            stopping: Cell::new(false),
            stop_deadline: Cell::new(None),
        };

        std::thread::sleep(Duration::from_millis(60));
        assert!(state.toggle_pause(), "toggle_pause should report paused");
        let elapsed_at_pause = state.elapsed();

        std::thread::sleep(Duration::from_millis(200));
        // While paused, elapsed() must stay near where it was when the
        // pause began -- checked as an absolute difference (not a strict
        // ordering) since either read can be delayed a little by thread
        // scheduling, which could otherwise make the "before" or "after"
        // value spuriously larger.
        let elapsed_while_paused = state.elapsed();
        let drift = elapsed_while_paused.abs_diff(elapsed_at_pause);
        assert!(
            drift < Duration::from_millis(50),
            "elapsed drifted by {:?} while paused (expected near-zero): before={:?} after={:?}",
            drift, elapsed_at_pause, elapsed_while_paused
        );

        assert!(!state.toggle_pause(), "toggle_pause should report resumed");
        std::thread::sleep(Duration::from_millis(60));
        let elapsed_after_resume = state.elapsed();
        // Elapsed grew by roughly the two "playing" intervals (~60ms +
        // ~60ms), not by the 200ms spent paused in between -- generous
        // bounds to absorb scheduling jitter without losing the point.
        assert!(elapsed_after_resume >= Duration::from_millis(80));
        assert!(elapsed_after_resume < Duration::from_millis(400));
    }
}
