mod imp;

use glib::Object;
use gtk::{gio, glib, Application};

glib::wrapper! {
    pub struct FileViewWindow(ObjectSubclass<imp::FileViewWindow>)
        @extends gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl FileViewWindow {
    pub fn new(app: &Application) -> Self {
        Object::builder()
            .property("application", app)
            .property("title", "HooYa! File View")
            .build()
    }
}
