use gtk::gdk::Display;
use gtk::{prelude::*, Orientation, gio, gdk, Align, Label, Button, Picture, CssProvider, STYLE_PROVIDER_PRIORITY_APPLICATION, Image};
use gtk::{glib, Application, ApplicationWindow};

const APP_ID: &str = "org.hooya.hooya_gtk";

fn main() -> glib::ExitCode {
    let application = Application::builder().application_id(APP_ID).build();
    application.connect_activate(|app| {
        let provider = CssProvider::new();
        provider.load_from_data(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &Display::default().expect("Could not connect to a display."),
            &provider,
            STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        build_browse_window(&app)
    });
    application.run()
}

fn build_browse_window(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Browse HooYa!")
        .default_width(800)
        .default_height(800)
        .build();

    let texture_container = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::Start)
        .spacing(0) // Smash all images together
        .valign(Align::Start)
        .build();
    let file = gio::File::for_path("/home/wesl-ee/img/hooya/store/pf74py/bafkreidamyljxqvgsugnn6l6tdgthoplckhyb5rvxbcucrk2hlsmpf74py");
    let asset_paintable = gdk::Texture::from_file(&file).unwrap();
    let sample_image = Picture::builder()
        .paintable(&asset_paintable)
        .width_request(400)
        .height_request(300)
        .valign(Align::Start)
        .build();
    texture_container.append(&sample_image);

    let v_box = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_top(10)
        .build();
    window.set_child(Some(&v_box));

    // Header
    let h_box_head = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .margin_start(10)
        .margin_end(10)
        .build();
    v_box.append(&h_box_head);

    let h_box_search_button = Button::builder()
        .icon_name("system-search")
        .build();
    let h_box_text = Label::builder()
        .label("Browsing — All Files")
        .halign(Align::Start)
        .hexpand(true)
        .build();
    h_box_head.append(&h_box_text);
    h_box_head.append(&h_box_search_button);
    // let test_button = gtk::Button::builder()
    //     .label("Rip and tear!")
    //     .build();
    let h_box_browse = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .build();
    h_box_browse.set_child(Some(&texture_container));
    v_box.append(&h_box_browse);

    let footer_peer_download_from_count_button = build_footer_peer_download_from_element();
    let footer_peer_upload_to_count_button = build_footer_peer_upload_to_element();
    let footer_favorites_count_button = build_footer_favorites_element();
    let footer_public_count_button = build_footer_public_element();

    let h_box_pagination = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .build();
    v_box.append(&h_box_pagination);
    let h_box_footer = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();
    h_box_footer.append(&footer_peer_download_from_count_button);
    h_box_footer.append(&footer_peer_upload_to_count_button);
    h_box_footer.append(&footer_favorites_count_button);
    h_box_footer.append(&footer_public_count_button);
    h_box_footer.add_css_class("footer");
    v_box.append(&h_box_footer);

    window.present();
}

// TODO Subclass this
fn build_footer_peer_upload_to_element() -> gtk::Box {
    let footer_peer_count_button = gtk::Box::builder()
        .spacing(3)
        .has_tooltip(true)
        .tooltip_text("Peers who made requests of local node within last 15 minutes")
        .build();
    let footer_peer_count_txt = Label::builder()
        .label("50")
        .build();
    let footer_peer_count_icon = Image::builder()
        .icon_name("network-transmit")
        .build();
    footer_peer_count_button.append(&footer_peer_count_icon);
    footer_peer_count_button.append(&footer_peer_count_txt);

    footer_peer_count_button
}

// TODO Subclass this
fn build_footer_peer_download_from_element() -> gtk::Box {
    let footer_peer_count_button = gtk::Box::builder()
        .spacing(3)
        .has_tooltip(true)
        .tooltip_text("Peers who answered local node requests within last 15 minutes")
        .build();
    let footer_peer_count_txt = Label::builder()
        .label("100")
        .build();
    let footer_peer_count_icon = Image::builder()
        .icon_name("network-receive")
        .build();
    footer_peer_count_button.append(&footer_peer_count_icon);
    footer_peer_count_button.append(&footer_peer_count_txt);

    footer_peer_count_button
}

// TODO Subclass this
fn build_footer_favorites_element() -> gtk::Box {
    let footer_favorites_count_button = gtk::Box::builder()
        .spacing(3)
        .has_tooltip(true)
        .tooltip_text("Favorites")
        .build();
    let footer_favorites_count_txt = Label::builder()
        .label("12,154")
        .build();
    let footer_favorites_count_icon = Image::builder()
        .icon_name("starred")
        .build();
    footer_favorites_count_button.append(&footer_favorites_count_icon);
    footer_favorites_count_button.append(&footer_favorites_count_txt);

    footer_favorites_count_button
}

// TODO Subclass this
fn build_footer_public_element() -> gtk::Box {
    let footer_public_count_button = gtk::Box::builder()
        .spacing(3)
        .has_tooltip(true)
        .tooltip_text("Local files made public to HooYa! network peers")
        .build();
    let footer_public_count_txt = Label::builder()
        .label("12,154")
        .build();
    let footer_public_count_icon = Image::builder()
        .icon_name("security-high")
        .build();
    footer_public_count_button.append(&footer_public_count_icon);
    footer_public_count_button.append(&footer_public_count_txt);

    footer_public_count_button
}
