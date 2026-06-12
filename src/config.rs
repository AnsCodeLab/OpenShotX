use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub paths: PathsConfig,
    #[serde(default)]
    pub capture: CaptureConfig,
    #[serde(default)]
    pub recording: RecordingConfig,
    #[serde(default)]
    pub hotkeys: HotkeysConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CaptureFormat {
    Png,
    Jpeg,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RecordingFormat {
    Mp4,
    Webm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsConfig {
    #[serde(default = "default_screenshots")]
    pub screenshots: String,
    #[serde(default = "default_videos")]
    pub videos: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    #[serde(default = "default_capture_format")]
    pub format: CaptureFormat,
    #[serde(default = "default_jpeg_quality")]
    pub jpeg_quality: u8,
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default = "default_copy_to_clipboard")]
    pub copy_to_clipboard: bool,
    #[serde(default)]
    pub include_cursor: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingConfig {
    #[serde(default = "default_recording_format")]
    pub format: RecordingFormat,
    #[serde(default = "default_recording_output")]
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeysConfig {
    #[serde(default = "default_capture_area")]
    pub capture_area: String,
    #[serde(default = "default_capture_screen")]
    pub capture_screen: String,
    #[serde(default = "default_capture_window")]
    pub capture_window: String,
    #[serde(default = "default_record_area")]
    pub record_area: String,
    #[serde(default = "default_record_screen")]
    pub record_screen: String,
}

fn default_screenshots() -> String { "~/Pictures".to_string() }
fn default_videos() -> String { "~/Videos".to_string() }
fn default_capture_format() -> CaptureFormat { CaptureFormat::Png }
fn default_jpeg_quality() -> u8 { 85 }
fn default_prefix() -> String { "screenshot".to_string() }
fn default_copy_to_clipboard() -> bool { true }
fn default_recording_format() -> RecordingFormat { RecordingFormat::Mp4 }
fn default_recording_output() -> String { "~/Videos".to_string() }
fn default_capture_area() -> String { "<Super><Shift>4".to_string() }
fn default_capture_screen() -> String { "Print".to_string() }
fn default_capture_window() -> String { "<Alt>Print".to_string() }
fn default_record_area() -> String { "<Super><Shift>5".to_string() }
fn default_record_screen() -> String { "<Super><Shift>6".to_string() }

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            screenshots: "~/Pictures".to_string(),
            videos: "~/Videos".to_string(),
        }
    }
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            format: CaptureFormat::Png,
            jpeg_quality: 85,
            prefix: "screenshot".to_string(),
            copy_to_clipboard: true,
            include_cursor: false,
        }
    }
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            format: RecordingFormat::Mp4,
            output: "~/Videos".to_string(),
        }
    }
}

impl Default for HotkeysConfig {
    fn default() -> Self {
        Self {
            capture_area: "<Super><Shift>4".to_string(),
            capture_screen: "Print".to_string(),
            capture_window: "<Alt>Print".to_string(),
            record_area: "<Super><Shift>5".to_string(),
            record_screen: "<Super><Shift>6".to_string(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            paths: PathsConfig::default(),
            capture: CaptureConfig::default(),
            recording: RecordingConfig::default(),
            hotkeys: HotkeysConfig::default(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        Self::load_from(&config_file_path())
    }

    pub fn load_from(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };
        serde_yml::from_str(&content).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&config_file_path())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_yml::to_string(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, content)
    }
}

pub fn config_file_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(".config")
        })
        .join("openshotx")
        .join("config.yaml")
}
