use crate::autostart;
use crate::config::Config;
use gtk4::{self as gtk, prelude::*};
use libadwaita::{self as adw, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

pub fn append_to(
    notebook: &gtk::Notebook,
    config: &Rc<RefCell<Config>>,
    minimize: Option<Rc<dyn Fn()>>,
) {
    let page = build_page(config, minimize);
    let label = gtk::Label::new(Some("General"));
    notebook.append_page(&page, Some(&label));
}

fn build_page(config: &Rc<RefCell<Config>>, minimize: Option<Rc<dyn Fn()>>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    // --- Storage group: screenshot and video destination folders ---
    let storage_group = adw::PreferencesGroup::builder()
        .title("Storage")
        .description("Where captured screenshots and recordings are saved")
        .build();

    storage_group.add(&path_row(
        "Screenshots path",
        &config.borrow().paths.screenshots.clone(),
        {
            let cfg = config.clone();
            move |val| cfg.borrow_mut().paths.screenshots = val
        },
    ));

    storage_group.add(&path_row(
        "Videos path",
        &config.borrow().paths.videos.clone(),
        {
            let cfg = config.clone();
            move |val| cfg.borrow_mut().paths.videos = val
        },
    ));

    page.add(&storage_group);

    // --- Startup group: tray autostart and quick minimize action ---
    let startup_group = adw::PreferencesGroup::builder().title("Startup").build();

    startup_group.add(&autostart_row(config));

    // "Minimize to tray" action, placed right under the autostart toggle.
    if let Some(minimize) = minimize {
        let btn = gtk::Button::with_label("Minimize to tray");
        btn.set_halign(gtk::Align::End);
        btn.set_margin_top(6);
        btn.set_margin_bottom(6);
        btn.set_tooltip_text(Some(
            "Hide this window to the system tray (minimizes normally if the tray icon isn't available)",
        ));
        btn.connect_clicked(move |_| minimize());
        startup_group.add(&btn);
    }

    page.add(&startup_group);

    page
}

/// Switch to enable/disable launching the tray icon on login.
///
/// The autostart `.desktop` file is written/removed immediately on toggle (so
/// the change takes effect even without pressing Save); the config bool is kept
/// in sync and persisted on Save.
fn autostart_row(config: &Rc<RefCell<Config>>) -> adw::SwitchRow {
    let enabled = autostart::is_enabled();
    config.borrow_mut().tray.autostart = enabled;

    let row = adw::SwitchRow::builder()
        .title("Start tray icon on login")
        .subtitle("Quick-capture menu in the system tray")
        .active(enabled)
        .build();

    let cfg = config.clone();
    row.connect_active_notify(move |row| {
        let state = row.is_active();
        let result = if state { autostart::enable() } else { autostart::disable() };
        match result {
            Ok(()) => {
                cfg.borrow_mut().tray.autostart = state;
            }
            Err(e) => {
                eprintln!("Failed to update autostart: {}", e);
                // Revert the switch to the real state on failure.
                row.set_active(autostart::is_enabled());
            }
        }
    });

    row
}

fn path_row<F: Fn(String) + 'static>(label: &str, initial: &str, on_change: F) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
    row.set_margin_top(8);
    row.set_margin_bottom(8);
    row.set_margin_start(12);
    row.set_margin_end(12);

    let lbl = gtk::Label::builder()
        .label(label)
        .halign(gtk::Align::Start)
        .css_classes(["caption", "dim-label"])
        .build();

    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    let entry = gtk::Entry::builder()
        .text(initial)
        .hexpand(true)
        .build();

    let browse_btn = gtk::Button::with_label("Browse");

    entry.connect_changed(move |e| on_change(e.text().to_string()));

    let entry_clone = entry.clone();
    browse_btn.connect_clicked(move |btn| {
        let entry_ref = entry_clone.clone();
        if let Some(root) = btn.root() {
            if let Ok(win) = root.downcast::<gtk4::Window>() {
                let dialog = gtk4::FileChooserDialog::new(
                    Some("Choose folder"),
                    Some(&win),
                    gtk4::FileChooserAction::SelectFolder,
                    &[
                        ("Cancel", gtk4::ResponseType::Cancel),
                        ("Select", gtk4::ResponseType::Accept),
                    ],
                );
                let entry_ref2 = entry_ref.clone();
                dialog.connect_response(move |d, response| {
                    if response == gtk4::ResponseType::Accept {
                        if let Some(file) = d.file() {
                            if let Some(path) = file.path() {
                                entry_ref2.set_text(&path.to_string_lossy());
                            }
                        }
                    }
                    d.close();
                });
                dialog.show();
            }
        }
    });

    hbox.append(&entry);
    hbox.append(&browse_btn);
    row.append(&lbl);
    row.append(&hbox);
    row
}
