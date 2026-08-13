//! OpenShotX CLI - Screenshot tool for Linux
//!
//! Usage:
//!   cargo run -- capture screen
//!   cargo run -- capture area
//!   cargo run -- capture window
//!   cargo run -- record screen
//!   cargo run -- record area
//!   cargo run -- ocr <image>

use openshotx::{
    backend::{X11Backend, WaylandBackend, CaptureData, DisplayBackend, DisplayResult},
    capture::{save_capture, SaveConfig, ImageFormat, copy_image_to_clipboard},
    select_area,
    select_window,
    AreaAction,
    AreaPick,
    SelectionArea,
    ocr::{extract_text_from_path, OcrConfig},
    recording::{RecordingConfig, start_recording, copy_to_clipboard as copy_recording_to_clipboard},
    scrolling::{ScrollCaptureConfig, capture_scrolling_pw, save_scrolling_capture},
};
use openshotx::config::Config;
use openshotx::gui;
use std::path::{Path, PathBuf};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let config = Config::load();

    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    match args[1].as_str() {
        "capture" => {
            if args.len() < 3 {
                eprintln!("Error: missing capture type");
                print_usage();
                std::process::exit(1);
            }
            run_capture(&args, &config).await;
        }
        "record" => {
            if args.len() < 3 {
                eprintln!("Error: missing recording type");
                print_usage();
                std::process::exit(1);
            }
            if let Err(e) = run_record(&args, &config).await {
                eprintln!("Recording failed: {}", e);
                std::process::exit(1);
            }
        }
            "ocr" => {
                if args.len() < 3 {
                    eprintln!("Error: missing image path");
                    print_usage();
                    std::process::exit(1);
                }
                run_ocr(&args);
            }
            "scroll" => {
                if let Err(e) = run_scroll(&args).await {
                    eprintln!("Scrolling capture failed: {}", e);
                    std::process::exit(1);
                }
            }
            "config" => {
            gui::run_settings(config);
        }
        "tray" => {
            let start_hidden = args.iter().any(|a| a == "--hidden" || a == "--minimized");
            gui::run_tray_app(config, start_hidden);
        }
        "--help" | "-h" => print_usage(),
            _ => {
                eprintln!("Error: unknown command '{}'", args[1]);
                print_usage();
                std::process::exit(1);
            }
        }
    }
    
    fn print_usage() {
        println!("OpenShotX - Screenshot tool for Linux");
        println!();
        println!("Usage: cargo run -- <command> [options]");
        println!();
        println!("Commands:");
        println!("  capture <type>    Capture a screenshot");
        println!("  record <type>     Record video (MP4/GIF)");
        println!("  ocr <image>       Extract text from an image");
        println!("  scroll            Capture scrolling content (auto-stitch frames)");
        println!("  config            Open the settings GUI");
        println!("  tray [--hidden]   Run the tray icon + settings window");
        println!("                    (--hidden starts in the tray only; used by autostart)");
        println!();
        println!("Capture types:");
        println!("  screen            Capture the entire screen");
        println!("  area              Capture a selected area (Wayland: interactive)");
        println!("  window            Capture a specific window (Wayland: interactive)");
        println!();
        println!("Recording types:");
        println!("  screen            Record the entire screen");
        println!("  area              Record a selected region (X11: drag to select)");
        println!("  window            Record a specific window (X11: click to select)");
        println!();
        println!("Capture options:");
        println!("  --output <path>   Save to specific path (default: ~/Pictures)");
        println!("  --no-cursor       Don't include cursor in screenshot");
        println!("  --jpeg [quality]  Save as JPEG with quality 1-100 (default: PNG)");
        println!("  --prefix <text>   Prefix for filename (default: 'screenshot')");
        println!("  --ocr             Run OCR on captured image and copy to clipboard");
        println!("  --open, --edit    Open the screenshot in an editor after saving");
        println!("  --notify          Show a desktop notification when done");
        println!();
        println!("Recording options:");
        println!("  --output <path>   Save to specific path (default uses timestamped name)");
        println!("  --gif             Record as GIF and copy to clipboard");
        println!();
        println!("Scrolling capture options:");
        println!("  --output <path>   Save to specific path (default: ~/Pictures)");
        println!("  --interval <ms>   Capture interval in milliseconds (default: 200)");
        println!("  --threshold <n>   Pixel diff threshold for stability 0-100 (default: 5)");
        println!("  --stable <n>      Number of stable frames to stop (default: 3)");
        println!("  --prefix <text>   Prefix for filename (default: 'scroll')");
        println!("  --max-height <n>  Maximum output height in pixels (default: 20000)");
        println!();
        println!("Examples:");
        println!("  cargo run -- capture screen");
        println!("  cargo run -- record screen");
        println!("  cargo run -- record area --gif");
        println!("  cargo run -- record window");
        println!("  cargo run -- scroll");
    }
    
async fn run_capture(args: &[String], config: &Config) {
        // Parse capture type
        let capture_type = args[2].as_str();
    
        // Parse options
        let mut output_path: Option<PathBuf> = None;
        let mut include_cursor = config.capture.include_cursor;
        let mut use_jpeg = matches!(config.capture.format, openshotx::config::CaptureFormat::Jpeg);
        let mut jpeg_quality = config.capture.jpeg_quality;
        let mut prefix: Option<String> = None;
        let mut run_ocr = false;
        let mut open_after = false;
        let mut notify = false;
        let mut ocr_lang: Option<String> = None;
        let mut ocr_min_conf: Option<i32> = None;
        let mut ocr_clipboard = true;
    
        let mut i = 3;
        while i < args.len() {
            match args[i].as_str() {
                "--output" => {
                    if i + 1 >= args.len() {
                        eprintln!("Error: --output requires a path");
                        std::process::exit(1);
                    }
                    output_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                }
                "--no-cursor" => {
                    include_cursor = false;
                    i += 1;
                }
                "--jpeg" => {
                    use_jpeg = true;
                    // Check if next arg is a number
                    if i + 1 < args.len() {
                        if let Ok(q) = args[i + 1].parse::<u8>() {
                            jpeg_quality = q;
                            i += 2;
                        } else {
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                }
                "--prefix" => {
                    if i + 1 >= args.len() {
                        eprintln!("Error: --prefix requires text");
                        std::process::exit(1);
                    }
                    prefix = Some(args[i + 1].clone());
                    i += 2;
                }
                "--ocr" => {
                    run_ocr = true;
                    i += 1;
                }
                "--open" | "--edit" => {
                    open_after = true;
                    i += 1;
                }
                "--notify" => {
                    notify = true;
                    i += 1;
                }
                "--lang" => {
                    if i + 1 >= args.len() {
                        eprintln!("Error: --lang requires a language code");
                        std::process::exit(1);
                    }
                    ocr_lang = Some(args[i + 1].clone());
                    i += 2;
                }
                "--min-conf" => {
                    if i + 1 >= args.len() {
                        eprintln!("Error: --min-conf requires a number");
                        std::process::exit(1);
                    }
                    let value: i32 = match args[i + 1].parse() {
                        Ok(v) => v,
                        Err(_) => {
                            eprintln!("Error: --min-conf requires a valid number");
                            std::process::exit(1);
                        }
                    };
                    ocr_min_conf = Some(value);
                    i += 2;
                }
                "--no-clipboard" => {
                    ocr_clipboard = false;
                    i += 1;
                }
                _ => {
                    eprintln!("Error: unknown option '{}'", args[i]);
                    std::process::exit(1);
                }
            }
        }
    
        // "area" always goes through the GTK overlay's Capture/Record
        // control panel when X11/XWayland is reachable, regardless of
        // Wayland vs X11 session type (issue #1: on Wayland, preferring
        // WaylandBackend here made the overlay/panel unreachable and
        // capture area fell back to GNOME's native picker, which has no
        // recording toggle). "screen"/"window" keep the existing
        // Wayland-preferred backend selection, unchanged.
        let capture: CaptureData = if capture_type == "area" {
            if X11Backend::is_supported() {
                println!("Select an area by dragging the mouse, then choose Capture or Record from the panel.");
                match select_area(AreaAction::Capture).expect("Failed to show area selection overlay") {
                    Some(AreaPick { action: AreaAction::Capture, area, .. }) => {
                        capture_area_pixels(area).expect("Area capture failed")
                    }
                    Some(AreaPick { action: AreaAction::Record, area, screen_width, screen_height }) => {
                        // Switched to recording from the control panel.
                        if let Err(e) = record_area_default(config, area, screen_width, screen_height, notify).await {
                            eprintln!("Recording failed: {}", e);
                            std::process::exit(1);
                        }
                        std::process::exit(0);
                    }
                    None => {
                        eprintln!("Selection cancelled");
                        std::process::exit(0);
                    }
                }
            } else if WaylandBackend::is_supported() {
                // No X11/XWayland reachable at all (rare Wayland-only
                // compositor): fall back to today's native interactive
                // portal picker. No Capture/Record toggle here -- explicit,
                // disclosed scope narrowing, not a regression from before
                // this fix existed.
                println!("Note: On Wayland, area capture requires user interaction via portal dialog");
                WaylandBackend::new().expect("Failed to initialize Wayland backend")
                    .capture_area(0, 0, 0, 0).expect("Area capture failed")
            } else {
                eprintln!("Error: No supported display backend found");
                eprintln!("This application requires X11 or Wayland");
                std::process::exit(1);
            }
        } else if WaylandBackend::is_supported() {
            println!("Using Wayland backend...");
            let backend = WaylandBackend::new().expect("Failed to initialize Wayland backend");
    
            match capture_type {
                "screen" => backend.capture_screen().expect("Screen capture failed"),
                "window" => {
                    println!("Note: On Wayland, window capture requires user interaction via portal dialog");
                    backend.capture_window(0).expect("Window capture failed")
                }
                _ => {
                    eprintln!("Error: unknown capture type '{}'", capture_type);
                    print_usage();
                    std::process::exit(1);
                }
            }
        } else if X11Backend::is_supported() {
            println!("Using X11 backend...");
            let backend = X11Backend::new().expect("Failed to initialize X11 backend");
    
            match capture_type {
                "screen" => backend.capture_screen().expect("Screen capture failed"),
                "window" => {
                    eprintln!("Error: window capture by ID not yet supported via CLI");
                    eprintln!("Use 'capture screen' and crop manually");
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("Error: unknown capture type '{}'", capture_type);
                    print_usage();
                    std::process::exit(1);
                }
            }
        } else {
            eprintln!("Error: No supported display backend found");
            eprintln!("This application requires X11 or Wayland");
            std::process::exit(1);
        };
    
        println!("Captured: {}x{}", capture.width, capture.height);
        println!("Format: {:?} ({} bpp)", capture.format, capture.format.bits_per_pixel);
        if capture.cursor.is_some() {
            println!("Cursor: captured ({})", if include_cursor { "will include" } else { "will exclude" });
        }
    
        // Build save config
        let format = if use_jpeg {
            ImageFormat::Jpeg { quality: jpeg_quality }
        } else {
            ImageFormat::Png
        };
    
        let output_dir = output_path.unwrap_or_else(|| {
            PathBuf::from(shellexpand::tilde(&config.paths.screenshots).as_ref())
        });
        let effective_prefix = prefix.unwrap_or_else(|| config.capture.prefix.clone());

        let save_config = SaveConfig::default()
            .with_format(format)
            .with_cursor(include_cursor)
            .with_output_dir(output_dir)
            .with_prefix(effective_prefix);
    
        // Save the capture
        let saved_path = match save_capture(&capture, &save_config) {
            Ok(path) => {
                println!("Saved to: {}", path.display());
                path
            }
            Err(e) => {
                eprintln!("Error saving capture: {}", e);
                std::process::exit(1);
            }
        };

        // Copy image to clipboard (for non-OCR captures, OCR has its own clipboard handling)
        if !run_ocr && config.capture.copy_to_clipboard {
            if let Err(e) = copy_image_to_clipboard(&saved_path) {
                eprintln!("Warning: Failed to copy image to clipboard: {}", e);
            }
        }

        // Open the screenshot in an editor / default viewer if requested
        if open_after {
            if let Err(e) = open_in_editor(&saved_path, &config.capture.editor) {
                eprintln!("Warning: Failed to open screenshot: {}", e);
            }
        }

        // Desktop notification for non-OCR captures (OCR notifies after extraction)
        if notify && !run_ocr {
            send_notification("Screenshot saved", &saved_path.display().to_string());
        }
    
        // Run OCR if requested
        if run_ocr {
            println!("Running OCR...");
            let mut ocr_config = OcrConfig::default()
                .with_clipboard(ocr_clipboard);
    
            if let Some(lang) = ocr_lang {
                ocr_config = ocr_config.with_language(lang);
            }
    
            if let Some(conf) = ocr_min_conf {
                ocr_config = ocr_config.with_min_confidence(conf);
            }
    
            match extract_text_from_path(&saved_path, &ocr_config) {
                Ok(result) => {
                    println!("OCR successful!");
                    println!("Confidence: {}%", result.confidence);
                    println!("Extracted text:");
                    println!("{}", "-".repeat(40));
                    println!("{}", result.text);
                    println!("{}", "-".repeat(40));
                    if result.copied_to_clipboard {
                        println!("Text copied to clipboard");
                    }
                    if notify {
                        let body = if result.copied_to_clipboard {
                            "Text copied to clipboard".to_string()
                        } else {
                            format!("{}% confidence", result.confidence)
                        };
                        send_notification("OCR complete", &body);
                    }
                }
                Err(e) => {
                    eprintln!("OCR failed: {}", e);
                    if notify {
                        send_notification("OCR failed", &e.to_string());
                    }
                    std::process::exit(1);
                }
            }
        }
    }

    /// Send a desktop notification via `notify-send`.
    ///
    /// Best-effort: if `notify-send` is missing or fails, it is silently ignored
    /// so capture flows never break on notification problems.
    fn send_notification(summary: &str, body: &str) {
        let _ = std::process::Command::new("notify-send")
            .arg("--app-name=OpenShotX")
            .arg(summary)
            .arg(body)
            .spawn();
    }

    /// Open a saved screenshot in an editor.
    ///
    /// Uses the configured editor command when set, otherwise falls back to the
    /// system default handler via `xdg-open`. The child is spawned detached so
    /// the CLI returns immediately.
    fn open_in_editor(path: &Path, editor: &str) -> std::io::Result<()> {
        let (program, args): (&str, Vec<&str>) = if editor.trim().is_empty() {
            ("xdg-open", vec![])
        } else {
            let mut parts = editor.split_whitespace();
            let prog = parts.next().unwrap_or("xdg-open");
            (prog, parts.collect())
        };

        println!("Opening in {}...", program);
        std::process::Command::new(program)
            .args(&args)
            .arg(path)
            .spawn()
            .map(|_| ())
    }

    /// Capture the pixels of `area`, choosing the right backend for this
    /// session: on Wayland, `X11Backend::capture_area` can't see real
    /// desktop content (XWayland's root window doesn't reflect Wayland
    /// client compositing), so grab the full monitor through the portal
    /// and crop client-side; on native X11, capture the region directly.
    fn capture_area_pixels(area: SelectionArea) -> DisplayResult<CaptureData> {
        if WaylandBackend::is_supported() {
            WaylandBackend::new()?
                .capture_screen()?
                .crop(area.x, area.y, area.width, area.height)
        } else {
            X11Backend::new()?.capture_area(area.x, area.y, area.width, area.height)
        }
    }

    /// Capture and save a screenshot of `area` using config defaults (no
    /// CLI flags), used when the user picks Capture from the control panel
    /// while running `record area`.
    fn capture_area_default(config: &Config, area: SelectionArea, notify: bool) -> Result<(), Box<dyn std::error::Error>> {
        let capture = capture_area_pixels(area)
            .map_err(|e| format!("Area capture failed: {}", e))?;

        let format = if matches!(config.capture.format, openshotx::config::CaptureFormat::Jpeg) {
            ImageFormat::Jpeg { quality: config.capture.jpeg_quality }
        } else {
            ImageFormat::Png
        };
        let output_dir = PathBuf::from(shellexpand::tilde(&config.paths.screenshots).as_ref());
        let save_config = SaveConfig::default()
            .with_format(format)
            .with_cursor(config.capture.include_cursor)
            .with_output_dir(output_dir)
            .with_prefix(config.capture.prefix.clone());

        let saved_path = save_capture(&capture, &save_config)?;
        println!("Saved to: {}", saved_path.display());

        if config.capture.copy_to_clipboard {
            if let Err(e) = copy_image_to_clipboard(&saved_path) {
                eprintln!("Warning: Failed to copy image to clipboard: {}", e);
            }
        }
        if notify {
            send_notification("Screenshot saved", &saved_path.display().to_string());
        }
        Ok(())
    }

    /// Record `area` using config defaults (no CLI flags), used when the
    /// user picks Record from the control panel while running `capture area`.
    /// `screen_width`/`screen_height` are the monitor-0 dimensions the
    /// overlay measured `area` against (needed to crop the Wayland
    /// ScreenCast stream down to `area`; ignored on the native X11 path).
    async fn record_area_default(
        config: &Config,
        area: SelectionArea,
        screen_width: u32,
        screen_height: u32,
        notify: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let ext = match config.recording.format {
            openshotx::config::RecordingFormat::Mp4 => "mp4",
            openshotx::config::RecordingFormat::Webm => "webm",
        };
        let rec_config = RecordingConfig {
            output_path: openshotx::recording::generate_recording_filename(
                &config.paths.videos,
                &config.recording.prefix,
                ext,
            ),
            highlight_cursor: config.recording.highlight_cursor,
            highlight_color: config.recording.highlight_color.clone(),
            highlight_radius: config.recording.highlight_radius,
            x: Some(area.x),
            y: Some(area.y),
            width: Some(area.width as u32),
            height: Some(area.height as u32),
            screen_width: Some(screen_width),
            screen_height: Some(screen_height),
        };

        let final_path = start_recording(rec_config).await?;
        if notify {
            send_notification("Recording saved", &final_path.display().to_string());
        }
        Ok(())
    }

    fn run_ocr(args: &[String]) {
        let image_path = &args[2];
    
        // Parse OCR options
        let mut ocr_lang: Option<String> = None;
        let mut ocr_min_conf: Option<i32> = None;
        let mut ocr_clipboard = true;
    
        let mut i = 3;
        while i < args.len() {
            match args[i].as_str() {
                "--lang" => {
                    if i + 1 >= args.len() {
                        eprintln!("Error: --lang requires a language code");
                        std::process::exit(1);
                    }
                    ocr_lang = Some(args[i + 1].clone());
                    i += 2;
                }
                "--min-conf" => {
                    if i + 1 >= args.len() {
                        eprintln!("Error: --min-conf requires a number");
                        std::process::exit(1);
                    }
                    let value: i32 = match args[i + 1].parse() {
                        Ok(v) => v,
                        Err(_) => {
                            eprintln!("Error: --min-conf requires a valid number");
                            std::process::exit(1);
                        }
                    };
                    ocr_min_conf = Some(value);
                    i += 2;
                }
                "--no-clipboard" => {
                    ocr_clipboard = false;
                    i += 1;
                }
                _ => {
                    eprintln!("Error: unknown option '{}'", args[i]);
                    print_usage();
                    std::process::exit(1);
                }
            }
        }
    
        // Build OCR config
        let mut ocr_config = OcrConfig::default()
            .with_clipboard(ocr_clipboard);
    
        if let Some(lang) = ocr_lang {
            ocr_config = ocr_config.with_language(lang);
        }
    
        if let Some(conf) = ocr_min_conf {
            ocr_config = ocr_config.with_min_confidence(conf);
        }
    
        // Run OCR
        println!("Running OCR on: {}", image_path);
        match extract_text_from_path(image_path, &ocr_config) {
            Ok(result) => {
                println!("OCR successful!");
                println!("Confidence: {}%", result.confidence);
                println!("Extracted text:");
                println!("{}", "-".repeat(40));
                println!("{}", result.text);
                println!("{}", "-".repeat(40));
                if result.copied_to_clipboard {
                    println!("Text copied to clipboard");
                }
            }
            Err(e) => {
                eprintln!("OCR failed: {}", e);
                std::process::exit(1);
            }
        }
    }
    
async fn run_record(args: &[String], config: &Config) -> Result<(), Box<dyn std::error::Error>> {
        let record_type = args[2].as_str();
        let mut output_path: Option<PathBuf> = None;
        let mut is_gif = false;
        let mut notify = false;

        let mut i = 3;
        while i < args.len() {
            match args[i].as_str() {
                "--output" => {
                    if i + 1 >= args.len() {
                        eprintln!("Error: --output requires a path");
                        std::process::exit(1);
                    }
                    output_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                }
                "--gif" => {
                    is_gif = true;
                    i += 1;
                }
                "--notify" => {
                    notify = true;
                    i += 1;
                }
                _ => {
                    eprintln!("Error: unknown option '{}'", args[i]);
                    std::process::exit(1);
                }
            }
        }
    
                let mut rec_config = RecordingConfig::default();

                // Configure output path: CLI --output overrides config, else use config.paths.videos
                if let Some(p) = output_path {
                    rec_config.output_path = p;
                    if is_gif && rec_config.output_path.extension().map(|e| e != "gif").unwrap_or(true) {
                        rec_config.output_path.set_extension("gif");
                    }
                } else {
                    let ext = if is_gif {
                        "gif"
                    } else {
                        match config.recording.format {
                            openshotx::config::RecordingFormat::Mp4 => "mp4",
                            openshotx::config::RecordingFormat::Webm => "webm",
                        }
                    };
                    rec_config.output_path = openshotx::recording::generate_recording_filename(
                        &config.paths.videos,
                        &config.recording.prefix,
                        ext,
                    );
                }
                rec_config.highlight_cursor = config.recording.highlight_cursor;
                rec_config.highlight_color = config.recording.highlight_color.clone();
                rec_config.highlight_radius = config.recording.highlight_radius;
            
                // Handle area/window selection if needed
                if record_type == "area" {
                    // Always try the overlay when X11/XWayland is reachable
                    // (issue #1: gating on WAYLAND_DISPLAY-unset meant a
                    // Wayland session got no region UI at all here).
                    if X11Backend::is_supported() {
                         println!("Select an area to record, then choose Capture or Record from the panel.");

                         match select_area(AreaAction::Record).map_err(|e| format!("Selection failed: {}", e))? {
                             Some(AreaPick { action: AreaAction::Record, area, screen_width, screen_height }) => {
                                 rec_config.x = Some(area.x);
                                 rec_config.y = Some(area.y);
                                 rec_config.width = Some(area.width as u32);
                                 rec_config.height = Some(area.height as u32);
                                 rec_config.screen_width = Some(screen_width);
                                 rec_config.screen_height = Some(screen_height);
                             }
                             Some(AreaPick { action: AreaAction::Capture, area, .. }) => {
                                 // Switched to a screenshot from the control panel.
                                 capture_area_default(config, area, notify)?;
                                 return Ok(());
                             }
                             None => {
                                 println!("Selection cancelled.");
                                 return Ok(());
                             }
                         }
                    } else {
                        // No X11/XWayland reachable at all: today's
                        // behavior unchanged (no region UI, whole-screen
                        // fallback via the persisted portal grant).
                        println!("Wayland: portal will let you select a screen or window to record.");
                    }
                } else if record_type == "window" {
                    if std::env::var("WAYLAND_DISPLAY").is_err() && X11Backend::is_supported() {
                        let selection = select_window().map_err(|e| format!("Window selection failed: {}", e))?;
                        if let Some(area) = selection {
                            rec_config.x = Some(area.x);
                            rec_config.y = Some(area.y);
                            rec_config.width = Some(area.width as u32);
                            rec_config.height = Some(area.height as u32);
                        } else {
                            println!("Selection cancelled.");
                            return Ok(());
                        }
                    } else {
                        println!("Wayland: portal will let you select a window to record.");
                    }
                } else if record_type != "screen" {
                     eprintln!("Error: recording type '{}' not supported (use 'screen', 'area', or 'window')", record_type);
                     std::process::exit(1);
                }

                let final_path = start_recording(rec_config).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
                
                // Post-processing
                if let Some(ext) = final_path.extension() {
                    if ext == "gif" {
                        // For GIFs, we default to copying to clipboard (feature requested)
                        if let Err(e) = copy_recording_to_clipboard(&final_path) {
                            eprintln!("Warning: Failed to copy GIF to clipboard: {}", e);
                        }
                    }
                }

                if notify {
                    send_notification("Recording saved", &final_path.display().to_string());
                }

                Ok(())
            }

    async fn run_scroll(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
        // Parse scroll options
        let mut output_path: Option<PathBuf> = None;
        let mut interval_ms: Option<u64> = None;
        let mut threshold: Option<u8> = None;
        let mut stable_count: Option<usize> = None;
        let mut prefix: Option<String> = None;
        let mut max_height: Option<u32> = None;

        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--output" => {
                    if i + 1 >= args.len() {
                        eprintln!("Error: --output requires a path");
                        std::process::exit(1);
                    }
                    output_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                }
                "--interval" => {
                    if i + 1 >= args.len() {
                        eprintln!("Error: --interval requires a number");
                        std::process::exit(1);
                    }
                    interval_ms = Some(args[i + 1].parse::<u64>()
                        .expect("Interval must be a number"));
                    i += 2;
                }
                "--threshold" => {
                    if i + 1 >= args.len() {
                        eprintln!("Error: --threshold requires a number");
                        std::process::exit(1);
                    }
                    threshold = Some(args[i + 1].parse::<u8>()
                        .expect("Threshold must be a number"));
                    i += 2;
                }
                "--stable" => {
                    if i + 1 >= args.len() {
                        eprintln!("Error: --stable requires a number");
                        std::process::exit(1);
                    }
                    stable_count = Some(args[i + 1].parse::<usize>()
                        .expect("Stable count must be a number"));
                    i += 2;
                }
                "--prefix" => {
                    if i + 1 >= args.len() {
                        eprintln!("Error: --prefix requires text");
                        std::process::exit(1);
                    }
                    prefix = Some(args[i + 1].clone());
                    i += 2;
                }
                "--max-height" => {
                    if i + 1 >= args.len() {
                        eprintln!("Error: --max-height requires a number");
                        std::process::exit(1);
                    }
                    max_height = Some(args[i + 1].parse::<u32>()
                        .expect("Max height must be a number"));
                    i += 2;
                }
                _ => {
                    eprintln!("Error: unknown option '{}'", args[i]);
                    std::process::exit(1);
                }
            }
        }

        // Build scroll config
        let mut config = ScrollCaptureConfig::default();

        if let Some(interval) = interval_ms {
            config = config.with_capture_interval(std::time::Duration::from_millis(interval));
        }

        if let Some(thresh) = threshold {
            config = config.with_stability_threshold(thresh);
        }

        if let Some(stable) = stable_count {
            config = config.with_stable_frame_count(stable);
        }

        if let Some(h) = max_height {
            config = config.with_max_height(h);
        }

        if let Some(p) = prefix {
            config.save_config = config.save_config.with_prefix(p);
        }

        if let Some(path) = output_path {
            config.save_config = config.save_config.with_output_dir(path);
        }

        // Run scrolling capture (works on both X11 and Wayland via PipeWire)
        let result = capture_scrolling_pw(&config).await?;

        // Save result
        let saved_path = save_scrolling_capture(&result, &config)?;
        println!("\nSaved to: {}", saved_path.display());

        Ok(())
    }