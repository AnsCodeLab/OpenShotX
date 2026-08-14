use crate::config::Config;
use crate::hotkeys::{self, Desktop, hotkey_display, tiling_snippet};
use gtk4::{self as gtk, prelude::*};
use libadwaita::{self as adw, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

pub fn append_to(notebook: &gtk::Notebook, config: &Rc<RefCell<Config>>) {
    let page = build_page(config);
    let label = gtk::Label::new(Some("Hotkeys"));
    notebook.append_page(&page, Some(&label));
}

fn build_page(config: &Rc<RefCell<Config>>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    let desktop = hotkeys::detect_desktop();

    // DE info, shown as the shortcuts group's description
    let de_name = match &desktop {
        Desktop::Gnome    => "GNOME",
        Desktop::Kde      => "KDE",
        Desktop::Xfce     => "XFCE",
        Desktop::Sway     => "Sway",
        Desktop::I3       => "i3",
        Desktop::Hyprland => "Hyprland",
        Desktop::Unknown  => "Unknown",
    };

    let shortcuts_group = adw::PreferencesGroup::builder()
        .title("Shortcuts")
        .description(format!("Detected desktop: {}", de_name))
        .build();

    // Action definitions: (display name, getter fn, setter fn)
    struct Action {
        name: &'static str,
        get: Box<dyn Fn(&Config) -> String>,
        set: Box<dyn Fn(&mut Config, String)>,
    }
    let actions: Vec<Action> = vec![
        Action {
            name: "Capture area",
            get: Box::new(|c| c.hotkeys.capture_area.clone()),
            set: Box::new(|c, v| c.hotkeys.capture_area = v),
        },
        Action {
            name: "Capture screen",
            get: Box::new(|c| c.hotkeys.capture_screen.clone()),
            set: Box::new(|c, v| c.hotkeys.capture_screen = v),
        },
        Action {
            name: "Capture window",
            get: Box::new(|c| c.hotkeys.capture_window.clone()),
            set: Box::new(|c, v| c.hotkeys.capture_window = v),
        },
        Action {
            name: "Record area",
            get: Box::new(|c| c.hotkeys.record_area.clone()),
            set: Box::new(|c, v| c.hotkeys.record_area = v),
        },
        Action {
            name: "Record screen",
            get: Box::new(|c| c.hotkeys.record_screen.clone()),
            set: Box::new(|c, v| c.hotkeys.record_screen = v),
        },
    ];

    // Collect (row, default_binding_display) for the reset button
    let mut shortcut_entries: Vec<(adw::EntryRow, String)> = Vec::new();

    for action in actions.into_iter() {
        let initial = hotkey_display(&(action.get)(&config.borrow()));
        let default_val = hotkey_display(&(action.get)(&crate::config::Config::default()));

        let row = adw::EntryRow::builder()
            .title(action.name)
            .text(&initial)
            .build();

        {
            let cfg = config.clone();
            row.connect_changed(move |r| {
                let raw = display_to_binding(&r.text());
                (action.set)(&mut cfg.borrow_mut(), raw);
            });
        }

        shortcut_entries.push((row.clone(), default_val));
        shortcuts_group.add(&row);
    }
    page.add(&shortcuts_group);

    // Footer with Reset + Register button
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.set_halign(gtk::Align::End);
    footer.set_margin_top(8);
    footer.set_margin_bottom(8);
    footer.set_margin_start(8);
    footer.set_margin_end(8);

    let reset_btn = gtk::Button::with_label("Reset defaults");
    {
        let cfg = config.clone();
        let entries = shortcut_entries;
        reset_btn.connect_clicked(move |_| {
            cfg.borrow_mut().hotkeys = crate::config::HotkeysConfig::default();
            for (entry, default_text) in &entries {
                entry.set_text(default_text);
            }
        });
    }
    footer.append(&reset_btn);

    // DE-specific register button.
    // Match on &desktop so we can still move the owned value into closures below.
    match &desktop {
        Desktop::Gnome => {
            let btn = gtk::Button::builder()
                .label("Register in GNOME")
                .css_classes(["suggested-action"])
                .build();
            let cfg = config.clone();
            btn.connect_clicked(move |_| {
                if let Err(e) = hotkeys::register_gnome(&cfg.borrow().hotkeys) {
                    eprintln!("GNOME registration failed: {}", e);
                }
            });
            footer.append(&btn);
        }
        Desktop::Kde => {
            let btn = gtk::Button::builder()
                .label("Register in KDE")
                .css_classes(["suggested-action"])
                .build();
            let cfg = config.clone();
            btn.connect_clicked(move |_| {
                if let Err(e) = hotkeys::register_kde(&cfg.borrow().hotkeys) {
                    eprintln!("KDE registration failed: {}", e);
                }
            });
            footer.append(&btn);
        }
        Desktop::Xfce => {
            let btn = gtk::Button::builder()
                .label("Register in XFCE")
                .css_classes(["suggested-action"])
                .build();
            let cfg = config.clone();
            btn.connect_clicked(move |_| {
                if let Err(e) = hotkeys::register_xfce(&cfg.borrow().hotkeys) {
                    eprintln!("XFCE registration failed: {}", e);
                }
            });
            footer.append(&btn);
        }
        Desktop::Sway | Desktop::I3 | Desktop::Hyprland => {
            let btn = gtk::Button::builder()
                .label("Copy config snippet")
                .css_classes(["suggested-action"])
                .build();
            let cfg = config.clone();
            // Reconstruct an owned Desktop value for the closure, since `desktop`
            // is only matched by reference here.
            let desktop_owned = match &desktop {
                Desktop::Sway     => Desktop::Sway,
                Desktop::I3       => Desktop::I3,
                Desktop::Hyprland => Desktop::Hyprland,
                // Unreachable: this arm is only entered for the three variants above.
                other => unreachable!("unexpected desktop variant: {:?}", other),
            };
            btn.connect_clicked(move |_| {
                let snippet = tiling_snippet(&cfg.borrow().hotkeys, &desktop_owned);
                if let Some(display) = gtk4::gdk::Display::default() {
                    display.clipboard().set_text(&snippet);
                } else {
                    eprintln!("Config snippet:\n{}", snippet);
                }
            });
            footer.append(&btn);
        }
        Desktop::Unknown => {
            let note = gtk::Label::builder()
                .label("Set these shortcuts manually in your desktop environment.")
                .css_classes(["caption", "dim-label"])
                .wrap(true)
                .build();
            footer.append(&note);
        }
    }

    let footer_group = adw::PreferencesGroup::builder().build();
    footer_group.add(&footer);
    page.add(&footer_group);

    page
}

/// Converts display format "Super+Shift+4" back to gsettings format "<Super><Shift>4".
fn display_to_binding(display: &str) -> String {
    if !display.contains('+') {
        return display.to_string();
    }
    let parts: Vec<&str> = display.split('+').collect();
    let (modifiers, key) = parts.split_at(parts.len() - 1);
    let mods: String = modifiers.iter().map(|m| format!("<{}>", m)).collect();
    format!("{}{}", mods, key[0])
}
