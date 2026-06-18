//! Manage the XDG autostart entry that launches the tray on login.
//!
//! The autostart `.desktop` file's *existence* is the source of truth for
//! whether the tray starts automatically. The config `tray.autostart` bool
//! mirrors it for display purposes.

use std::io;
use std::path::{Path, PathBuf};

/// Basename of the autostart entry.
const ENTRY_NAME: &str = "openshotx-tray.desktop";

/// Path to the autostart entry: `$XDG_CONFIG_HOME/autostart/openshotx-tray.desktop`
/// (falling back to `~/.config/autostart`).
pub fn autostart_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
        })
        .join("autostart")
        .join(ENTRY_NAME)
}

/// Build the contents of the autostart `.desktop` file.
///
/// `exec` is the absolute path to the binary; the entry runs `<exec> tray`.
fn entry_contents(exec: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=OpenShotX Tray\n\
         Comment=OpenShotX system tray (quick capture)\n\
         Exec={} tray --hidden\n\
         Icon=openshotx\n\
         Terminal=false\n\
         Categories=Graphics;\n\
         X-GNOME-Autostart-enabled=true\n",
        exec.display()
    )
}

/// Resolve the absolute path to the currently running binary.
fn current_exe() -> io::Result<PathBuf> {
    std::env::current_exe()
}

/// Enable autostart by writing the entry to the default location.
pub fn enable() -> io::Result<()> {
    enable_at(&autostart_path(), &current_exe()?)
}

/// Disable autostart by removing the entry from the default location.
pub fn disable() -> io::Result<()> {
    disable_at(&autostart_path())
}

/// Whether autostart is enabled at the default location.
pub fn is_enabled() -> bool {
    autostart_path().exists()
}

/// Write the autostart entry at `path` pointing `Exec` at `exec`.
pub fn enable_at(path: &Path, exec: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, entry_contents(exec))
}

/// Remove the autostart entry at `path`. Missing file is treated as success.
pub fn disable_at(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        // Unique per test via thread name; avoids Date/rand (unavailable in some envs).
        let base = std::env::temp_dir().join("openshotx-autostart-tests");
        let name = std::thread::current().name().unwrap_or("default").replace("::", "_");
        base.join(name)
    }

    #[test]
    fn enable_creates_entry_with_exec_path() {
        let dir = temp_dir();
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("openshotx-tray.desktop");
        let exec = Path::new("/opt/openshotx/bin/openshotx");

        enable_at(&path, exec).unwrap();

        assert!(path.exists());
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("Exec=/opt/openshotx/bin/openshotx tray"));
        assert!(contents.contains("X-GNOME-Autostart-enabled=true"));
    }

    #[test]
    fn disable_removes_entry() {
        let dir = temp_dir();
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("openshotx-tray.desktop");
        enable_at(&path, Path::new("/usr/bin/openshotx")).unwrap();
        assert!(path.exists());

        disable_at(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn disable_missing_is_ok() {
        let dir = temp_dir();
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("does-not-exist.desktop");
        // Should not error when the entry is already absent.
        disable_at(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn enable_then_present_disable_then_absent() {
        let dir = temp_dir();
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("openshotx-tray.desktop");

        enable_at(&path, Path::new("/usr/bin/openshotx")).unwrap();
        assert!(path.exists(), "entry should exist after enable");
        disable_at(&path).unwrap();
        assert!(!path.exists(), "entry should be gone after disable");
    }
}
