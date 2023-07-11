use gtk::glib;
use gtk::subclass::prelude::*;
use gtk::ApplicationWindow;

#[derive(Debug, Default)]
pub struct FileViewWindow {}

#[glib::object_subclass]
impl ObjectSubclass for FileViewWindow {
    const NAME: &'static str = "FileViewWindow";
    type Type = super::FileViewWindow;
    type ParentType = ApplicationWindow;
}

impl ObjectImpl for FileViewWindow {}
impl WidgetImpl for FileViewWindow {}
impl WindowImpl for FileViewWindow {}
impl ApplicationWindowImpl for FileViewWindow {}
