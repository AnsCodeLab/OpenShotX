use crate::config::Config;
use gtk4::{self as gtk, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

pub fn append_to(notebook: &gtk::Notebook, _config: &Rc<RefCell<Config>>) {
    let label = gtk::Label::new(Some("Recording"));
    let page = gtk::Label::new(Some("Recording settings (coming soon)"));
    notebook.append_page(&page, Some(&label));
}
