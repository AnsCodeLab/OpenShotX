//! GTK4 overlay for interactive area selection
//!
//! This module provides a full-screen transparent window that allows users
//! to select a screen area using mouse drag. Only used for X11 backend.

use gtk4::{
    gdk,
    glib::{self, clone},
    prelude::*,
    Application, ApplicationWindow, EventControllerKey, GestureDrag,
};
use gtk4::gdk::Key;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::backend::{CaptureData, DisplayBackend, DisplayResult, WaylandBackend, X11Backend};

/// Selected area coordinates
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionArea {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl SelectionArea {
    /// Normalize the selection (handle negative width/height from dragging)
    pub fn normalize(mut self) -> Self {
        if self.width < 0 {
            self.x += self.width;
            self.width = self.width.abs();
        }
        if self.height < 0 {
            self.y += self.height;
            self.height = self.height.abs();
        }
        self
    }

    /// Check if the selection is valid (has positive area)
    pub fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SelectionError {
    #[error("GTK initialization failed: {0}")]
    InitError(String),

    #[error("Selection was cancelled by user")]
    Cancelled,

    #[error("An area selection is already in progress")]
    AlreadyInProgress,

    #[error("Failed to capture the selected area: {0}")]
    CaptureFailed(String),
}

/// Which action to perform with a selected area, chosen from the on-screen
/// control panel after the drag finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AreaAction {
    #[default]
    Capture,
    Record,
}

/// Result of picking a rectangle through the control panel: which action
/// was chosen, the rectangle itself, and the monitor-0 screen dimensions
/// used to lay out the panel at the moment of the hit-test (needed by
/// callers that must crop a differently-sized capture down to this area).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AreaPick {
    pub action: AreaAction,
    pub area: SelectionArea,
    pub screen_width: u32,
    pub screen_height: u32,
}

/// Result of an area selection made through the control panel.
///
/// `Captured` carries pixels already grabbed synchronously, on the GTK
/// main thread, while the overlay window was still open and focused --
/// see `capture_area_pixels`'s doc comment for why this can't be
/// deferred to the caller. `Record` only carries geometry: the actual
/// recording pipeline starts separately afterward and negotiates its own
/// (already reliable, token-persisted) ScreenCast session.
#[derive(Debug)]
pub enum AreaOutcome {
    Captured(CaptureData),
    Record(AreaPick),
}

/// Result of an area selection: the outcome, or `None` if the user
/// cancelled.
pub type AreaSelectionResult = Result<Option<AreaOutcome>, SelectionError>;

/// State for the area selector overlay
struct SelectorState {
    start_x: f64,
    start_y: f64,
    current_x: f64,
    current_y: f64,
    is_dragging: bool,
    cancelled: bool,
    completed: bool,
    /// Action highlighted as primary in the control panel (the one matching
    /// whichever command opened the overlay).
    default_action: AreaAction,
    /// Absolute press position when a press started while the control panel
    /// was showing (`completed == true`). Set on `drag_begin`, consumed on
    /// the matching `drag_end` to hit-test the panel buttons. Kept separate
    /// from `start_x`/`start_y` so a stray press over the panel never
    /// clobbers the already-drawn selection rectangle.
    panel_press: Option<(f64, f64)>,
}

impl Default for SelectorState {
    fn default() -> Self {
        Self {
            start_x: 0.0,
            start_y: 0.0,
            current_x: 0.0,
            current_y: 0.0,
            is_dragging: false,
            cancelled: false,
            completed: false,
            default_action: AreaAction::default(),
            panel_press: None,
        }
    }
}

/// RAII guard that restores the prior `GDK_BACKEND` env var (or removes it
/// if it wasn't set before) when dropped, so callers of `force_x11_backend`
/// leave the process env exactly as they found it on every exit path:
/// success, an init error, or an unwind.
pub(crate) struct GdkBackendGuard {
    prior: Option<String>,
}

impl Drop for GdkBackendGuard {
    fn drop(&mut self) {
        // SAFETY: by the time this guard drops, the caller's blocking GTK
        // main-loop call has already returned, so GDK has long since read
        // `GDK_BACKEND` during initialization; nothing later in this
        // process reads it, so restoring here has no concurrent-reader
        // race.
        unsafe {
            match &self.prior {
                Some(value) => std::env::set_var("GDK_BACKEND", value),
                None => std::env::remove_var("GDK_BACKEND"),
            }
        }
    }
}

/// GDK prefers the Wayland backend when both are reachable, which would
/// make any GTK4 window created this way unreachable on a Wayland session
/// (issue #1: the overlay's Capture/Record panel was unreachable for
/// exactly this reason). Forces X11 for the lifetime of the returned
/// guard -- shared by the selection overlay and the recording HUD, the
/// two GTK entry points in this codebase that both need a real X11 window
/// (the overlay for coordinate selection, the HUD for the EWMH
/// always-on-top hint GTK4 no longer exposes a toolkit API for).
///
/// SAFETY: must be called before the caller's `Application`/`Window`
/// triggers GDK init, and before any other thread in this process has
/// reason to read `GDK_BACKEND` -- true for both current call sites, each
/// of which sets this immediately before its first GTK call.
pub(crate) fn force_x11_backend() -> GdkBackendGuard {
    let prior = std::env::var("GDK_BACKEND").ok();
    unsafe {
        std::env::set_var("GDK_BACKEND", "x11");
    }
    GdkBackendGuard { prior }
}

/// Path to the advisory single-instance lock file: only one area-selection
/// overlay may be open at a time.
fn overlay_lock_path() -> std::path::PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("openshotx-overlay.lock")
}

/// Holds an exclusive, non-blocking `flock` on the overlay lock file for as
/// long as this guard lives, so a double-pressed hotkey (or tray + hotkey
/// racing) can't stack multiple overlay windows on top of each other --
/// each new one drawing input away from whichever the user actually sees.
/// Deliberately a kernel-level file lock, not GTK's D-Bus GApplication
/// uniqueness: the kernel releases it automatically if the holding process
/// dies without calling `drop` (crash, `kill -9`), so a stuck lock can
/// never require manual cleanup -- unlike GApplication uniqueness, whose
/// stale-process handoff is exactly what made a second invocation's window
/// never appear at all (issue #1/#2).
struct OverlayLock {
    _file: std::fs::File,
}

impl OverlayLock {
    /// Try to acquire the lock. Returns `Ok(None)` if another instance
    /// already holds it (a selection is already in progress).
    fn try_acquire() -> std::io::Result<Option<Self>> {
        use std::os::unix::io::AsRawFd;

        let path = overlay_lock_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path)?;

        // SAFETY: flock is a simple advisory-lock syscall on a valid fd we
        // just opened ourselves; no aliasing or lifetime hazards.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == 0 {
            Ok(Some(Self { _file: file }))
        } else {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                Ok(None)
            } else {
                Err(err)
            }
        }
    }
}

/// GTK4 overlay window for interactive area selection
pub struct AreaSelector {
    state: Arc<Mutex<SelectorState>>,
}

impl AreaSelector {
    /// Create a new area selector. `default_action` is highlighted as the
    /// primary button in the control panel (the action the caller asked
    /// for), but the user is always free to pick either button.
    pub fn new(default_action: AreaAction) -> Self {
        Self {
            state: Arc::new(Mutex::new(SelectorState {
                default_action,
                ..SelectorState::default()
            })),
        }
    }

    /// Run the area selection dialog
    ///
    /// Returns `Ok(Some(AreaOutcome))` if the user drew a region and
    /// picked Capture or Record from the control panel.
    /// Returns `Ok(None)` if user cancelled (ESC).
    /// Returns `Err` if initialization, or the capture itself, failed.
    pub fn run(&self) -> AreaSelectionResult {
        // Only one overlay may be open at a time: a double-pressed hotkey
        // or tray+hotkey race must not stack multiple fullscreen windows,
        // each intercepting input independently of what's visually on top.
        let _overlay_lock = match OverlayLock::try_acquire() {
            Ok(Some(lock)) => lock,
            Ok(None) => return Err(SelectionError::AlreadyInProgress),
            Err(e) => {
                return Err(SelectionError::InitError(format!(
                    "Failed to acquire overlay lock: {}",
                    e
                )))
            }
        };

        let state = self.state.clone();
        let (result_tx, result_rx) = std::sync::mpsc::channel();

        // GDK prefers the Wayland backend when both are reachable, which
        // would make this overlay unreachable on a Wayland session (issue
        // #1). Force X11 for the lifetime of this call.
        let _gdk_backend_guard = force_x11_backend();

        // Create application. NON_UNIQUE: each invocation is a fresh,
        // one-shot process (hotkey/tray/CLI); it must never hand off to a
        // still-running instance from a previous invocation via D-Bus
        // activation, or the new process's window would never appear.
        let app = Application::builder()
            .application_id("com.openshotx.screenshot")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();

        // Clone state for the activate handler
        let state_activate = state.clone();
        app.connect_activate(move |application| {
            setup_window(application, state_activate.clone(), result_tx.clone());
        });

        // Run the application
        let _ = app.run_with_args::<String>(&[]);

        // Get the result
        match result_rx.recv() {
            Ok(Ok(selection)) => Ok(selection),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(SelectionError::InitError("No result received".into())),
        }
    }
}

/// Bounding box of a control-panel button, in the same screen-pixel space as
/// the selection rectangle.
struct ButtonRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl ButtonRect {
    fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

/// Geometry (width, height) of the first monitor, used both to draw the
/// overlay and to lay out the control panel.
fn primary_monitor_geometry() -> Option<(f64, f64)> {
    let display = gdk::Display::default()?;
    let monitors = display.monitors();
    if monitors.n_items() == 0 {
        return None;
    }
    let monitor = monitors.item(0)?.downcast::<gdk::Monitor>().ok()?;
    let geometry = monitor.geometry();
    Some((geometry.width() as f64, geometry.height() as f64))
}

/// Lay out the two control-panel buttons (Capture, Record) below the
/// selection rectangle, clamped to stay on screen (flips above the
/// selection if there's no room below).
fn toolbar_layout(
    sel_x: f64,
    sel_y: f64,
    sel_w: f64,
    sel_h: f64,
    screen_w: f64,
    screen_h: f64,
) -> (ButtonRect, ButtonRect) {
    const BTN_W: f64 = 120.0;
    const BTN_H: f64 = 40.0;
    const GAP: f64 = 10.0;
    const MARGIN: f64 = 12.0;

    let total_w = BTN_W * 2.0 + GAP;
    let mut bar_x = sel_x + sel_w / 2.0 - total_w / 2.0;
    bar_x = bar_x.clamp(MARGIN, (screen_w - total_w - MARGIN).max(MARGIN));

    let mut bar_y = sel_y + sel_h + MARGIN;
    if bar_y + BTN_H + MARGIN > screen_h {
        bar_y = (sel_y - MARGIN - BTN_H).max(MARGIN);
    }

    let capture_rect = ButtonRect { x: bar_x, y: bar_y, w: BTN_W, h: BTN_H };
    let record_rect = ButtonRect { x: bar_x + BTN_W + GAP, y: bar_y, w: BTN_W, h: BTN_H };
    (capture_rect, record_rect)
}

/// Draw one control-panel button: filled rect with a centered label,
/// optionally preceded by a small solid-color dot (drawn as a cairo shape,
/// never a font glyph -- emoji rendered incorrectly via cairo's toy text
/// API on this machine).
/// `primary` renders the button matching the action that opened the
/// overlay with an accent color; the other stays neutral.
fn draw_button(context: &gtk4::cairo::Context, rect: &ButtonRect, label: &str, dot_color: Option<(f64, f64, f64)>, primary: bool) {
    if primary {
        context.set_source_rgba(0.20, 0.47, 0.96, 0.95);
    } else {
        context.set_source_rgba(0.20, 0.20, 0.20, 0.90);
    }
    context.rectangle(rect.x, rect.y, rect.w, rect.h);
    let _ = context.fill();

    context.set_source_rgba(1.0, 1.0, 1.0, 0.9);
    context.set_line_width(1.0);
    context.rectangle(rect.x, rect.y, rect.w, rect.h);
    let _ = context.stroke();

    context.set_font_size(14.0);
    context.set_source_rgba(1.0, 1.0, 1.0, 1.0);
    let extents = context.text_extents(label).ok();
    let text_width = extents.as_ref().map(|e| e.width()).unwrap_or(0.0);

    // A leading dot is drawn as a cairo shape, not a font glyph: cairo's
    // toy text API doesn't reliably render color emoji (confirmed to
    // render incorrectly on this machine), so anything beyond plain ASCII
    // text is drawn directly instead of relying on a font to have the
    // glyph.
    const DOT_RADIUS: f64 = 4.0;
    const DOT_GAP: f64 = 8.0;
    let dot_width = if dot_color.is_some() { DOT_RADIUS * 2.0 + DOT_GAP } else { 0.0 };

    let group_width = dot_width + text_width;
    let group_x = rect.x + rect.w / 2.0 - group_width / 2.0;
    let center_y = rect.y + rect.h / 2.0;

    if let Some((r, g, b)) = dot_color {
        context.set_source_rgba(r, g, b, 1.0);
        context.arc(group_x + DOT_RADIUS, center_y, DOT_RADIUS, 0.0, 2.0 * std::f64::consts::PI);
        let _ = context.fill();
        context.set_source_rgba(1.0, 1.0, 1.0, 1.0);
    }

    if let Some(extents) = extents {
        let text_x = group_x + dot_width - extents.x_bearing();
        let text_y = center_y - extents.height() / 2.0 - extents.y_bearing();
        context.move_to(text_x, text_y);
        let _ = context.show_text(label);
    }
}

/// Outcome of a `drag_begin` event, decided purely from state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BeginOutcome {
    /// No panel was showing: started tracking a fresh selection drag.
    StartedDrag,
    /// The panel was showing: the press was captured for hit-testing on
    /// release, and the existing selection was left untouched.
    CapturedPanelPress,
}

/// Handle a drag-gesture press at absolute position `(x, y)`.
fn handle_drag_begin(state: &mut SelectorState, x: f64, y: f64) -> BeginOutcome {
    if state.completed {
        // Don't touch the already-drawn selection; just remember where
        // this press landed so `handle_drag_end` can hit-test it.
        state.panel_press = Some((x, y));
        return BeginOutcome::CapturedPanelPress;
    }
    state.start_x = x;
    state.start_y = y;
    state.current_x = x;
    state.current_y = y;
    state.is_dragging = true;
    BeginOutcome::StartedDrag
}

/// Handle a drag-gesture update at `(offset_x, offset_y)` from the press.
/// Returns `true` if the selection rectangle changed and needs a redraw.
fn handle_drag_update(state: &mut SelectorState, offset_x: f64, offset_y: f64) -> bool {
    if state.completed || !state.is_dragging {
        return false;
    }
    state.current_x = state.start_x + offset_x;
    state.current_y = state.start_y + offset_y;
    true
}

/// Outcome of a `drag_end` event, decided purely from state.
#[derive(Debug, Clone, Copy, PartialEq)]
enum EndOutcome {
    /// Neither a selection drag nor a panel press was in progress.
    Ignored,
    /// A valid selection was drawn: the panel should now be shown.
    SelectionCompleted,
    /// An invalid (zero-area) drag: treat as cancellation.
    Cancelled,
    /// A panel button was hit: this is the final result.
    PanelAction(AreaAction, SelectionArea, (f64, f64)),
    /// The press/release landed outside the panel: selection discarded so
    /// the user can drag a new one.
    PanelDismissed,
}

/// Handle a drag-gesture release at `(offset_x, offset_y)` from the press
/// that started this sequence (as reported by `GestureDrag`). `screen` is
/// the monitor size used to lay out the panel, when available.
fn handle_drag_end(
    state: &mut SelectorState,
    offset_x: f64,
    offset_y: f64,
    screen: Option<(f64, f64)>,
) -> EndOutcome {
    if let Some((press_x, press_y)) = state.panel_press.take() {
        // This press/release started while the panel was showing: hit-test
        // the release point against the panel buttons instead of touching
        // the selection rectangle.
        let release_x = press_x + offset_x;
        let release_y = press_y + offset_y;
        let sel_x = state.start_x.min(state.current_x);
        let sel_y = state.start_y.min(state.current_y);
        let sel_w = (state.current_x - state.start_x).abs();
        let sel_h = (state.current_y - state.start_y).abs();

        let hit = screen.and_then(|(screen_w, screen_h)| {
            let (capture_rect, record_rect) =
                toolbar_layout(sel_x, sel_y, sel_w, sel_h, screen_w, screen_h);
            if capture_rect.contains(release_x, release_y) {
                Some((AreaAction::Capture, (screen_w, screen_h)))
            } else if record_rect.contains(release_x, release_y) {
                Some((AreaAction::Record, (screen_w, screen_h)))
            } else {
                None
            }
        });

        return match hit {
            Some((action, screen_dims)) => {
                let area = SelectionArea {
                    x: sel_x as i32,
                    y: sel_y as i32,
                    width: sel_w as i32,
                    height: sel_h as i32,
                };
                EndOutcome::PanelAction(action, area, screen_dims)
            }
            None => {
                state.completed = false;
                EndOutcome::PanelDismissed
            }
        };
    }

    if state.completed || !state.is_dragging {
        return EndOutcome::Ignored;
    }
    state.current_x = state.start_x + offset_x;
    state.current_y = state.start_y + offset_y;
    state.is_dragging = false;

    let area = SelectionArea {
        x: state.start_x as i32,
        y: state.start_y as i32,
        width: (state.current_x - state.start_x) as i32,
        height: (state.current_y - state.start_y) as i32,
    }
    .normalize();

    if area.is_valid() {
        // Keep the window open: show the Capture/Record control panel
        // below the selection and wait for the user to pick.
        state.completed = true;
        EndOutcome::SelectionCompleted
    } else {
        EndOutcome::Cancelled
    }
}

/// Capture the pixels of `area`, choosing the right backend for this
/// session: on Wayland, `X11Backend::capture_area` can't see real desktop
/// content (XWayland's root window doesn't reflect Wayland client
/// compositing), so grab the full monitor through the portal and crop
/// client-side; on native X11, capture the region directly.
///
/// Called synchronously from the overlay's `drag_end` handler, before the
/// window closes -- see the call site's comment for why deferring this
/// to the caller (as it used to) silently broke every area screenshot.
fn capture_area_pixels(area: SelectionArea) -> DisplayResult<CaptureData> {
    if WaylandBackend::is_supported() {
        WaylandBackend::new()?
            .capture_screen()?
            .crop(area.x, area.y, area.width, area.height)
    } else {
        X11Backend::new()?.capture_area(area.x, area.y, area.width, area.height)
    }
}

/// Setup the overlay window (standalone function to avoid lifetime issues)
fn setup_window(
    app: &Application,
    state: Arc<Mutex<SelectorState>>,
    result_tx: std::sync::mpsc::Sender<AreaSelectionResult>,
) {
    // Get the display and monitor for screen dimensions
    let display = match gdk::Display::default() {
        Some(d) => d,
        None => {
            let _ = result_tx.send(Err(SelectionError::InitError("No display found".into())));
            return;
        }
    };

    // Get screen dimensions from the first monitor
    let monitor = {
        let monitors = display.monitors();
        let n = monitors.n_items();
        if n == 0 {
            let _ = result_tx.send(Err(SelectionError::InitError("No monitor found".into())));
            return;
        }
        // Get the first monitor from the list model
        match monitors.item(0) {
            Some(obj) => match obj.downcast::<gdk::Monitor>() {
                Ok(m) => m,
                Err(_) => {
                    let _ = result_tx.send(Err(SelectionError::InitError("Failed to get monitor".into())));
                    return;
                }
            },
            None => {
                let _ = result_tx.send(Err(SelectionError::InitError("No monitor at index 0".into())));
                return;
            }
        }
    };

    let geometry = monitor.geometry();
    let screen_width = geometry.width();
    let screen_height = geometry.height();

    // Create the window
    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(screen_width)
        .default_height(screen_height)
        .decorated(false)
        .resizable(false)
        .css_classes(["overlay", "transparent"])
        .build();

    // Set window to be fullscreen
    window.set_fullscreened(true);

    // Get the surface for cursor control
    let surface = window.surface();

    // Set cursor to crosshair when hovering over the window
    if let Some(ref surface) = surface {
        let cursor = gdk::Cursor::from_name("crosshair", None);
        surface.set_cursor(cursor.as_ref());
    }

    // Create a drawing area for rendering the selection
    let drawing_area = gtk4::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .build();

    let state_draw = state.clone();
    drawing_area.set_draw_func(move |_, context, width, height| {
        draw_overlay(context, width, height, &state_draw);
    });

    // Set the drawing area as the child
    window.set_child(Some(&drawing_area));

    // Setup drag gesture for area selection. It also handles the control
    // panel: once a selection is completed (`completed == true`), a press
    // is remembered as `panel_press` instead of restarting the selection,
    // and the matching release is hit-tested against the panel buttons.
    // Everything funnels through this single gesture deliberately -- a
    // second gesture controller on the same widget risks GTK claiming the
    // press/release sequence for one gesture and never delivering it to
    // the other, which is exactly the "panel doesn't respond" failure mode
    // this is meant to fix.
    let drag_gesture = GestureDrag::builder()
        .propagation_phase(gtk4::PropagationPhase::Capture)
        .build();

    let state_drag = state.clone();
    let window_weak = window.downgrade();
    let drawing_area_weak = drawing_area.downgrade();
    let result_tx_drag = result_tx.clone();

    // Note: connect_drag_begin takes 3 params (gesture, x, y)
    drag_gesture.connect_drag_begin(clone!(
        #[strong]
        state_drag,
        #[strong]
        drawing_area_weak,
        move |_gesture, x, y| {
            let mut st = state_drag.lock();
            let outcome = handle_drag_begin(&mut st, x, y);
            drop(st);

            if matches!(outcome, BeginOutcome::StartedDrag) {
                if let Some(drawing_area) = drawing_area_weak.upgrade() {
                    drawing_area.queue_draw();
                }
            }
        }
    ));

    drag_gesture.connect_drag_update(clone!(
        #[strong]
        state_drag,
        #[strong]
        drawing_area_weak,
        move |_gesture, x, y| {
            let mut st = state_drag.lock();
            let changed = handle_drag_update(&mut st, x, y);
            drop(st);

            if changed {
                if let Some(drawing_area) = drawing_area_weak.upgrade() {
                    drawing_area.queue_draw();
                }
            }
        }
    ));

    drag_gesture.connect_drag_end(clone!(
        #[strong]
        state_drag,
        #[strong]
        window_weak,
        #[strong]
        drawing_area_weak,
        #[strong]
        result_tx_drag,
        move |_gesture, x, y| {
            let mut st = state_drag.lock();
            let outcome = handle_drag_end(&mut st, x, y, primary_monitor_geometry());
            drop(st);

            match outcome {
                EndOutcome::Ignored => {}
                EndOutcome::SelectionCompleted | EndOutcome::PanelDismissed => {
                    if let Some(drawing_area) = drawing_area_weak.upgrade() {
                        drawing_area.queue_draw();
                    }
                }
                EndOutcome::Cancelled => {
                    let _ = result_tx_drag.send(Ok(None));
                    if let Some(window) = window_weak.upgrade() {
                        window.close();
                    }
                }
                EndOutcome::PanelAction(AreaAction::Record, area, (sw, sh)) => {
                    let pick = AreaPick {
                        action: AreaAction::Record,
                        area,
                        screen_width: sw as u32,
                        screen_height: sh as u32,
                    };
                    let _ = result_tx_drag.send(Ok(Some(AreaOutcome::Record(pick))));
                    if let Some(window) = window_weak.upgrade() {
                        window.close();
                    }
                }
                EndOutcome::PanelAction(AreaAction::Capture, area, _) => {
                    // Capture pixels now, synchronously, while this window
                    // is still open and focused. GNOME's Screenshot portal
                    // refuses to show its consent dialog once this process
                    // has no window at all ("Only the focused app is
                    // allowed to show a system access dialog" -- confirmed
                    // via journalctl), which is exactly what happens if
                    // capture waits until after `window.close()` below.
                    let result = capture_area_pixels(area)
                        .map(|data| Some(AreaOutcome::Captured(data)))
                        .map_err(|e| SelectionError::CaptureFailed(e.to_string()));
                    let _ = result_tx_drag.send(result);
                    if let Some(window) = window_weak.upgrade() {
                        window.close();
                    }
                }
            }
        }
    ));

    drawing_area.add_controller(drag_gesture);

    // Setup keyboard controller for ESC key
    let key_controller = EventControllerKey::builder()
        .propagation_phase(gtk4::PropagationPhase::Capture)
        .build();

    let state_key = state.clone();
    let window_weak_esc = window.downgrade();
    let result_tx_esc = result_tx.clone();

    key_controller.connect_key_pressed(clone!(
        #[strong]
        state_key,
        move |_, key, _, _| {
            if key == Key::Escape {
                let mut st = state_key.lock();
                st.cancelled = true;
                drop(st);

                let _ = result_tx_esc.send(Ok(None));

                if let Some(window) = window_weak_esc.upgrade() {
                    window.close();
                }

                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    ));

    drawing_area.add_controller(key_controller);

    // Show the window
    window.present();
}

/// Draw the overlay (darken background + selection rectangle)
fn draw_overlay(
    context: &gtk4::cairo::Context,
    _width: i32,
    _height: i32,
    state: &Arc<Mutex<SelectorState>>,
) {
    let st = state.lock();

    let (screen_width, screen_height) = match primary_monitor_geometry() {
        Some(dims) => dims,
        None => return,
    };

    // Clear to transparent
    context.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    let _ = context.paint();

    if st.is_dragging || st.completed {
        // Calculate selection rectangle
        let x = st.start_x.min(st.current_x);
        let y = st.start_y.min(st.current_y);
        let width = (st.current_x - st.start_x).abs();
        let height = (st.current_y - st.start_y).abs();

        // Darken the area outside the selection
        context.set_source_rgba(0.0, 0.0, 0.0, 0.5);

        // Top rectangle
        context.rectangle(0.0, 0.0, screen_width, y);
        let _ = context.fill();

        // Bottom rectangle
        context.rectangle(0.0, y + height, screen_width, screen_height - y - height);
        let _ = context.fill();

        // Left rectangle
        context.rectangle(0.0, y, x, height);
        let _ = context.fill();

        // Right rectangle
        context.rectangle(x + width, y, screen_width - x - width, height);
        let _ = context.fill();

        // Draw selection border (white)
        context.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        context.set_line_width(2.0);
        context.rectangle(x, y, width, height);
        let _ = context.stroke();

        // Draw dimensions text
        let text = format!("{}×{}", width as i32, height as i32);

        // Set up text rendering
        context.set_font_size(14.0);
        context.set_source_rgba(1.0, 1.0, 1.0, 1.0);

        // Get text extents (call methods with parentheses)
        let extents = context.text_extents(&text).unwrap();

        // Draw text background (semi-transparent black)
        let padding = 8.0;
        let text_x = x + width / 2.0 - extents.width() / 2.0 - extents.x_bearing();
        let text_y = y - 10.0;

        context.set_source_rgba(0.0, 0.0, 0.0, 0.7);
        context.rectangle(
            text_x - padding,
            text_y + extents.y_bearing() - padding,
            extents.width() + padding * 2.0,
            extents.height() + padding * 2.0,
        );
        let _ = context.fill();

        // Draw the text
        context.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        context.move_to(text_x, text_y);
        let _ = context.show_text(&text);

        // Once the drag is done, show the Capture/Record control panel.
        if st.completed {
            let (capture_rect, record_rect) =
                toolbar_layout(x, y, width, height, screen_width, screen_height);
            draw_button(context, &capture_rect, "Capture", None, st.default_action == AreaAction::Capture);
            draw_button(context, &record_rect, "Record", Some((0.90, 0.20, 0.20)), st.default_action == AreaAction::Record);
        }
    } else {
        // Not dragging - darken entire screen slightly
        context.set_source_rgba(0.0, 0.0, 0.0, 0.3);
        let _ = context.paint();
    }
}

impl Default for AreaSelector {
    fn default() -> Self {
        Self::new(AreaAction::default())
    }
}

/// Convenience function to run area selection. `default_action` is
/// highlighted as the primary control-panel button, but the user may still
/// pick either Capture or Record.
pub fn select_area(default_action: AreaAction) -> AreaSelectionResult {
    let selector = AreaSelector::new(default_action);
    selector.run()
}

/// Let the user click on an X11 window to select it for recording.
///
/// Grabs the pointer, waits for a button-press, then returns the geometry of
/// whichever X11 window was directly under the cursor (falling back to the
/// root window). Right-click or middle-click cancels and returns `Ok(None)`.
pub fn select_window() -> Result<Option<SelectionArea>, Box<dyn std::error::Error>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::*;

    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;

    let reply = conn.grab_pointer(
        false,
        root,
        EventMask::BUTTON_PRESS,
        GrabMode::SYNC,
        GrabMode::ASYNC,
        root,
        0u32,  // no cursor override
        0u32,  // CurrentTime
    )?.reply()?;

    if reply.status != GrabStatus::SUCCESS {
        return Err("Failed to grab pointer for window selection".into());
    }

    println!("Click on a window to select it for recording. Right-click to cancel.");

    loop {
        conn.allow_events(Allow::SYNC_POINTER, 0u32)?.ignore_error();
        conn.flush()?;
        let event = conn.wait_for_event()?;

        if let x11rb::protocol::Event::ButtonPress(ev) = event {
            conn.ungrab_pointer(0u32)?.ignore_error();
            conn.flush()?;

            if ev.detail != 1 {
                return Ok(None); // right-click or middle-click = cancel
            }

            // Use the child window directly under the click, fall back to root
            let target = if ev.child != 0 { ev.child } else { root };

            let geom = conn.get_geometry(target)?.reply()?;
            let trans = conn.translate_coordinates(target, root, 0, 0)?.reply()?;

            return Ok(Some(SelectionArea {
                x: trans.dst_x as i32,
                y: trans.dst_y as i32,
                width: geom.width as i32,
                height: geom.height as i32,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selection_normalize() {
        // Normal case (no normalization needed)
        let area = SelectionArea { x: 100, y: 100, width: 200, height: 150 };
        let normalized = area.normalize();
        assert_eq!(normalized.x, 100);
        assert_eq!(normalized.y, 100);
        assert_eq!(normalized.width, 200);
        assert_eq!(normalized.height, 150);

        // Negative width (dragged left)
        let area = SelectionArea { x: 300, y: 100, width: -200, height: 150 };
        let normalized = area.normalize();
        assert_eq!(normalized.x, 100);
        assert_eq!(normalized.y, 100);
        assert_eq!(normalized.width, 200);
        assert_eq!(normalized.height, 150);

        // Negative height (dragged up)
        let area = SelectionArea { x: 100, y: 250, width: 200, height: -150 };
        let normalized = area.normalize();
        assert_eq!(normalized.x, 100);
        assert_eq!(normalized.y, 100);
        assert_eq!(normalized.width, 200);
        assert_eq!(normalized.height, 150);

        // Both negative (dragged up-left)
        let area = SelectionArea { x: 300, y: 250, width: -200, height: -150 };
        let normalized = area.normalize();
        assert_eq!(normalized.x, 100);
        assert_eq!(normalized.y, 100);
        assert_eq!(normalized.width, 200);
        assert_eq!(normalized.height, 150);
    }

    #[test]
    fn test_selection_is_valid() {
        // Valid selection
        let area = SelectionArea { x: 100, y: 100, width: 200, height: 150 };
        assert!(area.is_valid());

        // Zero width
        let area = SelectionArea { x: 100, y: 100, width: 0, height: 150 };
        assert!(!area.is_valid());

        // Zero height
        let area = SelectionArea { x: 100, y: 100, width: 200, height: 0 };
        assert!(!area.is_valid());

        // Negative (before normalization)
        let area = SelectionArea { x: 100, y: 100, width: -200, height: 150 };
        assert!(!area.is_valid());
    }

    #[test]
    fn test_button_rect_contains() {
        let rect = ButtonRect { x: 100.0, y: 200.0, w: 120.0, h: 40.0 };
        // Inside, including edges
        assert!(rect.contains(100.0, 200.0));
        assert!(rect.contains(220.0, 240.0));
        assert!(rect.contains(160.0, 220.0));
        // Outside
        assert!(!rect.contains(99.0, 220.0));
        assert!(!rect.contains(221.0, 220.0));
        assert!(!rect.contains(160.0, 199.0));
        assert!(!rect.contains(160.0, 241.0));
    }

    #[test]
    fn test_toolbar_layout_below_selection_when_room() {
        let (capture_rect, record_rect) = toolbar_layout(500.0, 500.0, 300.0, 200.0, 1920.0, 1080.0);
        // Panel sits below the selection (start_y > selection bottom)
        assert!(capture_rect.y > 500.0 + 200.0);
        assert_eq!(capture_rect.y, record_rect.y);
        // Record button sits to the right of Capture with no overlap
        assert!(record_rect.x >= capture_rect.x + capture_rect.w);
        // Panel centered under the selection
        let panel_center = capture_rect.x + (record_rect.x + record_rect.w - capture_rect.x) / 2.0;
        assert!((panel_center - (500.0 + 300.0 / 2.0)).abs() < 1.0);
    }

    #[test]
    fn test_toolbar_layout_flips_above_when_no_room_below() {
        // Selection hugging the bottom edge of the screen
        let (capture_rect, _) = toolbar_layout(500.0, 1000.0, 300.0, 70.0, 1920.0, 1080.0);
        // Panel must not run past the bottom of the screen
        assert!(capture_rect.y + capture_rect.h <= 1080.0);
        // And must sit above the selection's top edge
        assert!(capture_rect.y + capture_rect.h <= 1000.0);
    }

    #[test]
    fn test_toolbar_layout_clamped_within_screen_horizontally() {
        // Selection hugging the left edge: panel must not go off-screen to the left
        let (capture_rect, record_rect) = toolbar_layout(0.0, 500.0, 20.0, 20.0, 1920.0, 1080.0);
        assert!(capture_rect.x >= 0.0);
        assert!(record_rect.x + record_rect.w <= 1920.0);

        // Selection hugging the right edge: panel must not go off-screen to the right
        let (capture_rect, record_rect) = toolbar_layout(1900.0, 500.0, 20.0, 20.0, 1920.0, 1080.0);
        assert!(capture_rect.x >= 0.0);
        assert!(record_rect.x + record_rect.w <= 1920.0);
    }

    #[test]
    fn test_area_action_default_is_capture() {
        assert_eq!(AreaAction::default(), AreaAction::Capture);
    }

    /// End-to-end simulation of the exact reported bug: drag out a
    /// selection, then click a control-panel button. Before this fix, any
    /// press after the drag finished was misread as the start of a brand
    /// new drag (the selection kept being redrawn and the panel button was
    /// never actually hit). This drives the real gesture handlers used by
    /// `setup_window`, just without any GTK/X11 involved.
    #[test]
    fn test_drag_then_panel_click_selects_record() {
        let mut state = SelectorState::default();
        let screen = Some((1920.0, 1080.0));

        // 1. Drag out a selection from (100,100) to (400,300).
        assert_eq!(handle_drag_begin(&mut state, 100.0, 100.0), BeginOutcome::StartedDrag);
        assert!(state.is_dragging);
        assert!(handle_drag_update(&mut state, 300.0, 200.0));
        assert_eq!(
            handle_drag_end(&mut state, 300.0, 200.0, screen),
            EndOutcome::SelectionCompleted
        );
        assert!(state.completed);
        assert!(state.panel_press.is_none());

        // 2. Press down on the Record button drawn for this selection.
        let (capture_rect, record_rect) = toolbar_layout(100.0, 100.0, 300.0, 200.0, 1920.0, 1080.0);
        let record_x = record_rect.x + record_rect.w / 2.0;
        let record_y = record_rect.y + record_rect.h / 2.0;
        assert_eq!(
            handle_drag_begin(&mut state, record_x, record_y),
            BeginOutcome::CapturedPanelPress
        );
        // Critical: the press over the panel must NOT restart or move the
        // already-drawn selection rectangle.
        assert_eq!(state.start_x, 100.0);
        assert_eq!(state.start_y, 100.0);
        assert_eq!(state.current_x, 400.0);
        assert_eq!(state.current_y, 300.0);
        assert!(!state.is_dragging);

        // 3. Release on the same spot: must resolve to Record on the
        // original selection, not a fresh/corrupted one.
        let outcome = handle_drag_end(&mut state, 0.0, 0.0, screen);
        assert_eq!(
            outcome,
            EndOutcome::PanelAction(
                AreaAction::Record,
                SelectionArea { x: 100, y: 100, width: 300, height: 200 },
                (1920.0, 1080.0)
            )
        );
        // Sanity: the two buttons are distinct rects (Record isn't Capture).
        assert_ne!(capture_rect.x, record_rect.x);
    }

    #[test]
    fn test_drag_then_panel_click_selects_capture() {
        let mut state = SelectorState::default();
        let screen = Some((1920.0, 1080.0));

        handle_drag_begin(&mut state, 100.0, 100.0);
        handle_drag_update(&mut state, 300.0, 200.0);
        handle_drag_end(&mut state, 300.0, 200.0, screen);

        let (capture_rect, _) = toolbar_layout(100.0, 100.0, 300.0, 200.0, 1920.0, 1080.0);
        let cx = capture_rect.x + capture_rect.w / 2.0;
        let cy = capture_rect.y + capture_rect.h / 2.0;
        handle_drag_begin(&mut state, cx, cy);
        let outcome = handle_drag_end(&mut state, 0.0, 0.0, screen);
        assert_eq!(
            outcome,
            EndOutcome::PanelAction(
                AreaAction::Capture,
                SelectionArea { x: 100, y: 100, width: 300, height: 200 },
                (1920.0, 1080.0)
            )
        );
    }

    #[test]
    fn test_click_outside_panel_discards_selection_and_allows_redraw() {
        let mut state = SelectorState::default();
        let screen = Some((1920.0, 1080.0));

        handle_drag_begin(&mut state, 100.0, 100.0);
        handle_drag_update(&mut state, 300.0, 200.0);
        handle_drag_end(&mut state, 300.0, 200.0, screen);
        assert!(state.completed);

        // Press and release far outside both buttons.
        handle_drag_begin(&mut state, 10.0, 10.0);
        assert_eq!(handle_drag_end(&mut state, 0.0, 0.0, screen), EndOutcome::PanelDismissed);
        assert!(!state.completed);

        // The user must be able to drag a brand new selection afterward.
        assert_eq!(handle_drag_begin(&mut state, 50.0, 60.0), BeginOutcome::StartedDrag);
        assert_eq!(state.start_x, 50.0);
        assert_eq!(state.start_y, 60.0);
        assert!(state.is_dragging);
    }

    #[test]
    fn test_zero_area_drag_is_cancelled_not_completed() {
        let mut state = SelectorState::default();
        handle_drag_begin(&mut state, 100.0, 100.0);
        // No movement at all.
        assert_eq!(handle_drag_end(&mut state, 0.0, 0.0, None), EndOutcome::Cancelled);
        assert!(!state.completed);
    }

    #[test]
    fn test_drag_update_ignored_once_panel_is_showing() {
        let mut state = SelectorState::default();
        let screen = Some((1920.0, 1080.0));
        handle_drag_begin(&mut state, 100.0, 100.0);
        handle_drag_update(&mut state, 300.0, 200.0);
        handle_drag_end(&mut state, 300.0, 200.0, screen);
        assert!(state.completed);

        // A stray drag_update after completion (e.g. a slightly-moving
        // press over the panel) must never touch the frozen selection.
        assert!(!handle_drag_update(&mut state, 999.0, 999.0));
        assert_eq!(state.current_x, 400.0);
        assert_eq!(state.current_y, 300.0);
    }

    /// Mirrors the `AreaPick` construction done in `setup_window`'s GTK
    /// closure (which isn't itself unit-testable without a live GTK/GDK
    /// display): given the `EndOutcome::PanelAction` 3-tuple produced by
    /// the pure drag handlers, confirm the screen dimensions survive the
    /// `f64 -> u32` conversion correctly, truncation included.
    #[test]
    fn test_area_pick_screen_dims_from_panel_action() {
        let mut state = SelectorState::default();
        let screen = Some((1920.7, 1080.4));

        handle_drag_begin(&mut state, 100.0, 100.0);
        handle_drag_update(&mut state, 300.0, 200.0);
        handle_drag_end(&mut state, 300.0, 200.0, screen);

        let (capture_rect, _) = toolbar_layout(100.0, 100.0, 300.0, 200.0, 1920.7, 1080.4);
        let cx = capture_rect.x + capture_rect.w / 2.0;
        let cy = capture_rect.y + capture_rect.h / 2.0;
        handle_drag_begin(&mut state, cx, cy);
        let outcome = handle_drag_end(&mut state, 0.0, 0.0, screen);

        let EndOutcome::PanelAction(action, area, (screen_w, screen_h)) = outcome else {
            panic!("expected PanelAction, got {outcome:?}");
        };
        let pick = AreaPick {
            action,
            area,
            screen_width: screen_w as u32,
            screen_height: screen_h as u32,
        };
        assert_eq!(pick.action, AreaAction::Capture);
        assert_eq!(pick.area, SelectionArea { x: 100, y: 100, width: 300, height: 200 });
        assert_eq!(pick.screen_width, 1920);
        assert_eq!(pick.screen_height, 1080);
    }

    #[test]
    fn test_overlay_lock_single_instance() {
        let first = OverlayLock::try_acquire().expect("first acquire should not error");
        assert!(first.is_some(), "first acquire should succeed when nothing else holds the lock");

        let second = OverlayLock::try_acquire().expect("second acquire should not error");
        assert!(second.is_none(), "a concurrent acquire must fail while the first guard is still held");

        drop(first);

        let third = OverlayLock::try_acquire().expect("acquire after release should not error");
        assert!(third.is_some(), "acquire must succeed again once the prior guard is dropped");
    }
}
