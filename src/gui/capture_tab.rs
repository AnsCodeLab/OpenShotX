use crate::config::{CaptureFormat, Config};
use gtk4::{self as gtk, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

pub fn append_to(notebook: &gtk::Notebook, config: &Rc<RefCell<Config>>) {
    let page = build_page(config);
    let label = gtk::Label::new(Some("Capture"));
    notebook.append_page(&page, Some(&label));
}

fn build_page(config: &Rc<RefCell<Config>>) -> gtk::ScrolledWindow {
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 12);
    vbox.set_margin_top(20);
    vbox.set_margin_bottom(20);
    vbox.set_margin_start(20);
    vbox.set_margin_end(20);

    // Format dropdown: PNG or JPEG
    let format_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let format_lbl = gtk::Label::builder()
        .label("Default format")
        .hexpand(true)
        .halign(gtk::Align::Start)
        .build();
    let format_combo = gtk::DropDown::from_strings(&["PNG", "JPEG"]);
    let initial_format = match config.borrow().capture.format {
        CaptureFormat::Png => 0u32,
        CaptureFormat::Jpeg => 1u32,
    };
    format_combo.set_selected(initial_format);
    format_box.append(&format_lbl);
    format_box.append(&format_combo);
    vbox.append(&format_box);

    // JPEG quality slider (only enabled when JPEG selected)
    let quality_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let quality_header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let quality_lbl = gtk::Label::builder()
        .label("JPEG quality")
        .hexpand(true)
        .halign(gtk::Align::Start)
        .build();
    let quality_val_lbl = gtk::Label::new(Some(&config.borrow().capture.jpeg_quality.to_string()));
    quality_header.append(&quality_lbl);
    quality_header.append(&quality_val_lbl);

    let quality_slider = gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 100.0, 1.0);
    quality_slider.set_value(config.borrow().capture.jpeg_quality as f64);
    quality_slider.set_hexpand(true);
    quality_slider.set_draw_value(false);
    let is_jpeg = matches!(config.borrow().capture.format, CaptureFormat::Jpeg);
    quality_slider.set_sensitive(is_jpeg);

    quality_box.append(&quality_header);
    quality_box.append(&quality_slider);
    vbox.append(&quality_box);

    // Wire format → enable/disable quality slider and update config
    {
        let cfg = config.clone();
        let slider = quality_slider.clone();
        format_combo.connect_selected_notify(move |dd| {
            let is_jpeg = dd.selected() == 1;
            slider.set_sensitive(is_jpeg);
            cfg.borrow_mut().capture.format = if is_jpeg {
                CaptureFormat::Jpeg
            } else {
                CaptureFormat::Png
            };
        });
    }
    {
        let cfg = config.clone();
        let val_lbl = quality_val_lbl.clone();
        quality_slider.connect_value_changed(move |s| {
            let v = s.value() as u8;
            val_lbl.set_label(&v.to_string());
            cfg.borrow_mut().capture.jpeg_quality = v;
        });
    }

    // Filename prefix
    let prefix_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let prefix_lbl = gtk::Label::builder()
        .label("Filename prefix")
        .halign(gtk::Align::Start)
        .css_classes(["caption", "dim-label"])
        .build();
    let prefix_entry = gtk::Entry::builder()
        .text(config.borrow().capture.prefix.as_str())
        .build();
    {
        let cfg = config.clone();
        prefix_entry.connect_changed(move |e| {
            cfg.borrow_mut().capture.prefix = e.text().to_string();
        });
    }
    prefix_box.append(&prefix_lbl);
    prefix_box.append(&prefix_entry);
    vbox.append(&prefix_box);

    // Toggle: copy to clipboard
    vbox.append(&toggle_row(
        "Copy to clipboard",
        config.borrow().capture.copy_to_clipboard,
        {
            let cfg = config.clone();
            move |v| cfg.borrow_mut().capture.copy_to_clipboard = v
        },
    ));

    // Toggle: include cursor
    vbox.append(&toggle_row(
        "Include cursor",
        config.borrow().capture.include_cursor,
        {
            let cfg = config.clone();
            move |v| cfg.borrow_mut().capture.include_cursor = v
        },
    ));

    gtk::ScrolledWindow::builder()
        .child(&vbox)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
}

fn toggle_row<F: Fn(bool) + 'static>(label: &str, initial: bool, on_change: F) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let lbl = gtk::Label::builder()
        .label(label)
        .hexpand(true)
        .halign(gtk::Align::Start)
        .build();
    let sw = gtk::Switch::builder()
        .active(initial)
        .valign(gtk::Align::Center)
        .build();
    sw.connect_active_notify(move |s| on_change(s.is_active()));
    row.append(&lbl);
    row.append(&sw);
    row
}
