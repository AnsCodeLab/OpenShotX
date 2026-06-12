use crate::config::Config;
use gtk4::{self as gtk, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

pub fn append_to(notebook: &gtk::Notebook, config: &Rc<RefCell<Config>>) {
    let page = build_page(config);
    let label = gtk::Label::new(Some("General"));
    notebook.append_page(&page, Some(&label));
}

fn build_page(config: &Rc<RefCell<Config>>) -> gtk::ScrolledWindow {
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

    gtk::ScrolledWindow::builder()
        .child(&vbox)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
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
