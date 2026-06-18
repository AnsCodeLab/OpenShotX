use crate::autostart;
use crate::config::Config;
use gtk4::{self as gtk, prelude::*};
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

fn build_page(config: &Rc<RefCell<Config>>, minimize: Option<Rc<dyn Fn()>>) -> gtk::ScrolledWindow {
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 16);
    vbox.set_margin_top(20);
    vbox.set_margin_bottom(20);
    vbox.set_margin_start(20);
    vbox.set_margin_end(20);

    vbox.append(&path_row(
        "Screenshots path",
        &config.borrow().paths.screenshots.clone(),
        {
            let cfg = config.clone();
            move |val| cfg.borrow_mut().paths.screenshots = val
        },
    ));

    vbox.append(&path_row(
        "Videos path",
        &config.borrow().paths.videos.clone(),
        {
            let cfg = config.clone();
            move |val| cfg.borrow_mut().paths.videos = val
        },
    ));

    vbox.append(&autostart_row(config));

    // "Minimize to tray" action, placed right under the autostart toggle.
    if let Some(minimize) = minimize {
        let btn = gtk::Button::with_label("Minimize to tray");
        btn.set_halign(gtk::Align::End);
        btn.set_tooltip_text(Some(
            "Hide this window to the system tray (minimizes normally if the tray icon isn't available)",
        ));
        btn.connect_clicked(move |_| minimize());
        vbox.append(&btn);
    }

    gtk::ScrolledWindow::builder()
        .child(&vbox)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
}

/// Switch to enable/disable launching the tray icon on login.
///
/// The autostart `.desktop` file is written/removed immediately on toggle (so
/// the change takes effect even without pressing Save); the config bool is kept
/// in sync and persisted on Save.
fn autostart_row(config: &Rc<RefCell<Config>>) -> gtk::Box {
    let enabled = autostart::is_enabled();
    config.borrow_mut().tray.autostart = enabled;

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let title = gtk::Label::builder()
        .label("Start tray icon on login")
        .halign(gtk::Align::Start)
        .build();
    let subtitle = gtk::Label::builder()
        .label("Quick-capture menu in the system tray")
        .halign(gtk::Align::Start)
        .css_classes(["caption", "dim-label"])
        .build();
    labels.append(&title);
    labels.append(&subtitle);

    let switch = gtk::Switch::builder()
        .active(enabled)
        .valign(gtk::Align::Center)
        .build();

    let cfg = config.clone();
    switch.connect_state_set(move |sw, state| {
        let result = if state { autostart::enable() } else { autostart::disable() };
        match result {
            Ok(()) => {
                cfg.borrow_mut().tray.autostart = state;
                glib_propagate(false)
            }
            Err(e) => {
                eprintln!("Failed to update autostart: {}", e);
                // Revert the switch to the real state on failure.
                sw.set_active(autostart::is_enabled());
                glib_propagate(true)
            }
        }
    });

    row.append(&labels);
    row.append(&switch);
    row
}

/// `connect_state_set` expects a `glib::Propagation`; this keeps the call sites
/// readable. `true` stops further default handling.
fn glib_propagate(stop: bool) -> gtk::glib::Propagation {
    if stop {
        gtk::glib::Propagation::Stop
    } else {
        gtk::glib::Propagation::Proceed
    }
}

fn path_row<F: Fn(String) + 'static>(label: &str, initial: &str, on_change: F) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 6);

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
