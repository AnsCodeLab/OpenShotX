use crate::config::{Config, RecordingFormat};
use gtk4::{self as gtk, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

pub fn append_to(notebook: &gtk::Notebook, config: &Rc<RefCell<Config>>) {
    let page = build_page(config);
    let label = gtk::Label::new(Some("Recording"));
    notebook.append_page(&page, Some(&label));
}

fn build_page(config: &Rc<RefCell<Config>>) -> gtk::ScrolledWindow {
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 12);
    vbox.set_margin_top(20);
    vbox.set_margin_bottom(20);
    vbox.set_margin_start(20);
    vbox.set_margin_end(20);

    // Format dropdown: MP4 or WebM
    let fmt_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let fmt_lbl = gtk::Label::builder()
        .label("Default format")
        .hexpand(true)
        .halign(gtk::Align::Start)
        .build();
    let fmt_combo = gtk::DropDown::from_strings(&["MP4", "WebM"]);
    let initial = match config.borrow().recording.format {
        RecordingFormat::Mp4 => 0u32,
        RecordingFormat::Webm => 1u32,
    };
    fmt_combo.set_selected(initial);
    {
        let cfg = config.clone();
        fmt_combo.connect_selected_notify(move |dd| {
            cfg.borrow_mut().recording.format = if dd.selected() == 0 {
                RecordingFormat::Mp4
            } else {
                RecordingFormat::Webm
            };
        });
    }
    fmt_box.append(&fmt_lbl);
    fmt_box.append(&fmt_combo);
    vbox.append(&fmt_box);

    // Output path
    let path_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let path_lbl = gtk::Label::builder()
        .label("Output path")
        .halign(gtk::Align::Start)
        .css_classes(["caption", "dim-label"])
        .build();
    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::Entry::builder()
        .text(config.borrow().recording.output.as_str())
        .hexpand(true)
        .build();
    let browse_btn = gtk::Button::with_label("Browse");

    {
        let cfg = config.clone();
        entry.connect_changed(move |e| {
            cfg.borrow_mut().recording.output = e.text().to_string();
        });
    }
    {
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
    }
    hbox.append(&entry);
    hbox.append(&browse_btn);
    path_box.append(&path_lbl);
    path_box.append(&hbox);
    vbox.append(&path_box);

    // Prefix row
    let prefix_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let prefix_lbl = gtk::Label::builder()
        .label("Filename prefix")
        .hexpand(true)
        .halign(gtk::Align::Start)
        .build();
    let prefix_entry = gtk::Entry::builder()
        .text(config.borrow().recording.prefix.as_str())
        .build();
    {
        let cfg = config.clone();
        prefix_entry.connect_changed(move |e| {
            cfg.borrow_mut().recording.prefix = e.text().to_string();
        });
    }
    prefix_box.append(&prefix_lbl);
    prefix_box.append(&prefix_entry);
    vbox.append(&prefix_box);

    // Highlight cursor row
    let hl_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let hl_lbl = gtk::Label::builder()
        .label("Highlight cursor")
        .hexpand(true)
        .halign(gtk::Align::Start)
        .build();
    let hl_switch = gtk::Switch::new();
    hl_switch.set_active(config.borrow().recording.highlight_cursor);
    {
        let cfg = config.clone();
        hl_switch.connect_active_notify(move |sw| {
            cfg.borrow_mut().recording.highlight_cursor = sw.is_active();
        });
    }
    hl_box.append(&hl_lbl);
    hl_box.append(&hl_switch);
    vbox.append(&hl_box);

    // Highlight color row
    let color_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let color_lbl = gtk::Label::builder()
        .label("Highlight color (hex)")
        .hexpand(true)
        .halign(gtk::Align::Start)
        .build();
    let color_entry = gtk::Entry::builder()
        .text(config.borrow().recording.highlight_color.as_str())
        .sensitive(config.borrow().recording.highlight_cursor)
        .build();
    {
        let cfg = config.clone();
        color_entry.connect_changed(move |e| {
            cfg.borrow_mut().recording.highlight_color = e.text().to_string();
        });
    }
    color_box.append(&color_lbl);
    color_box.append(&color_entry);
    vbox.append(&color_box);

    // Highlight radius row
    let radius_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let radius_lbl = gtk::Label::builder()
        .label("Highlight radius (px)")
        .hexpand(true)
        .halign(gtk::Align::Start)
        .build();
    let radius_spin = gtk::SpinButton::with_range(10.0, 100.0, 5.0);
    radius_spin.set_value(config.borrow().recording.highlight_radius as f64);
    radius_spin.set_sensitive(config.borrow().recording.highlight_cursor);
    {
        let cfg = config.clone();
        radius_spin.connect_value_changed(move |sb| {
            cfg.borrow_mut().recording.highlight_radius = sb.value() as u32;
        });
    }
    radius_box.append(&radius_lbl);
    radius_box.append(&radius_spin);
    vbox.append(&radius_box);

    // Wire highlight switch to color/radius sensitivity
    {
        let color_entry_ref = color_entry.clone();
        let radius_spin_ref = radius_spin.clone();
        hl_switch.connect_active_notify(move |sw| {
            let active = sw.is_active();
            color_entry_ref.set_sensitive(active);
            radius_spin_ref.set_sensitive(active);
        });
    }

    gtk::ScrolledWindow::builder()
        .child(&vbox)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
}
