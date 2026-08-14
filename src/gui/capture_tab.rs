use crate::config::{CaptureFormat, Config};
use gtk4::{self as gtk, prelude::*};
use libadwaita::{self as adw, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

pub fn append_to(notebook: &gtk::Notebook, config: &Rc<RefCell<Config>>) {
    let page = build_page(config);
    let label = gtk::Label::new(Some("Capture"));
    notebook.append_page(&page, Some(&label));
}

fn build_page(config: &Rc<RefCell<Config>>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    // --- Output group: format, quality, filename prefix ---
    let output_group = adw::PreferencesGroup::builder()
        .title("Output")
        .description("How captured screenshots are encoded and named")
        .build();

    let format_model = gtk::StringList::new(&["PNG", "JPEG"]);
    let format_row = adw::ComboRow::builder()
        .title("Default format")
        .model(&format_model)
        .build();
    let initial_format = match config.borrow().capture.format {
        CaptureFormat::Png => 0u32,
        CaptureFormat::Jpeg => 1u32,
    };
    format_row.set_selected(initial_format);

    let quality_adjustment = gtk::Adjustment::new(
        config.borrow().capture.jpeg_quality as f64,
        1.0,
        100.0,
        1.0,
        10.0,
        0.0,
    );
    let quality_row = adw::SpinRow::builder()
        .title("JPEG quality")
        .subtitle("Only used when the format above is JPEG")
        .adjustment(&quality_adjustment)
        .sensitive(matches!(config.borrow().capture.format, CaptureFormat::Jpeg))
        .build();

    {
        let cfg = config.clone();
        let quality_row = quality_row.clone();
        format_row.connect_selected_notify(move |row| {
            let is_jpeg = row.selected() == 1;
            quality_row.set_sensitive(is_jpeg);
            cfg.borrow_mut().capture.format = if is_jpeg { CaptureFormat::Jpeg } else { CaptureFormat::Png };
        });
    }
    {
        let cfg = config.clone();
        quality_row.connect_value_notify(move |row| {
            cfg.borrow_mut().capture.jpeg_quality = row.value() as u8;
        });
    }

    let prefix_row = adw::EntryRow::builder()
        .title("Filename prefix")
        .text(config.borrow().capture.prefix.as_str())
        .build();
    {
        let cfg = config.clone();
        prefix_row.connect_changed(move |row| {
            cfg.borrow_mut().capture.prefix = row.text().to_string();
        });
    }

    output_group.add(&format_row);
    output_group.add(&quality_row);
    output_group.add(&prefix_row);
    page.add(&output_group);

    // --- Behavior group: clipboard, cursor ---
    let behavior_group = adw::PreferencesGroup::builder()
        .title("Behavior")
        .build();

    let clipboard_row = adw::SwitchRow::builder()
        .title("Copy to clipboard")
        .subtitle("Copy every screenshot to the clipboard automatically")
        .active(config.borrow().capture.copy_to_clipboard)
        .build();
    {
        let cfg = config.clone();
        clipboard_row.connect_active_notify(move |row| {
            cfg.borrow_mut().capture.copy_to_clipboard = row.is_active();
        });
    }

    let cursor_row = adw::SwitchRow::builder()
        .title("Include cursor")
        .subtitle("Show the mouse cursor in captured screenshots")
        .active(config.borrow().capture.include_cursor)
        .build();
    {
        let cfg = config.clone();
        cursor_row.connect_active_notify(move |row| {
            cfg.borrow_mut().capture.include_cursor = row.is_active();
        });
    }

    behavior_group.add(&clipboard_row);
    behavior_group.add(&cursor_row);
    page.add(&behavior_group);

    // --- Preview group: post-capture preview window ---
    let preview_group = adw::PreferencesGroup::builder()
        .title("Preview")
        .description("A small floating preview after each screenshot")
        .build();

    let preview_row = adw::SwitchRow::builder()
        .title("Show preview after capture")
        .subtitle("Thumbnail with Copy/Open/Close actions, instead of just a notification")
        .active(config.borrow().capture.show_preview)
        .build();

    let auto_close_adjustment = gtk::Adjustment::new(
        config.borrow().capture.preview_auto_close_seconds as f64,
        0.0,
        60.0,
        1.0,
        5.0,
        0.0,
    );
    let auto_close_row = adw::SpinRow::builder()
        .title("Auto-close after (seconds)")
        .subtitle("0 disables auto-close; hovering the preview pauses the countdown")
        .adjustment(&auto_close_adjustment)
        .sensitive(config.borrow().capture.show_preview)
        .build();

    {
        let cfg = config.clone();
        let auto_close_row = auto_close_row.clone();
        preview_row.connect_active_notify(move |row| {
            let active = row.is_active();
            cfg.borrow_mut().capture.show_preview = active;
            auto_close_row.set_sensitive(active);
        });
    }
    {
        let cfg = config.clone();
        auto_close_row.connect_value_notify(move |row| {
            cfg.borrow_mut().capture.preview_auto_close_seconds = row.value() as u32;
        });
    }

    preview_group.add(&preview_row);
    preview_group.add(&auto_close_row);
    page.add(&preview_group);

    page
}
