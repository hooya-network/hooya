use gtk::{glib, Application};
use gtk::{prelude::*, ApplicationWindow};

const APP_ID: &str = "org.hooya.hooya_gtk";

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("HooYa!")
        .build();

    window.present();
}
