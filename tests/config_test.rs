use openshotx::config::{Config, CaptureFormat};
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn defaults_when_file_missing() {
    let cfg = Config::load_from(std::path::Path::new("/nonexistent/path/config.yaml"));
    assert_eq!(cfg.paths.screenshots, "~/Pictures");
    assert_eq!(cfg.paths.videos, "~/Videos");
    assert_eq!(cfg.capture.jpeg_quality, 85);
    assert!(cfg.capture.copy_to_clipboard);
}

#[test]
fn round_trip_yaml() {
    let mut cfg = Config::default();
    cfg.paths.screenshots = "/tmp/shots".to_string();
    cfg.capture.jpeg_quality = 60;
    cfg.hotkeys.capture_area = "Super+Print".to_string();

    let mut f = NamedTempFile::new().unwrap();
    cfg.save_to(f.path()).unwrap();

    let loaded = Config::load_from(f.path());
    assert_eq!(loaded.paths.screenshots, "/tmp/shots");
    assert_eq!(loaded.capture.jpeg_quality, 60);
    assert_eq!(loaded.hotkeys.capture_area, "Super+Print");
}

#[test]
fn partial_yaml_fills_missing_with_defaults() {
    let yaml = "paths:\n  screenshots: /custom/shots\n";
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();

    let cfg = Config::load_from(f.path());
    assert_eq!(cfg.paths.screenshots, "/custom/shots");
    assert_eq!(cfg.paths.videos, "~/Videos"); // default
}
