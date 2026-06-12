mod general_tab;
mod capture_tab;
mod recording_tab;
mod hotkeys_tab;

use crate::config::Config;
use gtk4::{self as gtk, prelude::*};
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

pub fn run_settings(initial_config: Config) {
    let app = adw::Application::builder()
        .application_id("io.github.anscodelab.openshotx")
        .build();

    app.connect_activate(move |app| {
        let config = Rc::new(RefCell::new(initial_config.clone()));
        let window = build_window(app, config);
        window.present();
    });

    app.run_with_args::<String>(&[]);
}

fn build_window(app: &adw::Application, config: Rc<RefCell<Config>>) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("openshotx Settings")
        .default_width(560)
        .default_height(500)
        .resizable(false)
        .build();

    let header = adw::HeaderBar::new();
    let cancel_btn = gtk::Button::with_label("Cancel");
    let save_btn = gtk::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");
    header.pack_start(&cancel_btn);
    header.pack_end(&save_btn);

    let notebook = gtk::Notebook::new();
    notebook.set_show_border(false);
    notebook.set_vexpand(true);

    general_tab::append_to(&notebook, &config);
    capture_tab::append_to(&notebook, &config);
    recording_tab::append_to(&notebook, &config);
    hotkeys_tab::append_to(&notebook, &config);

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    vbox.append(&header);
    vbox.append(&notebook);
    window.set_child(Some(&vbox));

    let w = window.clone();
    cancel_btn.connect_clicked(move |_| w.close());

    let w = window.clone();
    let cfg = config.clone();
    save_btn.connect_clicked(move |_| {
        if let Err(e) = cfg.borrow().save() {
            eprintln!("Failed to save config: {}", e);
        }
        w.close();
    });

    window
}
