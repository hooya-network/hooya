use std::cell::RefCell;

use gtk::{glib, Settings};
use gtk::subclass::prelude::*;
use gtk::ApplicationWindow;

#[derive(Debug, Default)]
pub struct BrowseGridWindow {
}

#[glib::object_subclass]
impl ObjectSubclass for BrowseGridWindow {
    const NAME: &'static str = "BrowseGridWindow";
    type Type = super::BrowseGridWindow;
    type ParentType = ApplicationWindow;
}

impl ObjectImpl for BrowseGridWindow {
    fn constructed(&self) {
        self.parent_constructed();
        let obj = self.obj();
        obj.a();
    }
}

impl WidgetImpl for BrowseGridWindow {}
impl WindowImpl for BrowseGridWindow { }
impl ApplicationWindowImpl for BrowseGridWindow { }
impl BrowseGridWindow {
}
