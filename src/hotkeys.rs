use crate::config::HotkeysConfig;

#[derive(Debug, PartialEq)]
pub enum Desktop {
    Gnome,
    Kde,
    Xfce,
    Sway,
    I3,
    Hyprland,
    Unknown,
}

pub fn detect_desktop() -> Desktop {
    match std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_uppercase()
        .as_str()
    {
        "GNOME" => Desktop::Gnome,
        "KDE" => Desktop::Kde,
        "XFCE" => Desktop::Xfce,
        "SWAY" => Desktop::Sway,
        "I3" => Desktop::I3,
        "HYPRLAND" => Desktop::Hyprland,
        _ => Desktop::Unknown,
    }
}

/// Converts gsettings binding format "<Super><Shift>4" → "Super+Shift+4" for display.
pub fn hotkey_display(binding: &str) -> String {
    binding
        .replace('<', "")
        .replace('>', "+")
        .trim_end_matches('+')
        .to_string()
}

pub fn register_gnome(hotkeys: &HotkeysConfig) -> Result<(), String> {
    let base = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings";
    let schema_base = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";

    let actions = [
        ("openshotx-capture-area",   "Capture Area (openshotx)",   "openshotx capture area",   hotkeys.capture_area.as_str()),
        ("openshotx-capture-screen", "Capture Screen (openshotx)", "openshotx capture screen", hotkeys.capture_screen.as_str()),
        ("openshotx-capture-window", "Capture Window (openshotx)", "openshotx capture window", hotkeys.capture_window.as_str()),
        ("openshotx-record-area",    "Record Area (openshotx)",    "openshotx record area",    hotkeys.record_area.as_str()),
        ("openshotx-record-screen",  "Record Screen (openshotx)",  "openshotx record screen",  hotkeys.record_screen.as_str()),
    ];

    for (id, name, command, binding) in &actions {
        let path_schema = format!("{}:{}/{}/", schema_base, base, id);
        gsettings(&["set", &path_schema, "name",    name])?;
        gsettings(&["set", &path_schema, "command", command])?;
        gsettings(&["set", &path_schema, "binding", binding])?;
    }

    let existing = gsettings_get("org.gnome.settings-daemon.plugins.media-keys", "custom-keybindings")?;
    let mut paths = parse_gvariant_strv(&existing);
    paths.retain(|p| !p.contains("/openshotx-"));
    for (id, _, _, _) in &actions {
        paths.push(format!("{}/{}/", base, id));
    }
    let list = format!("[{}]",
        paths.iter().map(|p| format!("'{}'", p)).collect::<Vec<_>>().join(", "));
    gsettings(&["set", "org.gnome.settings-daemon.plugins.media-keys", "custom-keybindings", &list])?;

    Ok(())
}

pub fn register_kde(hotkeys: &HotkeysConfig) -> Result<(), String> {
    let actions = [
        ("capture-area",   "openshotx capture area",   hotkeys.capture_area.as_str()),
        ("capture-screen", "openshotx capture screen", hotkeys.capture_screen.as_str()),
        ("capture-window", "openshotx capture window", hotkeys.capture_window.as_str()),
        ("record-area",    "openshotx record area",    hotkeys.record_area.as_str()),
        ("record-screen",  "openshotx record screen",  hotkeys.record_screen.as_str()),
    ];
    for (id, command, _binding) in &actions {
        let status = std::process::Command::new("kwriteconfig6")
            .args(["--file", "khotkeysrc",
                   "--group", &format!("openshotx-{}", id),
                   "--key", "Exec", command])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("kwriteconfig6 exited with {:?}", status.code()));
        }
    }
    Ok(())
}

pub fn register_xfce(hotkeys: &HotkeysConfig) -> Result<(), String> {
    let actions = [
        (hotkeys.capture_area.as_str(),   "openshotx capture area"),
        (hotkeys.capture_screen.as_str(), "openshotx capture screen"),
        (hotkeys.capture_window.as_str(), "openshotx capture window"),
        (hotkeys.record_area.as_str(),    "openshotx record area"),
        (hotkeys.record_screen.as_str(),  "openshotx record screen"),
    ];
    for (binding, command) in &actions {
        std::process::Command::new("xfconf-query")
            .args(["-c", "xfce4-keyboard-shortcuts", "-p",
                   &format!("/commands/custom/{}", binding),
                   "-s", command, "--create", "-t", "string"])
            .status()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Returns a config snippet string the user can paste into their WM config.
pub fn tiling_snippet(hotkeys: &HotkeysConfig, desktop: &Desktop) -> String {
    let fmt = |binding: &str, cmd: &str| -> String {
        let key = binding
            .replace("<Super>", "$mod+")
            .replace("<Shift>", "Shift+")
            .replace("<Alt>", "Alt+")
            .replace(['<', '>'], "");
        match desktop {
            Desktop::Sway | Desktop::I3 => format!("bindsym {} exec {}", key, cmd),
            Desktop::Hyprland => {
                let parts: Vec<&str> = key.splitn(2, '+').collect();
                if parts.len() == 2 {
                    format!("bind = {}, exec, {}", parts.join(", "), cmd)
                } else {
                    format!("bind = , {}, exec, {}", key, cmd)
                }
            }
            _ => format!("{} → {}", key, cmd),
        }
    };
    vec![
        fmt(&hotkeys.capture_area,   "openshotx capture area"),
        fmt(&hotkeys.capture_screen, "openshotx capture screen"),
        fmt(&hotkeys.capture_window, "openshotx capture window"),
        fmt(&hotkeys.record_area,    "openshotx record area"),
        fmt(&hotkeys.record_screen,  "openshotx record screen"),
    ].join("\n")
}

fn gsettings(args: &[&str]) -> Result<(), String> {
    let status = std::process::Command::new("gsettings")
        .args(args)
        .status()
        .map_err(|e| format!("gsettings: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("gsettings exited with {:?}", status.code()))
    }
}

fn gsettings_get(schema: &str, key: &str) -> Result<String, String> {
    let out = std::process::Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn parse_gvariant_strv(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with("@as") || trimmed == "[]" {
        return Vec::new();
    }
    trimmed
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|s| s.trim().trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
