use clap::{command, Arg};
use dotenv::dotenv;
use gtk::gdk::{Display, Texture};
use gtk::gdk_pixbuf::PixbufLoader;
use gtk::glib::clone;
use gtk::pango::EllipsizeMode;
use gtk::{
    glib, Application, ApplicationWindow, ContentFit, Entry, FlowBox,
    GestureClick, ScrolledWindow, SelectionMode,
};
use gtk::{
    prelude::*, Align, Button, CssProvider, Image, Label, Orientation, Picture,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use hooya::proto::control_client::ControlClient;
use hooya::proto::{ContentAtCidRequest, LocalFilePageRequest, TagsRequest};
use mason_grid_layout::MasonGridLayout;
use std::collections::HashMap;
use std::pin::Pin;
use std::str::FromStr;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc::{Sender, UnboundedSender};
use tokio_stream::{Stream, StreamExt};
use tonic::transport::{Channel, Endpoint};

// TODO Share with CLI client
mod config;

mod mason_grid_layout;

struct IncomingImage {
    chunk: Vec<u8>,
}

enum UiEvent {
    GridItemClicked { file: hooya::proto::File },
}

enum DataEvent {
    AppendImageToGrid {
        file: hooya::proto::File,
        stream: Pin<Box<dyn Stream<Item = IncomingImage> + Send>>,
    },
    ViewImage {
        file: hooya::proto::File,
        tags: HashMap<String, Vec<String>>,
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

    let empty: Vec<String> = vec![];
    application.run_with_args(&empty)
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
            let mut client_2 = client_1.clone();

            let j_1 = rt.spawn(clone!(@strong data_event_sender => async move {
                let rand_files = client_1
                    .local_file_page(LocalFilePageRequest {
                        page_size: 50,
                        page_token: "0".to_string(),
                        oldest_first: false,
                    }).await
                    .unwrap()
                    .into_inner()
                    .file;

                for file in rand_files {
                    let stream = request_data_at_cid(client_1.clone(), file.cid.clone())
                        .await;
                    data_event_sender
                        .send(DataEvent::AppendImageToGrid { file, stream: Box::pin(stream) })
                        .await
                        .unwrap();
                }
            }));
            let j_2 = rt.spawn(clone!(@strong data_event_sender => async move {
                while let Some(event) = ui_event_receiver.recv().await {
                    match event {
                        UiEvent::GridItemClicked { file } => {
                            let tags_resp = client_2.tags(TagsRequest {
                                cid: file.cid.clone()
                                }).await.unwrap().into_inner().tags;
                            let tags = tags_vec_to_map(tags_resp);
                            let stream = Box::pin(
                                request_data_at_cid(client_2.clone(), file.cid.clone())
                                .await);
                            data_event_sender
                                .send(DataEvent::ViewImage { file, tags, stream })
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
        .layout_manager(&MasonGridLayout::default())
        .name("view-grid")
        .build();
    let h_box_browse = gtk::ScrolledWindow::builder().vexpand(true).build();
    h_box_browse.set_child(Some(&m_grid));
    v_box.append(&h_box_browse);

    let h_box_footer = build_footer();

    let h_box_pagination = build_page_nav(1, 20);
    v_box.append(&h_box_pagination);

    v_box.append(&h_box_footer);

    let c = glib::MainContext::default();

    // Network event receiver
    c.spawn_local(clone!(@weak app => async move {
        while let Some(event) = data_event_receiver.recv().await {
            match event {
                DataEvent::AppendImageToGrid { file, mut stream } => {
                    let pb_loader = PixbufLoader::new();
                    let img = Picture::builder()
                        .content_fit(ContentFit::Fill)
                        .focusable(true)
                        .can_focus(true)
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

                    gesture.connect_pressed(clone!(@strong ui_event_sender, @strong img => move |_, n, _, _| {
                        // Bootleg focus, probably subclass this later to
                        // handle things like pressing "enter" while focused
                        if n == 1 {
                            // Single-click
                            img.grab_focus();
                        }
                        if n == 2 {
                            // Double-click
                            ui_event_sender.send(UiEvent::GridItemClicked { file: file.clone() })
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
                DataEvent::ViewImage { file, tags, stream } => {
                    build_file_view_window(&app, file, tags, stream)
                        .await;
                }
            }
        }
    }));

    window.present();
    data_event_sender
}

fn build_footer() -> gtk::Box {
    let footer_peer_download_from_count_button =
        build_footer_peer_download_from_element();
    let footer_peer_upload_to_count_button =
        build_footer_peer_upload_to_element();
    let footer_favorites_count_button = build_footer_favorites_element();
    let footer_public_count_button = build_footer_public_element();

    let h_box_footer = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .name("footer")
        .build();
    h_box_footer.append(&footer_peer_download_from_count_button);
    h_box_footer.append(&footer_peer_upload_to_count_button);
    h_box_footer.append(&footer_favorites_count_button);
    h_box_footer.append(&footer_public_count_button);

    h_box_footer
}

async fn build_file_view_window(
    app: &Application,
    file: hooya::proto::File,
    tags: HashMap<String, Vec<String>>,
    mut stream: Pin<Box<dyn Stream<Item = IncomingImage> + Send>>,
) {
    let window = ApplicationWindow::new(app);
    window.present();

    let pb_loader = PixbufLoader::new();

    let v_box = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .build();

    let main_box = gtk::Box::builder()
        .name("single-image-view")
        .vexpand(true)
        .build();

    // TODO Don't construct this here but hold one per app and pass by
    // reference because the values will be real-time, not static as they
    // are now
    let footer = build_footer();

    let detail_box = gtk::Box::builder()
        .margin_start(10)
        .margin_end(10)
        .spacing(20)
        .orientation(Orientation::Vertical)
        .build();

    let tags_box = FlowBox::builder()
        .column_spacing(10)
        .column_spacing(10)
        .selection_mode(SelectionMode::None)
        .orientation(Orientation::Horizontal)
        .build();

    for (namespace, descriptors) in tags {
        let namespace_box = gtk::Box::builder()
            .halign(Align::Start)
            .orientation(Orientation::Vertical)
            .build();
        let namespace_subheading = Label::builder()
            .label(namespace.clone())
            .halign(Align::Start)
            .css_classes(["subhead"])
            .build();
        let tag_box = FlowBox::builder()
            .orientation(Orientation::Horizontal)
            .selection_mode(SelectionMode::None)
            .build();
        for d in descriptors {
            let d_box = gtk::Box::builder()
                .css_classes([
                    &format!("namespace-{}", namespace),
                    "descriptor-box",
                ])
                .halign(Align::Start)
                .build();

            let d_info = Label::builder().label("?").build();

            let d_label = Label::builder().label(d).build();

            d_box.append(&d_info);
            d_box.append(&d_label);
            tag_box.append(&d_box);
        }

        namespace_box.append(&namespace_subheading);
        namespace_box.append(&tag_box);
        tags_box.append(&namespace_box);
    }

    let new_tag_box = gtk::Box::builder().build();
    let new_tag_button = gtk::Button::builder().label("Edit tags").build();
    new_tag_box.append(&new_tag_button);
    tags_box.append(&new_tag_box);

    let net_info_box = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .build();

    net_info_box.append(
        &Label::builder()
            .label("Net Info")
            .halign(Align::Start)
            .css_classes(["subhead"])
            .build(),
    );

    let file_info_box = gtk::Box::builder()
        .orientation(Orientation::Vertical)
        .build();

    file_info_box.append(
        &Label::builder()
            .label("File Info")
            .halign(Align::Start)
            .css_classes(["subhead"])
            .build(),
    );

    let mut props = vec![];
    props.push(("CID", hooya::cid::encode(file.cid.clone()), true));
    if let Some(m) = file.mimetype {
        props.push(("Mimetype", m, false));
    }
    props.push(("Size", human_readable_size(file.size), false));

    // TODO Sample data until I write calculate this info server-side
    props.push(("Dimensions", "1396x2500 (3.7 MPixel)".to_string(), false));

    for p in props {
        let row = gtk::Box::builder().spacing(10).build();

        let label_box = gtk::Box::builder().css_classes(["info-label"]).build();

        let info_label = Label::builder().label("?").build();
        let label = Label::builder().label(p.0).build();

        let val = Label::builder().wrap(true).label(p.1).build();

        if p.0 == "CID" {
            // This element is the longest and norm,ally sets the
            // container width
            val.set_ellipsize(EllipsizeMode::Middle);
            val.set_width_chars(30)
        }

        if p.2 {
            val.set_css_classes(&["clickable"]);
        }

        label_box.append(&info_label);
        label_box.append(&label);
        row.append(&label_box);
        row.append(&val);
        file_info_box.append(&row);
    }

    let sample_net_props = [
        ("Uploader", "wesl-ee.eth", true),
        ("Mimetype", "None", false),
        ("Date", "6 hours ago", false),
        ("Favorites", "20", false),
        ("Duplication", "3 peers", true),
        ("Rating", "Safe", false),
        ("Source", "pixiv.net/artworks/109539168", true),
    ];

    for p in sample_net_props {
        let row = gtk::Box::builder().spacing(10).build();

        let label_box = gtk::Box::builder().css_classes(["info-label"]).build();

        let info_label = Label::builder().label("?").build();
        let label = Label::builder().label(p.0).build();

        let val = Label::builder().wrap(true).label(p.1).build();

        if p.2 {
            val.set_css_classes(&["clickable"]);
        }

        label_box.append(&info_label);
        label_box.append(&label);
        row.append(&label_box);
        row.append(&val);
        net_info_box.append(&row);
    }

    let img = Picture::builder().build();

    detail_box.append(&tags_box);
    detail_box.append(&file_info_box);
    detail_box.append(&net_info_box);
    let scroll_detail_window = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&detail_box)
        .build();

    main_box.append(&img);
    main_box.append(&scroll_detail_window);
    v_box.append(&main_box);
    v_box.append(&footer);
    window.set_child(Some(&v_box));

    pb_loader.connect_area_prepared(clone!(@strong window => move |pb| {
        let pixbuf = pb.pixbuf().unwrap();
        img.set_paintable(Some(&Texture::for_pixbuf(&pixbuf)));
        // TODO Remember width, height if user has adjusted just to reduce
        // UX friction
        let (req_width, req_height) = clamp_dimensions(pixbuf.width(), pixbuf.height(),
            500, 500);

        img.set_height_request(req_height);
        img.set_width_request(req_width);

        if req_width > req_height {
            main_box.set_css_classes(&["vertical-stack"]);
            detail_box.set_hexpand(true);
            tags_box.set_vexpand(true);
            main_box.set_orientation(Orientation::Vertical);
            // scroll_detail_window.set_height_request(req_height);
            // window.set_default_height(req_height * 2);
            scroll_detail_window.set_min_content_height(500.min(req_height));

        } else {
            v_box.set_css_classes(&["horizontal-stack"]);
            detail_box.set_vexpand(true);
            tags_box.set_hexpand(true);
            main_box.set_orientation(Orientation::Horizontal);
            // window.set_default_width(700.max(req_width * 2))
            //
            // window.connect_default_width_notify(clone!(@strong main_box, @strong scroll_detail_window => move |w| {
                // let (pref, _) = w.preferred_size();
                // if w.default_width() < pref.width() {
                //     w.set_default_size(req_width, req_height);
                //     main_box.remove(&scroll_detail_window);
                // }
            // }));

        }
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

// TODO Custom widget would be better
// fn build_palette_box(colors: Vec<gdk::RGBA>) -> gtk::Box {
//     let ret = gtk::Box::builder().build();
//
//     let mut row = gtk::Box::builder().build();
//
//     for (i, c) in colors.iter().enumerate() {
//         if i % 4 == 0 {
//             ret.append(&row);
//             row = gtk::Box::builder().build();
//         }
//     }
//
//     ret.append(&row);
//
//     ret
// }

fn tags_vec_to_map(
    tags: Vec<hooya::proto::Tag>,
) -> HashMap<String, Vec<String>> {
    tags.iter().fold(HashMap::new(), |mut acc, t| {
        let mut exist_namespace =
            acc.get(&t.namespace).unwrap_or(&vec![]).to_owned();
        exist_namespace.push(t.descriptor.clone());
        acc.insert(t.namespace.clone(), exist_namespace);
        acc
    })
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

fn human_readable_size(size: i64) -> String {
    const SIZE_TRANSLATION: [(i64, &str); 4] =
        [(4, "TiB"), (3, "GiB"), (2, "MiB"), (1, "KiB")];

    for (power, label) in SIZE_TRANSLATION {
        let dividend: i64 = 1 << (10 * power);
        let div_res = size as f64 / dividend as f64;
        if div_res.floor() >= 1.0 {
            return format!("{:.2}{}", div_res, label);
        }
    }

    format!("{}B", size)
}
