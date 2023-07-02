use gtk::prelude::*;
use gtk::{glib, Application, ApplicationWindow, Frame};

const APP_ID: &str = "org.hooya.hooya_gtk";

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Browse HooYa!")
        .default_width(800)
        .default_height(600)
        .build();

    let frame = Frame::builder()
        .label("All")
        .margin_bottom(10)
        .margin_top(10)
        .margin_start(10)
        .margin_end(10)
        .build();

    let texture_container = gtk::Box::builder().build();

    let sample_image = gtk::Image::builder().pixel_size(1000).build();

    texture_container.append(&sample_image);

    let scroll_window = gtk::ScrolledWindow::builder().build();

    let mason_layout = gtk::Grid::builder()
        // .row_spacing(6)
        // .column_spacing(6)
        .build();

    mason_layout.attach(&texture_container, 0, 0, 1, 1);

    scroll_window.set_child(Some(&mason_layout));

    frame.set_child(Some(&scroll_window));

    window.set_child(Some(&frame));

    window.present();
}
