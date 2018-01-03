extern crate cairo;
extern crate gdk;
extern crate gio;
extern crate glib;
extern crate gstreamer as gst;
extern crate gstreamer_player as gst_player;
extern crate gstreamer_video as gst_video;
extern crate gtk;
#[macro_use]
extern crate lazy_static;
extern crate send_cell;

#[macro_use]
extern crate closet;

use cairo::Context as CairoContext;
use gdk::prelude::*;
use gio::prelude::*;
use gio::MenuExt;
use gio::MenuItemExt;
use glib::translate::ToGlibPtr;
use gst::prelude::*;
use gtk::prelude::*;
use send_cell::SendCell;
use std::cell::RefCell;
use std::env;
use std::os::raw::c_void;
use std::process;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;

#[derive(Clone)]
struct PlayerContext {
    player: gst_player::Player,
    renderer: gst_player::PlayerVideoOverlayVideoRenderer,
    video_area: gtk::Widget,
    has_gtkgl: bool,
}

#[derive(Clone)]
struct VideoPlayerInner {
    player_context: Option<PlayerContext>,
    window: gtk::Window,
    main_box: gtk::Box,
    fullscreen_action: gio::SimpleAction,
    restore_action: gio::SimpleAction,
    pause_action: gio::SimpleAction,
    seek_forward_action: gio::SimpleAction,
    seek_backward_action: gio::SimpleAction,
    subtitle_action: gio::SimpleAction,
    audio_track_action: gio::SimpleAction,
    pause_button: gtk::Button,
    seek_backward_button: gtk::Button,
    seek_forward_button: gtk::Button,
    fullscreen_button: gtk::Button,
    progress_bar: gtk::Scale,
    toolbar_box: gtk::Box,
    subtitle_track_menu: gio::Menu,
    audio_track_menu: gio::Menu,
}

struct VideoPlayer {
    inner: Arc<Mutex<VideoPlayerInner>>,
}

static SEEK_BACKWARD_OFFSET: u64 = 2000;
static SEEK_FORWARD_OFFSET: u64 = 5000;
enum SeekDirection {
    Backward,
    Forward,
}

lazy_static! {
    static ref INHIBIT_COOKIE: Mutex<Option<u32>> = {
        Mutex::new(None)
    };
    static ref INITIAL_POSITION: Mutex<Option<(i32, i32)>> = {
        Mutex::new(None)
    };
    static ref INITIAL_SIZE: Mutex<Option<(i32, i32)>> = {
        Mutex::new(None)
    };
}

impl PlayerContext {
    pub fn new() -> Self {
        let dispatcher = gst_player::PlayerGMainContextSignalDispatcher::new(None);
        let (sink, video_area, has_gtkgl) = if let Some(gtkglsink) = gst::ElementFactory::make("gtkglsink", None) {
            let glsinkbin = gst::ElementFactory::make("glsinkbin", None).unwrap();
            glsinkbin
                .set_property("sink", &gtkglsink.to_value())
                .unwrap();

            let widget = gtkglsink.get_property("widget").unwrap();
            (glsinkbin, widget.get::<gtk::Widget>().unwrap(), true)
        } else {
            let sink = gst::ElementFactory::make("glimagesink", None).unwrap();
            let widget = gtk::DrawingArea::new();
            (sink, widget.upcast::<gtk::Widget>(), false)
        };

        let renderer1 = gst_player::PlayerVideoOverlayVideoRenderer::new_with_sink(&sink);
        let renderer = renderer1.clone();
        let player = gst_player::Player::new(
            Some(&renderer1.upcast::<gst_player::PlayerVideoRenderer>()),
            Some(&dispatcher.upcast::<gst_player::PlayerSignalDispatcher>()),
        );

        // Get position updates every 250ms.
        let mut config = player.get_config();
        config.set_position_update_interval(250);
        player.set_config(config).unwrap();

        PlayerContext {
            player,
            renderer,
            video_area,
            has_gtkgl,
        }
    }
}

impl VideoPlayer {
    pub fn new(gtk_app: &gtk::Application) -> Self {
        let fullscreen_action = gio::SimpleAction::new_stateful("fullscreen", None, &false.to_variant());
        gtk_app.add_action(&fullscreen_action);

        let restore_action = gio::SimpleAction::new_stateful("restore", None, &true.to_variant());
        gtk_app.add_action(&restore_action);

        let pause_action = gio::SimpleAction::new_stateful("pause", None, &false.to_variant());
        gtk_app.add_action(&pause_action);

        let seek_forward_action = gio::SimpleAction::new_stateful("seek-forward", None, &false.to_variant());
        gtk_app.add_action(&seek_forward_action);

        let seek_backward_action = gio::SimpleAction::new_stateful("seek-backward", None, &false.to_variant());
        gtk_app.add_action(&seek_backward_action);

        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_default_size(320, 240);
        window.set_resizable(true);

        let main_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let toolbar_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);

        let pause_button = gtk::Button::new();
        let pause_actionable = pause_button.clone().upcast::<gtk::Actionable>();
        pause_actionable.set_action_name("app.pause");

        let seek_backward_button = gtk::Button::new();
        let seek_bw_actionable = seek_backward_button.clone().upcast::<gtk::Actionable>();
        seek_bw_actionable.set_action_name("app.seek-backward");
        let backward_image = gtk::Image::new_from_icon_name(
            "media-seek-backward-symbolic",
            gtk::IconSize::SmallToolbar.into(),
        );
        seek_backward_button.set_image(&backward_image);

        let seek_forward_button = gtk::Button::new();
        let seek_fw_actionable = seek_forward_button.clone().upcast::<gtk::Actionable>();
        seek_fw_actionable.set_action_name("app.seek-forward");
        let forward_image = gtk::Image::new_from_icon_name(
            "media-seek-forward-symbolic",
            gtk::IconSize::SmallToolbar.into(),
        );
        seek_forward_button.set_image(&forward_image);

        toolbar_box.pack_start(&seek_backward_button, false, false, 0);
        toolbar_box.pack_start(&pause_button, false, false, 0);
        toolbar_box.pack_start(&seek_forward_button, false, false, 0);

        let progress_bar = gtk::Scale::new(gtk::Orientation::Horizontal, None);
        progress_bar.set_draw_value(true);
        progress_bar.set_value_pos(gtk::PositionType::Right);

        toolbar_box.pack_start(&progress_bar, true, true, 10);

        let fullscreen_button = gtk::Button::new();
        let fullscreen_image = gtk::Image::new_from_icon_name(
            "view-fullscreen-symbolic",
            gtk::IconSize::SmallToolbar.into(),
        );
        fullscreen_button.set_image(&fullscreen_image);
        let fs_actionable = fullscreen_button.clone().upcast::<gtk::Actionable>();
        fs_actionable.set_action_name("app.fullscreen");

        toolbar_box.pack_start(&fullscreen_button, false, false, 0);

        main_box.pack_start(&toolbar_box, false, false, 10);
        window.add(&main_box);

        let subtitle_track_menu = gio::Menu::new();
        let subtitle_action = gio::SimpleAction::new_stateful(
            "subtitle",
            glib::VariantTy::new("s").unwrap(),
            &"".to_variant(),
        );
        gtk_app.add_action(&subtitle_action);

        let audio_track_menu = gio::Menu::new();
        let audio_track_action = gio::SimpleAction::new_stateful(
            "audio-track",
            glib::VariantTy::new("s").unwrap(),
            &"audio-0".to_variant(),
        );
        gtk_app.add_action(&audio_track_action);

        let video_player = VideoPlayerInner {
            player_context: None,
            window,
            main_box,
            fullscreen_action,
            restore_action,
            pause_action,
            seek_forward_action,
            seek_backward_action,
            subtitle_action,
            audio_track_action,
            seek_backward_button,
            seek_forward_button,
            pause_button,
            fullscreen_button,
            progress_bar,
            toolbar_box,
            subtitle_track_menu,
            audio_track_menu,
        };
        let inner = Arc::new(Mutex::new(video_player));

        gtk_app.connect_startup(clone_army!([inner] move |app| {
            let quit = gio::SimpleAction::new("quit", None);
            quit.connect_activate(clone_army!([app] move |_, _| {
                app.quit();
            }));
            app.add_action(&quit);

            app.set_accels_for_action("app.quit", &*vec!["<Meta>q", "<Ctrl>q"]);
            app.set_accels_for_action("app.fullscreen", &*vec!["<Meta>f", "<Alt>f"]);
            app.set_accels_for_action("app.restore", &*vec!["Escape"]);
            app.set_accels_for_action("app.pause", &*vec!["space"]);
            app.set_accels_for_action("app.seek-forward", &*vec!["<Meta>Right", "<Alt>Right"]);
            app.set_accels_for_action("app.seek-backward", &*vec!["<Meta>Left", "<Alt>Left"]);

            let menu = gio::Menu::new();
            let audio_menu = gio::Menu::new();
            let subtitles_menu = gio::Menu::new();
            menu.append("Quit", "app.quit");

            if let Ok(inner) = inner.lock() {
                subtitles_menu.append_submenu("Subtitle track", &inner.subtitle_track_menu);
                audio_menu.append_submenu("Audio track", &inner.audio_track_menu);
            }

            menu.append_submenu("Audio", &audio_menu);
            menu.append_submenu("Subtitles", &subtitles_menu);

            app.set_menubar(&menu);
        }));

        gtk_app.connect_open(clone_army!([inner] move |app, files, _| {
                app.activate();
                if let Ok(mut inner) = inner.lock() {
                    inner.open_files(files);
                }
            }));

        gtk_app.connect_shutdown(clone_army!([inner] move |_| {
                if let Ok(inner) = inner.lock() {
                    inner.stop_player();
                }
            }));

        if let Ok(inner) = inner.lock() {
            inner
                .fullscreen_action
                .connect_change_state(clone_army!([inner, gtk_app] move |_, _| {
                inner.enter_fullscreen(&gtk_app);
            }));

            inner
                .restore_action
                .connect_change_state(clone_army!([inner, gtk_app] move |_, _| {
                inner.leave_fullscreen(&gtk_app);
            }));

            inner
                .window
                .connect_delete_event(clone_army!([gtk_app] move |_, _| {
                    gtk_app.quit();
                    Inhibit(false)
                }));

            inner.window.connect_map_event(move |widget, _| {
                if let Ok(size) = INITIAL_SIZE.lock() {
                    if let Some((width, height)) = *size {
                        widget.resize(width, height);
                    }
                }
                if let Ok(position) = INITIAL_POSITION.lock() {
                    if let Some((x, y)) = *position {
                        widget.move_(x, y);
                    }
                }
                Inhibit(false)
            });
        }

        VideoPlayer { inner }
    }

    pub fn start(&self, app: &gtk::Application) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.player_context = Some(PlayerContext::new());

            inner.setup(app);

            if let Some(ref ctx) = inner.player_context {
                let video_area = &ctx.video_area;
                inner.main_box.pack_start(&*video_area, true, true, 0);
                inner.main_box.reorder_child(&*video_area, 0);
                video_area.show();

                inner
                    .progress_bar
                    .connect_format_value(clone_army!([ctx] move |_, _| -> std::string::String {
                    let position = ctx.player.get_position();
                    format!("{:.0}", position)
                }));

                video_area.connect_realize(clone_army!([inner] move |_| {
                        inner.prepare_video_overlay();
                    }));

                video_area.connect_draw(clone_army!([inner] move |_, cairo_context| {
                        inner.draw_video_area(cairo_context);
                        Inhibit(false)
                    }));

                video_area.connect_configure_event(clone_army!([inner] move |_, event| -> bool {
                        inner.resize_video_area(event);
                        true
                    }));
            }

            inner
                .pause_action
                .connect_change_state(clone_army!([inner] move |_, _| {
                inner.toggle_pause();
            }));

            inner
                .seek_forward_action
                .connect_change_state(clone_army!([inner] move |_, _| {
                    inner.seek(&SeekDirection::Forward, SEEK_FORWARD_OFFSET);
                }));

            inner
                .seek_backward_action
                .connect_change_state(clone_army!([inner] move |_, _| {
                    inner.seek(&SeekDirection::Backward, SEEK_BACKWARD_OFFSET);
                }));

            inner
                .subtitle_action
                .connect_change_state(clone_army!([inner] move |action, value| {
                    if let Some(val) = value.clone() {
                        if let Some(idx) = val.get::<std::string::String>() {
                            let (_prefix, idx) = idx.split_at(4);
                            let idx = idx.parse::<i32>().unwrap();
                            if let Some(ref ctx) = inner.player_context {
                                ctx.player.set_subtitle_track_enabled(idx > -1);
                                if idx >= 0 {
                                    ctx.player.set_subtitle_track(idx).unwrap();
                                }
                                action.set_state(&val);
                            }
                        }
                    }
                }));

            inner
                .audio_track_action
                .connect_change_state(clone_army!([inner] move |action, value| {
                    if let Some(val) = value.clone() {
                        if let Some(idx) = val.get::<std::string::String>() {
                            let (_prefix, idx) = idx.split_at(6);
                            let idx = idx.parse::<i32>().unwrap();
                            if let Some(ref ctx) = inner.player_context {
                                ctx.player.set_audio_track_enabled(idx > -1);
                                if idx >= 0 {
                                    ctx.player.set_audio_track(idx).unwrap();
                                }
                                action.set_state(&val);
                            }
                        }
                    }
                }));

            inner.start(app);
        }
    }
}

impl VideoPlayerInner {
    pub fn setup(&self, gtk_app: &gtk::Application) {
        if let Some(ref ctx) = self.player_context {
            let video_area = &ctx.video_area;
            let video_area_clone = SendCell::new(video_area.clone());
            ctx.player
                .connect_video_dimensions_changed(move |_, width, height| {
                    let video_area = video_area_clone.borrow();
                    video_area.set_size_request(width, height);
                });

            let file_list = Arc::new(Mutex::new(vec![]));
            let inner = SendCell::new(self.clone());
            let window_clone = SendCell::new(self.window.clone());
            ctx.player
                .connect_media_info_updated(clone_army!([file_list, inner] move |_, info| {
                let uri = info.get_uri();
                let mut file_list = file_list.lock().unwrap();
                // Call this only once per asset.
                if !&file_list.contains(&uri) {
                    file_list.push(uri.clone());
                    let window = window_clone.borrow();
                    if let Some(title) = info.get_title() {
                        window.set_title(&*title);
                    } else {
                        window.set_title(&*info.get_uri());
                    }

                    let inner = inner.borrow();
                    inner.fill_subtitle_track_menu(info);
                    inner.fill_audio_track_menu(info);
                }
            }));

            let pause_button_clone = SendCell::new(self.pause_button.clone());
            ctx.player.connect_state_changed(move |_, state| {
                let pause_button = pause_button_clone.borrow();
                match state {
                    gst_player::PlayerState::Paused => {
                        let image = gtk::Image::new_from_icon_name(
                            "media-playback-start-symbolic",
                            gtk::IconSize::SmallToolbar.into(),
                        );
                        pause_button.set_image(&image);
                    },
                    gst_player::PlayerState::Playing => {
                        let image = gtk::Image::new_from_icon_name(
                            "media-playback-pause-symbolic",
                            gtk::IconSize::SmallToolbar.into(),
                        );
                        pause_button.set_image(&image);
                    },
                    _ => {},
                };
            });

            let range = self.progress_bar.clone().upcast::<gtk::Range>();
            let player = &ctx.player;
            let seek_signal_handler_id = range.connect_value_changed(clone_army!([player] move |range| {
                let value = range.get_value();
                player.seek(gst::ClockTime::from_seconds(value as u64));
            }));

            let progress_bar_clone = SendCell::new(self.progress_bar.clone());
            let signal_handler_id = Arc::new(Mutex::new(seek_signal_handler_id));
            ctx.player
                .connect_duration_changed(clone_army!([signal_handler_id] move |_, duration| {
                let progress_bar = progress_bar_clone.borrow();
                let range = progress_bar.clone().upcast::<gtk::Range>();
                let seek_signal_handler_id = signal_handler_id.lock().unwrap();
                glib::signal_handler_block(&range, &seek_signal_handler_id);
                range.set_range(0.0, duration.seconds().unwrap() as f64);
                glib::signal_handler_unblock(&range, &seek_signal_handler_id);
            }));

            let progress_bar_clone = SendCell::new(self.progress_bar.clone());
            ctx.player
                .connect_position_updated(clone_army!([signal_handler_id] move |_, position| {
                let progress_bar = progress_bar_clone.borrow();
                let range = progress_bar.clone().upcast::<gtk::Range>();
                let seek_signal_handler_id = signal_handler_id.lock().unwrap();
                glib::signal_handler_block(&range, &seek_signal_handler_id);
                range.set_value(position.seconds().unwrap() as f64);
                glib::signal_handler_unblock(&range, &seek_signal_handler_id);
            }));

            let app_clone = SendCell::new(gtk_app.clone());
            ctx.player.connect_error(move |_, error| {
                // FIXME: display some GTK error dialog...
                eprintln!("Error! {}", error);
                let app = &app_clone.borrow();
                app.quit();
            });
        }
    }

    pub fn start(&mut self, app: &gtk::Application) {
        self.window.show_all();
        app.add_window(&self.window);
    }

    pub fn stop_player(&self) {
        if let Some(ref ctx) = self.player_context {
            ctx.player.stop();
        }
    }

    pub fn seek(&self, direction: &SeekDirection, offset: u64) {
        if let Some(ref ctx) = self.player_context {
            let player = &ctx.player;
            let position = player.get_position();
            let offset = gst::ClockTime::from_mseconds(offset);
            let destination = match *direction {
                SeekDirection::Backward => {
                    if position >= offset {
                        Some(position - offset)
                    } else {
                        None
                    }
                },
                SeekDirection::Forward => {
                    let duration = player.get_duration();
                    if duration != gst::ClockTime::none() && position + offset <= duration {
                        Some(position + offset)
                    } else {
                        None
                    }
                },
            };
            if let Some(destination) = destination {
                player.seek(destination);
            }
        }
    }

    pub fn toggle_pause(&self) {
        if let Some(ref ctx) = self.player_context {
            let pause_action = &self.pause_action;
            let player = &ctx.player;
            if let Some(is_paused) = pause_action.get_state() {
                let paused = is_paused.get::<bool>().unwrap();
                if paused {
                    player.play();
                } else {
                    player.pause();
                }
                pause_action.set_state(&(!paused).to_variant());
            }
        }
    }

    pub fn enter_fullscreen(&self, app: &gtk::Application) {
        let fullscreen_action = &self.fullscreen_action;
        if let Some(is_fullscreen) = fullscreen_action.get_state() {
            let fullscreen = is_fullscreen.get::<bool>().unwrap();
            if !fullscreen {
                let window = &self.window;
                let gdk_window = window.get_window().unwrap();
                let flags = gtk::ApplicationInhibitFlags::SUSPEND | gtk::ApplicationInhibitFlags::IDLE;
                *INHIBIT_COOKIE.lock().unwrap() = Some(app.inhibit(window, flags, None));
                *INITIAL_SIZE.lock().unwrap() = Some(window.get_size());
                *INITIAL_POSITION.lock().unwrap() = Some(window.get_position());
                let cursor = gdk::Cursor::new(gdk::CursorType::BlankCursor);
                window.fullscreen();
                gdk_window.set_cursor(Some(&cursor));
                self.toolbar_box.set_visible(false);
                fullscreen_action.set_state(&true.to_variant());
            }
        }
    }

    pub fn leave_fullscreen(&self, app: &gtk::Application) {
        let fullscreen_action = &self.fullscreen_action;
        if let Some(is_fullscreen) = fullscreen_action.get_state() {
            let fullscreen = is_fullscreen.get::<bool>().unwrap();

            if fullscreen {
                let window = &self.window;
                let gdk_window = window.get_window().unwrap();
                if let Ok(mut cookie) = INHIBIT_COOKIE.lock() {
                    app.uninhibit(cookie.unwrap());
                    *cookie = None;
                }
                window.unfullscreen();
                self.toolbar_box.set_visible(true);
                window.present();
                gdk_window.set_cursor(None);
                fullscreen_action.set_state(&false.to_variant());
            }
        }
    }

    pub fn prepare_video_overlay(&self) {
        if let Some(ref ctx) = self.player_context {
            let video_window = &ctx.video_area;
            let gdk_window = video_window.get_window().unwrap();
            let video_overlay = &ctx.renderer;
            if !gdk_window.ensure_native() {
                println!("Can't create native window for widget");
                process::exit(-1);
            }

            let display_type_name = gdk_window.get_display().get_type().name();

            // Check if we're using X11 or ...
            if cfg!(target_os = "linux") {
                if !ctx.has_gtkgl {
                    // Check if we're using X11 or ...
                    if display_type_name == "GdkX11Display" {
                        extern "C" {
                            pub fn gdk_x11_window_get_xid(window: *mut glib::object::GObject) -> *mut c_void;
                        }

                        unsafe {
                            let xid = gdk_x11_window_get_xid(gdk_window.to_glib_none().0);
                            video_overlay.set_window_handle(xid as usize);
                        }
                    } else {
                        println!("Add support for display type '{}'", display_type_name);
                        process::exit(-1);
                    }
                }
            } else if cfg!(target_os = "macos") {
                if display_type_name == "GdkQuartzDisplay" {
                    extern "C" {
                        pub fn gdk_quartz_window_get_nsview(window: *mut glib::object::GObject) -> *mut c_void;
                    }

                    unsafe {
                        let window = gdk_quartz_window_get_nsview(gdk_window.to_glib_none().0);
                        video_overlay.set_window_handle(window as usize);
                    }
                } else {
                    println!("Unsupported display type '{}", display_type_name);
                    process::exit(-1);
                }
            }
        }
    }

    fn draw_video_area(&self, cairo_context: &CairoContext) {
        if let Some(ref ctx) = self.player_context {
            let video_window = &ctx.video_area;
            let width = video_window.get_allocated_width();
            let height = video_window.get_allocated_height();

            // Paint some black borders
            cairo_context.rectangle(0., 0., f64::from(width), f64::from(height));
            cairo_context.fill();
        }
    }

    fn resize_video_area(&self, event: &gdk::EventConfigure) {
        if let Some(ref ctx) = self.player_context {
            let video_overlay = &ctx.renderer;
            let (width, height) = event.get_size();
            let (x, y) = event.get_position();

            let player = &ctx.player;
            if let Ok(video_track) = player.get_property("current-video-track") {
                if let Some(video_track) = video_track.get::<gst_player::PlayerVideoInfo>() {
                    let video_width = video_track.get_width();
                    let video_height = video_track.get_height();
                    let src_rect = gst_video::VideoRectangle::new(0, 0, video_width, video_height);

                    let dst_rect = gst_video::VideoRectangle::new(x, y, width as i32, height as i32);
                    let rect = gst_video::center_video_rectangle(&src_rect, &dst_rect, true);
                    video_overlay.set_render_rectangle(rect.x, rect.y, rect.w, rect.h);
                    video_overlay.expose();
                    let video_window = &ctx.video_area;
                    video_window.queue_draw();
                }
            }
        }
    }

    pub fn fill_subtitle_track_menu(&self, info: &gst_player::PlayerMediaInfo) {
        let mut i = 0;
        let section = gio::Menu::new();

        let item = gio::MenuItem::new(&*"Disable", &*"subtitle");
        item.set_detailed_action("app.subtitle::sub--1");
        section.append_item(&item);

        for sub_stream in info.get_subtitle_streams() {
            if let Some(lang) = sub_stream.get_language() {
                let action_id = format!("app.subtitle::sub-{}", i);
                let item = gio::MenuItem::new(&*lang, &*action_id);
                item.set_detailed_action(&*action_id);
                section.append_item(&item);
                i += 1;
            }
        }
        self.subtitle_track_menu.append_section(None, &section);
        self.subtitle_action.change_state(&("sub--1").to_variant());
    }

    pub fn fill_audio_track_menu(&self, info: &gst_player::PlayerMediaInfo) {
        let mut i = 0;
        let section = gio::Menu::new();

        let item = gio::MenuItem::new(&*"Disable", &*"subtitle");
        item.set_detailed_action("app.audio-track::audio--1");
        section.append_item(&item);

        for audio_stream in info.get_audio_streams() {
            if let Some(lang) = audio_stream.get_language() {
                let action_id = format!("app.audio-track::audio-{}", i);
                let lang = format!("{} {} channels", lang, audio_stream.get_channels());
                let item = gio::MenuItem::new(&*lang, &*action_id);
                item.set_detailed_action(&*action_id);
                section.append_item(&item);
                i += 1;
            }
        }
        self.audio_track_menu.append_section(None, &section);
    }

    pub fn play_uri(&self, uri: &str) {
        if let Some(ref ctx) = self.player_context {
            let player = &ctx.player;

            player.connect_uri_loaded(move |player, _| {
                player.play();
            });

            player
                .set_property("uri", &glib::Value::from(&uri))
                .unwrap();
        }
    }

    pub fn open_files(&mut self, files: &[gio::File]) {
        let mut playlist = vec![];
        for file in files.to_vec() {
            if let Some(uri) = file.get_uri() {
                playlist.push(std::string::String::from(uri.as_str()));
            }
        }

        assert!(!files.is_empty());
        self.play_uri(&*playlist[0]);

        let inner_clone = SendCell::new(self.clone());
        let index_cell = RefCell::new(AtomicUsize::new(0));
        if let Some(ref ctx) = self.player_context {
            let player = &ctx.player;
            player.connect_end_of_stream(move |_| {
                let mut cell = index_cell.borrow_mut();
                let index = cell.get_mut();
                *index += 1;
                if *index < playlist.len() {
                    let inner_clone = inner_clone.borrow();
                    inner_clone.play_uri(&*playlist[*index]);
                }
                // TODO: else quit?
            });
        }
    }
}

fn main() {
    #[cfg(not(unix))]
    {
        println!("Add support for target platform");
        process::exit(-1);
    }

    #[cfg(target_os = "linux")]
    {
        // FIXME: We should somehow detect at runtime if we're running under a
        // Wayland compositor and thus don't call this.
        extern "C" {
            pub fn XInitThreads() -> c_void;
        }

        unsafe {
            XInitThreads();
        }
    }

    gst::init().expect("Failed to initialize GStreamer.");
    gtk::init().expect("Failed to initialize GTK.");

    let gtk_app = gtk::Application::new("net.base-art.glide", gio::ApplicationFlags::HANDLES_OPEN)
        .expect("Application initialization failed");

    if let Some(settings) = gtk::Settings::get_default() {
        settings
            .set_property("gtk-application-prefer-dark-theme", &true)
            .unwrap();
    }

    let app = VideoPlayer::new(&gtk_app);
    gtk_app.connect_activate(move |gtk_app| {
        app.start(gtk_app);
    });

    let args = env::args().collect::<Vec<_>>();
    gtk_app.run(&args);
}
