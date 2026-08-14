use std::path::PathBuf;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use zbus::zvariant::OwnedValue;
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;

pub fn generate_recording_filename(output_dir: &str, prefix: &str, extension: &str) -> PathBuf {
    let dir = std::path::PathBuf::from(shellexpand::tilde(output_dir).as_ref());
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    dir.join(format!("{prefix}_{timestamp}.{extension}"))
}

/// Path to the recording lock file: only one recording (`screen`/`area`/
/// `window`, MP4/WebM or GIF) may run at a time. Unlike
/// `overlay::overlay_lock_path`'s pure mutual-exclusion lock, this file's
/// content is the holder's PID, so a second `record` invocation -- the
/// common case being the *same* hotkey pressed again -- can find and
/// gracefully stop it instead of either stacking a second recording or
/// silently doing nothing.
fn recording_lock_path() -> std::path::PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("openshotx-recording.lock")
}

/// Holds the exclusive `flock` on the recording lock file for the whole
/// recording's lifetime. Same auto-release-on-crash guarantee as
/// `overlay::OverlayLock`: the kernel drops the lock the moment this
/// process's fd table is torn down, `kill -9` included, so a stuck lock
/// can never require manual cleanup.
pub struct RecordingLock {
    _file: std::fs::File,
}

enum RecordingLockResult {
    /// No other recording is running; caller now holds the lock.
    Acquired(RecordingLock),
    /// Another recording already holds the lock, running as `pid` (`0` if
    /// its PID couldn't be read back from the lock file).
    AlreadyRunning(libc::pid_t),
}

impl RecordingLock {
    fn try_acquire() -> std::io::Result<RecordingLockResult> {
        use std::io::{Read, Seek, SeekFrom, Write};
        use std::os::unix::io::AsRawFd;

        let path = recording_lock_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new().create(true).read(true).write(true).open(&path)?;

        // SAFETY: flock is a simple advisory-lock syscall on a valid fd we
        // just opened ourselves; no aliasing or lifetime hazards.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == 0 {
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            write!(file, "{}", std::process::id())?;
            file.flush()?;
            Ok(RecordingLockResult::Acquired(Self { _file: file }))
        } else {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                let mut contents = String::new();
                let _ = file.read_to_string(&mut contents);
                let pid: libc::pid_t = contents.trim().parse().unwrap_or(0);
                Ok(RecordingLockResult::AlreadyRunning(pid))
            } else {
                Err(err)
            }
        }
    }
}

/// Outcome of [`acquire_or_signal_stop`].
pub enum StartRecordingLock {
    /// No other recording was running (or the lock file itself was
    /// unavailable): proceed. Carries the held lock when there is one to
    /// hold onto for the recording's lifetime; `None` only in the
    /// best-effort I/O-failure case, where recording proceeds unprotected.
    Proceed(Option<RecordingLock>),
    /// Another recording was already running and has now been sent
    /// `SIGINT` to stop it gracefully. The caller must not start a new
    /// recording.
    SignaledStop,
}

/// Acquire the recording lock, or -- if a recording is already running --
/// send it `SIGINT` (the same graceful-stop signal the HUD's Stop button
/// and the tray's "Stop Recording" use) so the caller starts nothing new.
/// This is what gives a `record` hotkey "press to start, press again to
/// stop" behavior: without it, a second press just launched an unrelated
/// overlapping recording, and the first one had no reachable way to stop
/// short of a hard kill (which leaves an unplayable file with no `moov`
/// atom ever written).
///
/// On lock-file I/O failure the recording proceeds anyway without
/// toggle-stop protection (best-effort; matches `OverlayLock`'s posture of
/// never blocking the feature over an advisory lock).
pub fn acquire_or_signal_stop() -> StartRecordingLock {
    match RecordingLock::try_acquire() {
        Ok(RecordingLockResult::Acquired(lock)) => StartRecordingLock::Proceed(Some(lock)),
        Ok(RecordingLockResult::AlreadyRunning(pid)) if pid > 0 => {
            println!("A recording is already in progress -- stopping it.");
            // SAFETY: sending a signal to a pid we read from our own lock file.
            unsafe { libc::kill(pid, libc::SIGINT) };
            StartRecordingLock::SignaledStop
        }
        Ok(RecordingLockResult::AlreadyRunning(_)) => {
            eprintln!("Error: a recording is already in progress (could not read its PID to stop it).");
            StartRecordingLock::SignaledStop
        }
        Err(e) => {
            eprintln!("Warning: recording lock unavailable ({}); proceeding without toggle-stop.", e);
            StartRecordingLock::Proceed(None)
        }
    }
}

#[derive(Debug, Error)]
pub enum RecordError {
    #[error("GStreamer initialization failed: {0}")]
    InitError(String),
    
    #[error("GStreamer error: {0}")]
    GStreamerError(String),
    
    #[error("Wayland portal error: {0}")]
    PortalError(String),
    
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("Unsupported backend: {0}")]
    UnsupportedBackend(String),

    #[error("Cancelled by user")]
    Cancelled,
    
    #[error("No suitable video encoder found. Please install gst-plugins-good/ugly/bad.")]
    NoEncoderFound,
    
    #[error("GIF encoding error: {0}")]
    GifError(String),
}

pub type RecordResult<T> = Result<T, RecordError>;

#[derive(Debug, Clone)]
pub struct RecordingConfig {
    pub output_path: PathBuf,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub screen_width: Option<u32>,
    pub screen_height: Option<u32>,
    pub highlight_cursor: bool,
    pub highlight_color: String,
    pub highlight_radius: u32,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        let mut path = dirs::video_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("output.mp4");
        Self {
            output_path: path,
            width: None,
            height: None,
            x: None,
            y: None,
            screen_width: None,
            screen_height: None,
            highlight_cursor: false,
            highlight_color: "#FFFF00".to_string(),
            highlight_radius: 30,
        }
    }
}

struct EncoderProfile {
    name: &'static str,
    encoder: &'static str,
    props: &'static str,
    muxer: &'static str,
    extension: &'static str,
}

// Priority list of encoders
const PROFILES: &[EncoderProfile] = &[
    // VP8 (WebM) - Prioritized fallback over H.264 if missing, and better than Theora
    EncoderProfile {
        name: "VP8", 
        encoder: "vp8enc", 
        props: "deadline=1", 
        muxer: "webmmux", 
        extension: "webm"
    }, 
    // VP9 (WebM)
    EncoderProfile {
        name: "VP9", 
        encoder: "vp9enc", 
        props: "deadline=1", 
        muxer: "webmmux", 
        extension: "webm"
    },
    // Standard H.264
    EncoderProfile {
        name: "H.264 (x264)", 
        encoder: "x264enc", 
        props: "speed-preset=ultrafast tune=zerolatency", 
        muxer: "mp4mux", 
        extension: "mp4"
    },
    // Cisco OpenH264 — needs h264parse to negotiate caps with mp4mux
    EncoderProfile {
        name: "H.264 (OpenH264)",
        encoder: "openh264enc",
        props: "! h264parse",
        muxer: "mp4mux",
        extension: "mp4"
    },
    // Theora (Ogg) - Last resort
    EncoderProfile {
        name: "Theora", 
        encoder: "theoraenc", 
        props: "", 
        muxer: "oggmux", 
        extension: "ogv"
    },
];

/// Start a recording session
pub async fn start_recording(config: RecordingConfig) -> RecordResult<PathBuf> {
    // 1. Initialize GStreamer
    gst::init().map_err(|e| RecordError::InitError(e.to_string()))?;

    // Check if GIF requested
    if config.output_path.extension().map_or(false, |e| e == "gif") {
        return record_gif_rust(config).await;
    }

    // 2. Select Encoder Profile
    let (profile, final_path) = select_encoder(&config.output_path)?;
    println!("Using Encoder: {} ({})", profile.name, profile.encoder);
    
    if final_path != config.output_path {
        println!("Note: Output filename changed to match format: {:?}", final_path);
    }

    // 3. Build pipeline description
    let pipeline_str = build_pipeline(&config, profile, &final_path).await?;
    println!("Starting recording to: {:?}", final_path);

    // 4. Create pipeline
    let pipeline = gst::parse::launch(&pipeline_str)
        .map_err(|e| RecordError::GStreamerError(format!("Failed to parse pipeline: {}", e)))
        ?.downcast::<gst::Pipeline>()
        .map_err(|_| RecordError::GStreamerError("Cast to Pipeline failed".into()))?;

    // 4b. Setup cursor overlay if requested
    if config.highlight_cursor {
        setup_cursor_overlay(&pipeline, &config);
    }

    // 5. Start playing
    if let Err(err) = pipeline.set_state(gst::State::Playing) {
        eprintln!("Failed to set pipeline to Playing: {}", err);
        if let Some(bus) = pipeline.bus() {
            while let Some(msg) = bus.pop() {
                if let gst::MessageView::Error(err) = msg.view() {
                    eprintln!("Detailed Error from {}: {}", 
                        err.src().map(|s| s.name()).unwrap_or("unknown".into()), 
                        err.error()
                    );
                    if let Some(debug) = err.debug() {
                        eprintln!("Debug Info: {}", debug);
                    }
                }
            }
        }
        let _ = pipeline.set_state(gst::State::Null);
        return Err(RecordError::GStreamerError(format!("State change failed: {}", err)));
    }

    // 6. Show the recording HUD (live timer + Pause/Resume/Stop), falling
    // back to a plain terminal Ctrl+C loop if it can't initialize (e.g. no
    // display available). Either way, once this resolves the pipeline has
    // already reached EOS (or the wait for it timed out) and is ready for
    // the Null cleanup below.
    let watch_result = match crate::recording_hud::run(pipeline.clone()) {
        Ok(()) => Ok(()),
        Err(RecordError::InitError(e)) => {
            eprintln!("Recording HUD unavailable ({}); falling back to terminal mode.", e);
            watch_pipeline_via_ctrl_c(&pipeline).await
        }
        Err(e) => Err(e),
    };

    // 7. Cleanup: always attempt this regardless of how watching ended, so
    // a pipeline error still gets a best-effort finalize before the error
    // propagates (matches the original single-loop behavior).
    let null_result = pipeline
        .set_state(gst::State::Null)
        .map_err(|e| RecordError::GStreamerError(format!("Failed to set state to Null: {}", e)));

    watch_result?;
    null_result?;

    println!("Recording saved to {:?}", final_path);
    if let Ok(metadata) = std::fs::metadata(&final_path) {
        println!("File size: {:.2} MB", metadata.len() as f64 / 1024.0 / 1024.0);
    }
    
    Ok(final_path)
}

/// Watch `pipeline`'s bus until Ctrl+C or a pipeline error, then wait (up
/// to 5s) for EOS after sending it. The fallback recording-progress loop
/// when the HUD can't be shown; on `Ok(())`, `pipeline` has reached EOS (or
/// the wait for it timed out) and is ready for the caller's own
/// `set_state(Null)`.
async fn watch_pipeline_via_ctrl_c(pipeline: &gst::Pipeline) -> RecordResult<()> {
    let bus = pipeline.bus().ok_or_else(|| RecordError::GStreamerError("Pipeline has no bus".into()))?;

    println!("Recording... Press Ctrl+C to stop.");

    // Handle Ctrl+C
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    // Phase 1: Recording until Ctrl+C or Error
    let mut stopping = false;
    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                println!("\nStopping recording... Finalizing file...");
                pipeline.send_event(gst::event::Eos::new());
                stopping = true;
                break;
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                // Poll bus
                for msg in bus.iter_timed(gst::ClockTime::ZERO) {
                    use gst::MessageView;
                    match msg.view() {
                        MessageView::Eos(..) => {
                            println!("End of stream reached (unexpected).");
                            stopping = true;
                            break;
                        }
                        MessageView::Error(err) => {
                            eprintln!("Error from element {:?}: {}", err.src().map(|s| s.name()), err.error());
                            return Err(RecordError::GStreamerError(err.error().to_string()));
                        }
                        _ => (),
                    }
                }
                if stopping { break; }
            }
        }
    }

    // Phase 2: Wait for EOS if we initiated stop
    if stopping {
        let start_wait = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(5); // 5s timeout for finalization
        
        loop {
            if start_wait.elapsed() > timeout {
                eprintln!("Timeout waiting for EOS. Forcing stop.");
                break;
            }

            // Check bus
            let mut eos_received = false;
            for msg in bus.iter_timed(gst::ClockTime::from_mseconds(100)) {
                use gst::MessageView;
                match msg.view() {
                    MessageView::Eos(..) => {
                        println!("File finalized successfully.");
                        eos_received = true;
                        break;
                    }
                    MessageView::Error(err) => {
                        eprintln!("Error during finalization: {}", err.error());
                        eos_received = true; // Stop waiting
                        break;
                    }
                    _ => (),
                }
            }
            if eos_received { break; }
        }
    }

    Ok(())
}

pub fn copy_to_clipboard(path: &PathBuf) -> RecordResult<()> {
    use std::process::{Command, Stdio};
    use std::io::Write;
    
    println!("Copying to clipboard...");
    
    // Convert path to file:// URI for better compatibility with chat apps (Discord, Slack, etc.)
    // They often fail to handle raw image/gif bytes but handle text/uri-list correctly.
    let uri = url::Url::from_file_path(path)
        .map_err(|_| RecordError::GStreamerError("Failed to convert path to URI".into()))?
        .to_string();

    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        // Wayland: use wl-copy with text/uri-list
        let mut child = Command::new("wl-copy")
            .arg("--type")
            .arg("text/uri-list")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|_| RecordError::IoError(std::io::Error::new(std::io::ErrorKind::NotFound, "wl-copy not found. Install wl-clipboard.")))?;
            
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(uri.as_bytes())?;
        }
        
        let status = child.wait()?;
        if !status.success() {
            return Err(RecordError::GStreamerError("wl-copy failed".into()));
        }
    } else {
        // X11: use xclip with text/uri-list
        let mut child = Command::new("xclip")
            .arg("-selection")
            .arg("clipboard")
            .arg("-t")
            .arg("text/uri-list")
            .arg("-i")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|_| RecordError::IoError(std::io::Error::new(std::io::ErrorKind::NotFound, "xclip not found. Install xclip.")))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(uri.as_bytes())?;
        }

        let status = child.wait()?;
        if !status.success() {
             return Err(RecordError::GStreamerError("xclip failed".into()));
        }
    }
    
    println!("Copied GIF URI to clipboard!");
    Ok(())
}

fn select_encoder(requested_path: &PathBuf) -> RecordResult<(&'static EncoderProfile, PathBuf)> {
    // Check for x264enc first to warn user if missing
    if gst::ElementFactory::find("x264enc").is_none() {
        println!("\n\x1b[33mWARNING: H.264 encoder (x264enc) not found!\x1b[0m");
        println!("Falling back to inferior encoders (Theora/VP8). For high-quality MP4 recording, please install:");
        println!("  Ubuntu/Debian: \x1b[1msudo apt install gstreamer1.0-plugins-ugly\x1b[0m");
        println!("  Arch:          \x1b[1msudo pacman -S gst-plugins-ugly\x1b[0m");
        println!("  Fedora:        \x1b[1msudo dnf install gstreamer1-plugins-ugly-free\x1b[0m\n");
    }

    if let Some(ext) = requested_path.extension().and_then(|s| s.to_str()) {
        for profile in PROFILES {
            if profile.extension == ext {
                if gst::ElementFactory::find(profile.encoder).is_some() && 
                   gst::ElementFactory::find(profile.muxer).is_some() {
                    return Ok((profile, requested_path.clone()));
                }
            }
        }
        println!("Warning: Requested format '{}' not supported or encoder missing.", ext);
    }

    for profile in PROFILES {
        if gst::ElementFactory::find(profile.encoder).is_some() && 
           gst::ElementFactory::find(profile.muxer).is_some() {
            let mut new_path = requested_path.clone();
            new_path.set_extension(profile.extension);
            return Ok((profile, new_path));
        }
    }

    Err(RecordError::NoEncoderFound)
}

async fn build_pipeline(config: &RecordingConfig, profile: &EncoderProfile, output_path: &PathBuf) -> RecordResult<String> {
    let output_str = output_path.to_string_lossy();

    // Get video source
    let video_source = if std::env::var("WAYLAND_DISPLAY").is_ok() {
        get_wayland_source(config).await?
    } else {
        get_x11_source(config)?
    };

    if config.highlight_cursor {
        Ok(format!(
            "{} ! videoconvert ! cairooverlay name=co ! videoconvert ! videorate ! queue ! videoconvert ! {} {} ! {} ! filesink location=\"{}\"",
            video_source,
            profile.encoder, profile.props, profile.muxer, output_str
        ))
    } else {
        Ok(format!(
            "{} ! videoconvert ! videorate ! queue ! videoconvert ! {} {} ! {} ! filesink location=\"{}\"",
            video_source,
            profile.encoder, profile.props, profile.muxer, output_str
        ))
    }
}

fn screencast_token_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
        .join("openshotx")
        .join("screencast_token")
}

pub fn load_screencast_token() -> Option<String> {
    std::fs::read_to_string(screencast_token_path()).ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn save_screencast_token(token: &str) {
    let path = screencast_token_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, token);
}

/// Build the `videocrop` GStreamer stage that crops a PipeWire monitor
/// stream down to the selected region, given the selection rect
/// `(x, y, width, height)` and the full monitor's `(screen_width,
/// screen_height)`. Margins are clamped to `0` (never negative) via i64
/// arithmetic so a region flush against the screen edge doesn't underflow.
/// Returns the `" ! videocrop ..."` suffix to append to the base source.
fn videocrop_suffix(x: i32, y: i32, width: u32, height: u32, screen_width: u32, screen_height: u32) -> String {
    let left = x.max(0);
    let top = y.max(0);
    let right = ((screen_width as i64) - (x as i64) - (width as i64)).max(0);
    let bottom = ((screen_height as i64) - (y as i64) - (height as i64)).max(0);

    format!(" ! videocrop left={} top={} right={} bottom={}", left, top, right, bottom)
}

async fn get_wayland_source(config: &RecordingConfig) -> RecordResult<String> {
    use ashpd::desktop::screencast::Screencast;
    use zbus::zvariant::Value;

    println!("Requesting Wayland ScreenCast session...");

    let restore_token = load_screencast_token();

    let proxy = Screencast::new().await
        .map_err(|e| RecordError::PortalError(e.to_string()))?;

    let session = proxy.create_session().await
        .map_err(|e| RecordError::PortalError(e.to_string()))?;

    let connection = proxy.connection();

    // 1. Select Sources — Persistent mode so the compositor remembers the choice
    proxy.select_sources(
        &session,
        ashpd::desktop::screencast::CursorMode::Embedded,
        ashpd::desktop::screencast::SourceType::Monitor | ashpd::desktop::screencast::SourceType::Window,
        false, // multiple
        restore_token.as_deref(),
        ashpd::desktop::PersistMode::ExplicitlyRevoked,
    ).await.map_err(|e| RecordError::PortalError(e.to_string()))?;

    if restore_token.is_none() {
        println!("Please select a screen or window to record...");
    }

    // 2. Start
    let msg = connection.call_method(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        Some("org.freedesktop.portal.ScreenCast"),
        "Start",
        &(&session, "", HashMap::<String, Value>::new()),
    ).await.map_err(|e| RecordError::PortalError(format!("Start call failed: {}", e)))?;

    let request_path: zbus::zvariant::OwnedObjectPath = msg.body().deserialize()
        .map_err(|e| RecordError::PortalError(format!("Failed to parse Start response: {}", e)))?;

    let results: HashMap<String, OwnedValue> = wait_for_response(connection, &request_path).await?;

    // Save restore token if the portal returned one
    if let Some(token_value) = results.get("restore_token") {
        if let Ok(token) = String::try_from(token_value.try_clone().unwrap()) {
            if !token.is_empty() {
                save_screencast_token(&token);
            }
        }
    }

    let streams_value = results.get("streams")
        .ok_or_else(|| RecordError::PortalError("No streams in portal response".into()))?;

    let streams: Vec<(u32, HashMap<String, OwnedValue>)> = streams_value.try_clone().unwrap()
        .try_into()
        .map_err(|e| RecordError::PortalError(format!("Invalid streams format: {}", e)))?;

    let stream = streams.first()
        .ok_or_else(|| RecordError::PortalError("No streams returned".into()))?;

    let node_id = stream.0;
    println!("Got PipeWire Node ID: {}", node_id);

    let base_source = format!("pipewiresrc path={} do-timestamp=true", node_id);
    Ok(apply_wayland_crop(base_source, config))
}

/// Append the `videocrop` suffix to `base_source` when a full crop rect
/// (`x`/`y`/`width`/`height`) and screen size (`screen_width`/
/// `screen_height`) are all known; otherwise return `base_source`
/// unchanged -- today's whole-monitor/whole-window behavior for `record
/// screen`/`record window`, which must not regress.
fn apply_wayland_crop(base_source: String, config: &RecordingConfig) -> String {
    match (config.x, config.y, config.width, config.height, config.screen_width, config.screen_height) {
        (Some(x), Some(y), Some(width), Some(height), Some(screen_width), Some(screen_height)) => {
            format!("{}{}", base_source, videocrop_suffix(x, y, width, height, screen_width, screen_height))
        }
        _ => base_source,
    }
}

async fn wait_for_response(
    connection: &zbus::Connection, 
    path: &zbus::zvariant::ObjectPath<'_>
) -> RecordResult<HashMap<String, OwnedValue>> {
    use futures_util::StreamExt;
    
    let match_rule = format!(
        "type='signal',interface='org.freedesktop.portal.Request',member='Response',path='{}'",
        path
    );
    
    let rule: zbus::MatchRule = match_rule.as_str().try_into()
        .map_err(|e| RecordError::PortalError(format!("Invalid match rule: {}", e)))?;

    let mut stream = zbus::MessageStream::for_match_rule(
        rule,
        connection,
        Some(1),
    ).await.map_err(|e| RecordError::PortalError(format!("Failed to create message stream: {}", e)))?;

    let message = stream.next().await
        .ok_or_else(|| RecordError::PortalError("No response from portal".into()))?
        .map_err(|e| RecordError::PortalError(format!("Signal error: {}", e)))?;

    // Response signal signature: (ua{sv})
    let (status, results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize()
        .map_err(|e| RecordError::PortalError(format!("Failed to deserialize portal response: {}", e)))?;

    if status != 0 {
        return Err(RecordError::Cancelled);
    }
    
    Ok(results)
}

fn setup_cursor_overlay(pipeline: &gst::Pipeline, config: &RecordingConfig) {
    let overlay = match pipeline.by_name("co") {
        Some(e) => e,
        None => return,
    };

    let cursor_pos = Arc::new(Mutex::new((0.0f64, 0.0f64)));
    let tracker = cursor_pos.clone();

    std::thread::spawn(move || {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::ConnectionExt as _;
        if let Ok((conn, screen_num)) = x11rb::connect(None) {
            let root = conn.setup().roots[screen_num].root;
            loop {
                if let Ok(cookie) = conn.query_pointer(root) {
                    if let Ok(reply) = cookie.reply() {
                        if let Ok(mut p) = tracker.lock() {
                            *p = (reply.root_x as f64, reply.root_y as f64);
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        }
    });

    let color_hex = config.highlight_color.trim_start_matches('#').to_string();
    let radius = config.highlight_radius as f64;

    overlay.connect("draw", false, move |values| {
        // Extract the cairo_t* from the GLib value using raw pointer access.
        // cairo-rs 0.20 uses glib 0.20 while gstreamer 0.24 uses glib 0.21,
        // so we bypass the type system by reading the boxed pointer directly.
        let cr: cairo::Context = unsafe {
            use gst::glib::translate::ToGlibPtr;
            let gvalue: &gst::glib::Value = &values[1];
            let raw_gvalue: *const gst::glib::gobject_ffi::GValue = gvalue.to_glib_none().0;
            let ptr = gst::glib::gobject_ffi::g_value_get_boxed(raw_gvalue);
            if ptr.is_null() {
                return None;
            }
            cairo::Context::from_raw_none(ptr as *mut cairo::ffi::cairo_t)
        };

        let (x, y) = cursor_pos.lock().map(|p| *p).unwrap_or((0.0, 0.0));
        if x > 0.0 || y > 0.0 {
            let r = u8::from_str_radix(&color_hex[0..2], 16).unwrap_or(255) as f64 / 255.0;
            let g = u8::from_str_radix(&color_hex[2..4], 16).unwrap_or(255) as f64 / 255.0;
            let b = u8::from_str_radix(&color_hex[4..6], 16).unwrap_or(0) as f64 / 255.0;
            cr.arc(x, y, radius, 0.0, 2.0 * std::f64::consts::PI);
            cr.set_source_rgba(r, g, b, 0.4);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(r, g, b, 1.0);
            cr.set_line_width(3.0);
            let _ = cr.stroke();
        }
        None
    });
}

fn get_x11_source(config: &RecordingConfig) -> RecordResult<String> {
    let mut source = String::from("ximagesrc show-pointer=true use-damage=false");
    
    if let (Some(x), Some(y), Some(w), Some(h)) = (config.x, config.y, config.width, config.height) {
        source.push_str(&format!(" startx={} starty={} endx={} endy={}", x, y, x + w as i32 - 1, y + h as i32 - 1));
    }

    Ok(source)
}

async fn record_gif_rust(config: RecordingConfig) -> RecordResult<PathBuf> {
    use std::process::{Command, Stdio};
    use std::io::Write;
    
    println!("Starting GIF recording (via FFmpeg Pipe)...");
    
    // Check if ffmpeg is available
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("Error: ffmpeg not found!");
        eprintln!("Please install ffmpeg to record GIFs:");
        eprintln!("  sudo pacman -S ffmpeg");
        eprintln!("  sudo apt install ffmpeg");
        return Err(RecordError::NoEncoderFound);
    }

    // Build pipeline: Source -> videoconvert -> rgba -> appsink
    let source_str = if std::env::var("WAYLAND_DISPLAY").is_ok() {
        get_wayland_source(&config).await?
    } else {
        get_x11_source(&config)?
    };

    let pipeline_str = format!(
        "{} ! videoconvert ! videorate ! video/x-raw,format=RGBA,framerate=25/1 ! appsink name=sink emit-signals=true sync=false drop=false max-buffers=200",
        source_str
    );

    let pipeline = gst::parse::launch(&pipeline_str)
        .map_err(|e| RecordError::GStreamerError(format!("Failed to parse pipeline: {}", e)))
        ?.downcast::<gst::Pipeline>()
        .map_err(|_| RecordError::GStreamerError("Cast to Pipeline failed".into()))?;

    let appsink = pipeline.by_name("sink")
        .ok_or_else(|| RecordError::GStreamerError("AppSink not found".into()))? 
        .downcast::<gst_app::AppSink>()
        .map_err(|_| RecordError::GStreamerError("Cast to AppSink failed".into()))?;

    // Start pipeline
    pipeline.set_state(gst::State::Playing)
        .map_err(|e| RecordError::GStreamerError(format!("Failed to start pipeline: {}", e)))?;

    println!("Recording GIF... Press Ctrl+C to stop.");

    // Handle Ctrl+C
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    let mut stopping = false;
    let mut ffmpeg_child: Option<std::process::Child> = None;
    
    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                println!("\nStopping recording...");
                stopping = true;
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(1)) => {
                // Pull sample
                match appsink.try_pull_sample(gst::ClockTime::from_mseconds(5)) {
                    Some(sample) => {
                        let buffer = sample.buffer().ok_or_else(|| RecordError::GStreamerError("No buffer in sample".into()))?;
                        let map = buffer.map_readable().map_err(|_| RecordError::GStreamerError("Failed to map buffer".into()))?;
                        
                        // Initialize FFmpeg on first frame
                        if ffmpeg_child.is_none() {
                            let caps = sample.caps().ok_or_else(|| RecordError::GStreamerError("No caps".into()))?;
                            let structure = caps.structure(0).ok_or_else(|| RecordError::GStreamerError("No structure".into()))?;
                            let width = structure.get::<i32>("width").map_err(|_| RecordError::GStreamerError("No width".into()))? as u32;
                            let height = structure.get::<i32>("height").map_err(|_| RecordError::GStreamerError("No height".into()))? as u32;

                            println!("Detected stream: {}x{}", width, height);

                            let child = Command::new("ffmpeg")
                                .arg("-y") // Overwrite
                                .arg("-loglevel").arg("warning")
                                .arg("-nostats")
                                .arg("-f").arg("rawvideo")
                                .arg("-pix_fmt").arg("rgba")
                                .arg("-s").arg(format!("{}x{}", width, height))
                                .arg("-r").arg("25")
                                .arg("-i").arg("pipe:0")
                                // High quality GIF palette generation
                                .arg("-vf").arg("split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse")
                                .arg(&config.output_path)
                                .stdin(Stdio::piped())
                                .stdout(Stdio::null())
                                .stderr(Stdio::inherit())
                                .spawn()
                                .map_err(|e| RecordError::IoError(e))?;
                            
                            ffmpeg_child = Some(child);
                        }

                        // Write to FFmpeg stdin
                        if let Some(child) = &mut ffmpeg_child {
                            if let Some(stdin) = &mut child.stdin {
                                if let Err(e) = stdin.write_all(map.as_slice()) {
                                    // Broken pipe usually means ffmpeg exited
                                    if e.kind() != std::io::ErrorKind::BrokenPipe {
                                        eprintln!("Failed to write to ffmpeg: {}", e);
                                    }
                                    stopping = true;
                                }
                            }
                        }
                    }
                    None => {
                        // No data yet
                    }
                }
            }
        }
        if stopping { break; }
    }

    // Stop pipeline
    pipeline.set_state(gst::State::Null)
        .map_err(|e| RecordError::GStreamerError(format!("Failed to stop pipeline: {}", e)))?;

    // Close stdin to signal EOF to ffmpeg
    if let Some(mut child) = ffmpeg_child {
        drop(child.stdin.take()); // Close stdin
        println!("Finalizing GIF (FFmpeg processing)...");
        let status = child.wait().map_err(|e| RecordError::IoError(e))?;
        
        if !status.success() {
            let code = status.code();
            #[cfg(unix)]
            let signal = {
                use std::os::unix::process::ExitStatusExt;
                status.signal()
            };
            #[cfg(not(unix))]
            let signal = None;

            // Signal 2 (SIGINT) is expected because Ctrl+C hits the whole process group.
            // Some FFmpeg versions/filters return 255 or 130 on interruption.
            let is_expected_interruption = signal == Some(2) || code == Some(255) || code == Some(130);

            if !is_expected_interruption {
                return Err(RecordError::GifError(format!("FFmpeg failed with status: {}", status)));
            }
        }
    } else {
        return Err(RecordError::GifError("No frames captured".into()));
    }

    println!("GIF saved to {:?}", config.output_path);
    Ok(config.output_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_videocrop_suffix_interior_region() {
        // 300x200 region at (100,100) on a 1920x1080 screen.
        let suffix = videocrop_suffix(100, 100, 300, 200, 1920, 1080);
        assert_eq!(suffix, " ! videocrop left=100 top=100 right=1520 bottom=780");
    }

    #[test]
    fn test_videocrop_suffix_flush_against_right_bottom_edge() {
        // Region touching the screen's right/bottom edge: margins must be
        // exactly 0, not negative.
        let suffix = videocrop_suffix(1620, 880, 300, 200, 1920, 1080);
        assert_eq!(suffix, " ! videocrop left=1620 top=880 right=0 bottom=0");
    }

    #[test]
    fn test_apply_wayland_crop_omits_crop_when_any_field_missing() {
        // record screen / record window today's behavior: no full crop
        // rect + screen size means the base pipewiresrc string comes back
        // unchanged. Exercises the real function get_wayland_source calls,
        // not a re-derived copy of its condition.
        let base = "pipewiresrc path=42 do-timestamp=true".to_string();
        let config = RecordingConfig {
            x: Some(100),
            y: Some(100),
            width: Some(300),
            height: Some(200),
            screen_width: None,
            screen_height: None,
            ..RecordingConfig::default()
        };

        assert_eq!(apply_wayland_crop(base.clone(), &config), base);
    }

    #[test]
    fn test_apply_wayland_crop_appends_suffix_when_all_fields_present() {
        let base = "pipewiresrc path=42 do-timestamp=true".to_string();
        let config = RecordingConfig {
            x: Some(100),
            y: Some(100),
            width: Some(300),
            height: Some(200),
            screen_width: Some(1920),
            screen_height: Some(1080),
            ..RecordingConfig::default()
        };

        assert_eq!(
            apply_wayland_crop(base, &config),
            "pipewiresrc path=42 do-timestamp=true ! videocrop left=100 top=100 right=1520 bottom=780"
        );
    }

    #[test]
    fn test_recording_config_default_has_no_screen_dimensions() {
        let config = RecordingConfig::default();
        assert_eq!(config.screen_width, None);
        assert_eq!(config.screen_height, None);
    }
}