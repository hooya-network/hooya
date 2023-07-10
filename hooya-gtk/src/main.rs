use clap::{command, Arg};
use dotenv::dotenv;
use gtk::gdk::{Cursor, Display, Texture};
use gtk::gdk_pixbuf::PixbufLoader;
use gtk::glib::clone;
use gtk::{
    glib, Application, ApplicationWindow, ContentFit, Entry, GestureClick,
};
use gtk::{
    prelude::*, Align, Button, CssProvider, Image, Label, Orientation, Picture,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use hooya::proto::control_client::ControlClient;
use hooya::proto::{ContentAtCidRequest, RandomLocalCidRequest};
use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_stream::{Stream, StreamExt};
use tonic::transport::Channel;

// TODO Share with CLI client
mod config;

mod mason_grid_layout;

struct IncomingImage {
    chunk: Vec<u8>,
}

enum UiEvent {}

enum DataEvent {
    AppendImageToGrid {
        cid: Vec<u8>,
        stream: Pin<Box<dyn Stream<Item = IncomingImage> + Send>>,
    },
}

const APP_ID: &str = "org.hooya.hooya_gtk";

fn main() -> glib::ExitCode {
    dotenv().ok();
    let matches = command!()
        .arg(
            Arg::new("endpoint")
                .long("endpoint")
                .env("HOOYAD_ENDPOINT")
                .default_value(config::DEFAULT_HOOYAD_ENDPOINT),
        )
        .get_matches();

    let application = Application::builder().application_id(APP_ID).build();

    // thread-to-thread communication
    let (ui_event_sender, mut ui_event_receiver) =
        tokio::sync::mpsc::channel(1);
    let (data_event_sender, data_event_receiver) =
        tokio::sync::mpsc::channel(1);

    let data_event_receiver = Rc::new(RefCell::new(Some(data_event_receiver)));
    application.connect_activate(move |app| {
        let provider = CssProvider::new();
        provider.load_from_data(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &Display::default().expect("Could not connect to a display."),
            &provider,
            STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        build_browse_window(
            app,
            ui_event_sender.clone(),
            data_event_receiver.clone(),
        )
    });

    thread::spawn(move || {
        use tokio::runtime::Runtime;
        let rt = Runtime::new().expect("create tokio runtime");

        rt.block_on(async {
            let mut client: ControlClient<Channel> =
                ControlClient::connect(format!(
                    "http://{}",
                    matches.get_one::<String>("endpoint").unwrap()
                ))
                .await
                .expect("Connect to hooyad"); // TODO UI for this

            let rand_cids = client
                .random_local_cid(RandomLocalCidRequest { count: 100 })
                .await
                .unwrap()
                .into_inner()
                .cid;

            for cid in rand_cids {
                let resp = client
                    .content_at_cid(ContentAtCidRequest { cid: cid.clone() })
                    .await
                    .unwrap();

                let inner_resp = resp.into_inner();
                let stream = Box::pin(
                    inner_resp
                        .map(move |c| {
                            let chunk = c.unwrap().data;
                            IncomingImage { chunk }
                        })
                        // Minimal delay to allow GUI to maybe update during stream
                        .throttle(Duration::from_millis(20)),
                );
                data_event_sender
                    .send(DataEvent::AppendImageToGrid { cid, stream })
                    .await
                    .unwrap();
            }
            while let Some(_m) = ui_event_receiver.recv().await {}
        })
    });

    application.run()
}

fn build_browse_window(
    app: &Application,
    _ui_event_sender: Sender<UiEvent>,
    data_event_receiver: Rc<RefCell<Option<Receiver<DataEvent>>>>,
) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Browse HooYa!")
        .default_width(800)
        .default_height(1600)
        .build();

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

    let h_box_search_button =
        Button::builder().icon_name("system-search").build();
    let h_box_text = Label::builder()
        .label("Browsing — All Files")
        .name("view-head")
        .halign(Align::Start)
        .hexpand(true)
        .build();
    h_box_head.append(&h_box_text);
    h_box_head.append(&h_box_search_button);
    // let test_button = gtk::Button::builder()
    //     .label("Rip and tear!")
    //     .build();
    let m_grid = gtk::Box::builder()
        .layout_manager(&mason_grid_layout::MasonGridLayout::default())
        .name("view-grid")
        .build();
    let h_box_browse = gtk::ScrolledWindow::builder().vexpand(true).build();
    h_box_browse.set_child(Some(&m_grid));
    v_box.append(&h_box_browse);

    let footer_peer_download_from_count_button =
        build_footer_peer_download_from_element();
    let footer_peer_upload_to_count_button =
        build_footer_peer_upload_to_element();
    let footer_favorites_count_button = build_footer_favorites_element();
    let footer_public_count_button = build_footer_public_element();

    let h_box_pagination = build_page_nav(1, 20);
    v_box.append(&h_box_pagination);
    let h_box_footer = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .name("footer")
        .build();
    h_box_footer.append(&footer_peer_download_from_count_button);
    h_box_footer.append(&footer_peer_upload_to_count_button);
    h_box_footer.append(&footer_favorites_count_button);
    h_box_footer.append(&footer_public_count_button);
    v_box.append(&h_box_footer);

    let c = glib::MainContext::default();

    // Network event receiver
    c.spawn_local(async move {
        let mut data_event_receiver = data_event_receiver
            .replace(None)
            .take()
            .expect("data_event_reciver");
        while let Some(event) = data_event_receiver.recv().await {
            match event {
                DataEvent::AppendImageToGrid { cid, mut stream } => {
                    let pb_loader = PixbufLoader::new();
                    let img = Picture::builder()
                        .content_fit(ContentFit::Fill)
                        .cursor(&Cursor::from_name("grab", None).unwrap())
                        .build();
                    m_grid.append(&img);
                    pb_loader.connect_area_prepared(clone!(@strong img => move |pb| {
                        let pixbuf = pb.pixbuf().unwrap();
                        img.set_paintable(Some(&Texture::for_pixbuf(&pixbuf)));
                    }));

                    while let Some(i_img) = stream.next().await {
                        let f_chunk = i_img.chunk;
                        pb_loader.write(&f_chunk).unwrap();
                    }

                    let gesture = GestureClick::builder()
                        .build();

                    gesture.connect_pressed(clone!(@strong cid => move |_, n, _, _| {
                        if n == 2 {
                            // Double-click
                            println!("Will soon open window for {}", hooya::cid::encode(cid.clone()));
                        }
                    }));

                    img.add_controller(gesture);

                    let res = pb_loader.close();
                    if let Err(e) = res {
                        m_grid.remove(&img);
                        println!("AERR {}", e)
                    }
                }
            }
        }
    });

    window.present();
}

// TODO Subclass this
fn build_footer_peer_upload_to_element() -> gtk::Box {
    let footer_peer_count_button = gtk::Box::builder()
        .spacing(3)
        .has_tooltip(true)
        .tooltip_text(
            "Peers who made requests of local node within last 15 minutes",
        )
        .build();
    let footer_peer_count_txt = Label::builder().label("50").build();
    let footer_peer_count_icon =
        Image::builder().icon_name("network-transmit").build();
    footer_peer_count_button.append(&footer_peer_count_icon);
    footer_peer_count_button.append(&footer_peer_count_txt);

    footer_peer_count_button
}

// TODO Subclass this
fn build_footer_peer_download_from_element() -> gtk::Box {
    let footer_peer_count_button = gtk::Box::builder()
        .spacing(3)
        .has_tooltip(true)
        .tooltip_text(
            "Peers who answered local node requests within last 15 minutes",
        )
        .build();
    let footer_peer_count_txt = Label::builder().label("100").build();
    let footer_peer_count_icon =
        Image::builder().icon_name("network-receive").build();
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
    let footer_favorites_count_txt = Label::builder().label("12,154").build();
    let footer_favorites_count_icon =
        Image::builder().icon_name("starred").build();
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
    let footer_public_count_txt = Label::builder().label("12,154").build();
    let footer_public_count_icon =
        Image::builder().icon_name("security-high").build();
    footer_public_count_button.append(&footer_public_count_icon);
    footer_public_count_button.append(&footer_public_count_txt);

    footer_public_count_button
}

// TODO Subclass this
fn build_page_nav(curr: u32, max: u32) -> gtk::Box {
    let ret = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(20)
        .name("page-nav")
        .build();

    let max_show_either_side = 2;
    let until = max + 1;

    // What are you trying to do
    if curr > max || curr < 1 || max < 1 {
        println!("Tried to construct page nav with illegal bounds!");
        return ret;
    }

    for i in 1..until {
        let dist = curr.abs_diff(i);
        if i == curr {
            let child = Entry::builder()
                .max_width_chars(3)
                .text(i.to_string())
                .build();
            child.set_xalign(0.5);
            ret.append(&child);
        } else if i == 1 || i == max || dist <= max_show_either_side {
            let child = Button::builder().label(i.to_string()).build();
            ret.append(&child);
        } else if dist == max_show_either_side + 1 {
            let child = Label::builder().label("…").build();
            ret.append(&child);
        }
    }

    ret
}
