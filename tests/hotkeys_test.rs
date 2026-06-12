use openshotx::hotkeys::{detect_desktop, Desktop, hotkey_display};
use std::sync::Mutex;

// Serialize all tests that mutate XDG_CURRENT_DESKTOP to avoid race conditions.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn detects_gnome() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("XDG_CURRENT_DESKTOP", "GNOME");
    assert!(matches!(detect_desktop(), Desktop::Gnome));
}

#[test]
fn detects_kde() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("XDG_CURRENT_DESKTOP", "KDE");
    assert!(matches!(detect_desktop(), Desktop::Kde));
}

#[test]
fn detects_unknown() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("XDG_CURRENT_DESKTOP", "something-weird");
    assert!(matches!(detect_desktop(), Desktop::Unknown));
}

#[test]
fn hotkey_display_strips_angle_brackets() {
    assert_eq!(hotkey_display("<Super><Shift>4"), "Super+Shift+4");
    assert_eq!(hotkey_display("<Alt>Print"), "Alt+Print");
    assert_eq!(hotkey_display("Print"), "Print");
}
