use crate::config::{Config, RecordingFormat};
use gtk4::{self as gtk, prelude::*};
use libadwaita::{self as adw, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

pub fn append_to(notebook: &gtk::Notebook, config: &Rc<RefCell<Config>>) {
    let page = build_page(config);
    let label = gtk::Label::new(Some("Recording"));
    notebook.append_page(&page, Some(&label));
}

fn build_page(config: &Rc<RefCell<Config>>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    // --- Output group: format, output path, filename prefix ---
    let output_group = adw::PreferencesGroup::builder()
        .title("Output")
        .description("How recordings are encoded and named")
        .build();

    let format_model = gtk::StringList::new(&["MP4", "WebM"]);
    let format_row = adw::ComboRow::builder()
        .title("Default format")
        .model(&format_model)
        .build();
    let initial_format = match config.borrow().recording.format {
        RecordingFormat::Mp4 => 0u32,
        RecordingFormat::Webm => 1u32,
    };
    format_row.set_selected(initial_format);
    {
        let cfg = config.clone();
        format_row.connect_selected_notify(move |row| {
            cfg.borrow_mut().recording.format = if row.selected() == 0 {
                RecordingFormat::Mp4
            } else {
                RecordingFormat::Webm
            };
        });
    }

    // Output path row (custom child: Entry + Browse button, wrapped as a PreferencesGroup child)
    let path_row = gtk::Box::new(gtk::Orientation::Vertical, 6);
    path_row.set_margin_top(8);
    path_row.set_margin_bottom(8);
    path_row.set_margin_start(12);
    path_row.set_margin_end(12);
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
    path_row.append(&path_lbl);
    path_row.append(&hbox);

    let prefix_row = adw::EntryRow::builder()
        .title("Filename prefix")
        .text(config.borrow().recording.prefix.as_str())
        .build();
    {
        let cfg = config.clone();
        prefix_row.connect_changed(move |row| {
            cfg.borrow_mut().recording.prefix = row.text().to_string();
        });
    }

    output_group.add(&format_row);
    output_group.add(&path_row);
    output_group.add(&prefix_row);
    page.add(&output_group);

    // --- Cursor Highlight group: highlight toggle, color, radius ---
    let highlight_group = adw::PreferencesGroup::builder()
        .title("Cursor Highlight")
        .build();

    let highlight_cursor = config.borrow().recording.highlight_cursor;

    let hl_row = adw::SwitchRow::builder()
        .title("Highlight cursor")
        .subtitle("Draw a highlight around the mouse cursor during recording")
        .active(highlight_cursor)
        .build();
    {
        let cfg = config.clone();
        hl_row.connect_active_notify(move |row| {
            cfg.borrow_mut().recording.highlight_cursor = row.is_active();
        });
    }

    let color_row = adw::EntryRow::builder()
        .title("Highlight color (hex)")
        .text(config.borrow().recording.highlight_color.as_str())
        .sensitive(highlight_cursor)
        .build();
    {
        let cfg = config.clone();
        color_row.connect_changed(move |row| {
            cfg.borrow_mut().recording.highlight_color = row.text().to_string();
        });
    }

    let radius_adjustment = gtk::Adjustment::new(
        config.borrow().recording.highlight_radius as f64,
        10.0,
        100.0,
        5.0,
        5.0,
        0.0,
    );
    let radius_row = adw::SpinRow::builder()
        .title("Highlight radius (px)")
        .subtitle("Size of the cursor highlight circle")
        .adjustment(&radius_adjustment)
        .sensitive(highlight_cursor)
        .build();
    {
        let cfg = config.clone();
        radius_row.connect_value_notify(move |row| {
            cfg.borrow_mut().recording.highlight_radius = row.value() as u32;
        });
    }

    // Wire highlight switch to color/radius sensitivity
    {
        let color_row_ref = color_row.clone();
        let radius_row_ref = radius_row.clone();
        hl_row.connect_active_notify(move |row| {
            let active = row.is_active();
            color_row_ref.set_sensitive(active);
            radius_row_ref.set_sensitive(active);
        });
    }

    highlight_group.add(&hl_row);
    highlight_group.add(&color_row);
    highlight_group.add(&radius_row);
    page.add(&highlight_group);

    page
}
