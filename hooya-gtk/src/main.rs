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
use std::pin::Pin;
use std::str::FromStr;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc::{Sender, UnboundedSender};
use tokio_stream::{Stream, StreamExt};
use tonic::transport::{Channel, Endpoint};

// TODO Share with CLI client
mod config;

mod file_view_window;
mod mason_grid_layout;

struct IncomingImage {
    chunk: Vec<u8>,
}

enum UiEvent {
    GridItemClicked { cid: Vec<u8> },
}

enum DataEvent {
    AppendImageToGrid {
        cid: Vec<u8>,
        stream: Pin<Box<dyn Stream<Item = IncomingImage> + Send>>,
    },
    ViewImage {
        _cid: Vec<u8>,
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

    application.connect_activate(move |app| {
        let provider = CssProvider::new();
        provider.load_from_data(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &Display::default().expect("Could not connect to a display."),
            &provider,
            STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let endpoint = Endpoint::from_str(&format!(
            "http://{}",
            matches.get_one::<String>("endpoint").unwrap()
        ))
        .unwrap();

        build_ui(app, endpoint);
    });

    application.run()
}

fn build_ui(app: &Application, endpoint: Endpoint) {
    // Unbounded because this is called from buttons in the MainContext thread
    // and therefore .send() does not block
    let (ui_event_sender, mut ui_event_receiver) =
        tokio::sync::mpsc::unbounded_channel::<UiEvent>();

    let data_event_sender = build_browse_window(app, ui_event_sender);
    thread::spawn(move || {
        use tokio::runtime::Runtime;
        let rt = Runtime::new().expect("create tokio runtime");
        rt.block_on(async {
            let mut client_1: ControlClient<Channel> =
                ControlClient::connect(endpoint)
                    .await
                    .expect("Connect to hooyad"); // TODO UI for this
            let client_2 = client_1.clone();

            let j_1 = rt.spawn(clone!(@strong data_event_sender => async move {
                let rand_cids = client_1
                    .random_local_cid(RandomLocalCidRequest { count: 20 })
                    .await
                    .unwrap()
                    .into_inner()
                    .cid;

                for cid in rand_cids {
                    let stream = request_data_at_cid(client_1.clone(), cid.clone())
                        .await;
                    data_event_sender
                        .send(DataEvent::AppendImageToGrid { cid, stream: Box::pin(stream) })
                        .await
                        .unwrap();
                }
            }));
            let j_2 = rt.spawn(clone!(@strong data_event_sender => async move {
                while let Some(event) = ui_event_receiver.recv().await {
                    match event {
                        UiEvent::GridItemClicked { cid } => {
                            let stream = Box::pin(
                                request_data_at_cid(client_2.clone(), cid.clone())
                                .await);
                            data_event_sender
                                .send(DataEvent::ViewImage { _cid: cid, stream })
                                .await
                                .unwrap();
                        }
                    }
                }
            }));
            let (res, _) = tokio::join!(j_1, j_2);
            res.unwrap();
        });
    });
}

async fn request_data_at_cid(
    mut client: ControlClient<Channel>,
    cid: Vec<u8>,
) -> impl Stream<Item = IncomingImage> {
    let resp_res = client
        .content_at_cid(ContentAtCidRequest { cid: cid.clone() })
        .await;
    let inner_resp = resp_res.unwrap().into_inner();

    Box::pin(
        inner_resp
            // Minimal delay to allow GUI to maybe update during stream
            .throttle(Duration::from_millis(10))
            .map(move |c| {
                let chunk = c.unwrap().data;
                IncomingImage { chunk }
            }),
    )
}

fn build_browse_window(
    app: &Application,
    ui_event_sender: UnboundedSender<UiEvent>,
) -> Sender<DataEvent> {
    // Bounded channel because otherwise we hammer the UI with data before it is
    // done processing it.
    //
    // BUG Increasing this channel size by a factor of 10 has caused a hang in
    // the .recv() method in the MainContext when sending > 100 large images.
    // Not sure why and frankly I don't care rn
    let (data_event_sender, mut data_event_receiver) =
        tokio::sync::mpsc::channel(1);

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
    c.spawn_local(clone!(@weak app => async move {
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

                    gesture.connect_pressed(clone!(@strong ui_event_sender => move |_, n, _, _| {
                        if n == 2 {
                            // Double-click
                            ui_event_sender.send(UiEvent::GridItemClicked { cid: cid.clone() })
                                .unwrap();
                        }
                    }));

                    img.add_controller(gesture);

                    let res = pb_loader.close();
                    if let Err(e) = res {
                        m_grid.remove(&img);
                        println!("AERR {}", e)
                    }
                }
                DataEvent::ViewImage { _cid, mut stream } => {
                    let window = file_view_window::FileViewWindow::new(&app);
                    window.present();

                    let pb_loader = PixbufLoader::new();
                    let img = Picture::builder()
                        .build();
                    window.set_child(Some(&img));
                    pb_loader.connect_area_prepared(clone!(@strong window, @strong img => move |pb| {
                        let pixbuf = pb.pixbuf().unwrap();
                        img.set_paintable(Some(&Texture::for_pixbuf(&pixbuf)));
                        let (req_width, req_height) = clamp_dimensions(pixbuf.width(), pixbuf.height(),
                            500, 500);
                        window.set_size_request(req_width, req_height);
                    }));

                    while let Some(i_img) = stream.next().await {
                        let f_chunk = i_img.chunk;
                        pb_loader.write(&f_chunk).unwrap();
                    }

                    let res = pb_loader.close();
                    if let Err(e) = res {
                        window.close();
                        println!("AERR {}", e)
                    }
                }
            }
        }
    }));

    window.present();
    data_event_sender
}

fn clamp_dimensions(
    width: i32,
    height: i32,
    width_clamp: i32,
    height_clamp: i32,
) -> (i32, i32) {
    if width <= width_clamp && height <= height_clamp {
        return (width, height);
    }

    let mut ret_width = width_clamp;
    let mut ret_height = height_clamp;

    let width_clamped_height = width_clamp * height / width;
    let height_clamped_width = height_clamp * width / height;

    match height.cmp(&width) {
        std::cmp::Ordering::Greater => ret_width = height_clamped_width,
        std::cmp::Ordering::Less => ret_height = width_clamped_height,
        _ => (),
    }

    (ret_width, ret_height)
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
