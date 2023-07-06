mod imp;

use gtk::glib;

glib::wrapper! {
    pub struct MasonGridLayout(ObjectSubclass<imp::MasonGridLayout>)
        @extends gtk::LayoutManager;
}

impl Default for MasonGridLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl MasonGridLayout {
    pub fn new() -> Self {
        glib::Object::new()
    }
}
