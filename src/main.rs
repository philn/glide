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
struct VideoPlayerInner {
    pub player: gst_player::Player,
    renderer: gst_player::PlayerVideoOverlayVideoRenderer,
    window: gtk::Window,
    video_area: gtk::DrawingArea,
    fullscreen_action: gio::SimpleAction,
    pause_action: gio::SimpleAction,
    pause_button: gtk::Button,
    seek_backward_button: gtk::Button,
    seek_forward_button: gtk::Button,
    progress_bar: gtk::Scale,
    toolbar_box: gtk::Box,
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

impl VideoPlayer {
    pub fn new(gtk_app: &gtk::Application) -> Self {
        let dispatcher = gst_player::PlayerGMainContextSignalDispatcher::new(None);
        let sink = gst::ElementFactory::make("glimagesink", None).unwrap();
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

        let fullscreen_action =
            gio::SimpleAction::new_stateful("fullscreen", None, &false.to_variant());
        gtk_app.add_action(&fullscreen_action);

        let pause_action = gio::SimpleAction::new_stateful("pause", None, &false.to_variant());
        gtk_app.add_action(&pause_action);

        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_default_size(320, 240);
        window.set_resizable(true);

        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let video_area = gtk::DrawingArea::new();
        vbox.pack_start(&video_area, true, true, 0);

        let toolbar_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);

        let pause_button = gtk::Button::new();

        let seek_backward_button = gtk::Button::new();
        let backward_image = gtk::Image::new_from_icon_name(
            "media-seek-backward-symbolic",
            gtk::IconSize::SmallToolbar.into(),
        );
        seek_backward_button.set_image(&backward_image);

        let seek_forward_button = gtk::Button::new();
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
        progress_bar.connect_format_value(
            clone_army!([player] move |_, _| -> std::string::String {
                let position = player.get_position();
                format!("{:.0}", position)
            }),
        );

        toolbar_box.pack_start(&progress_bar, true, true, 10);

        vbox.pack_start(&toolbar_box, false, false, 10);
        window.add(&vbox);

        let video_player = VideoPlayerInner {
            player,
            renderer,
            window,
            video_area,
            fullscreen_action,
            pause_action,
            seek_backward_button,
            seek_forward_button,
            pause_button,
            progress_bar,
            toolbar_box,
        };
        let inner = Arc::new(Mutex::new(video_player));

        gtk_app.connect_startup(move |app| {
            let quit = gio::SimpleAction::new("quit", None);
            quit.connect_activate(clone_army!([app] move |_, _| {
                app.quit();
            }));
            app.add_action(&quit);
        });

        gtk_app.connect_open(clone_army!([inner] move |app, files, _| {
                app.activate();
                if let Ok(mut inner) = inner.lock() {
                    inner.open_files(files);
                }
            }));

        gtk_app.connect_shutdown(clone_army!([inner] move |_| {
                if let Ok(inner) = inner.lock() {
                    inner.player.stop();
                }
            }));

        if let Ok(inner) = inner.lock() {
            inner.setup(gtk_app);
        }

        VideoPlayer { inner }
    }

    pub fn start(&self, app: &gtk::Application) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.start(app);
        }
    }
}

impl VideoPlayerInner {
    pub fn setup(&self, gtk_app: &gtk::Application) {
        {
            let self_clone = self.clone();
            self.video_area.connect_realize(move |_| {
                self_clone.prepare_video_overlay();
            });
        }

        {
            let self_clone = self.clone();
            self.video_area.connect_draw(move |_, cairo_context| {
                self_clone.draw_video_area(cairo_context);
                Inhibit(false)
            });
        }

        {
            let self_clone = self.clone();
            self.video_area
                .connect_configure_event(move |_, event| -> bool {
                    self_clone.resize_video_area(event);
                    true
                });
        }

        {
            let self_clone = self.clone();
            self.pause_button.connect_clicked(move |_| {
                self_clone.toggle_pause();
            });
        }
        {
            let self_clone = self.clone();
            self.seek_backward_button.connect_clicked(move |_| {
                self_clone.seek(&SeekDirection::Backward, SEEK_BACKWARD_OFFSET);
            });
        }
        {
            let self_clone = self.clone();
            self.seek_forward_button.connect_clicked(move |_| {
                self_clone.seek(&SeekDirection::Forward, SEEK_FORWARD_OFFSET);
            });
        }

        let self_clone = self.clone();
        self.window
            .connect_key_press_event(clone_army!([gtk_app] move |_, key| {
            let keyval = key.get_keyval();
            let keystate = key.get_state();

            if keystate.intersects(gdk::ModifierType::META_MASK) {
                if keyval == gdk::enums::key::f {
                    self_clone.toggle_fullscreen(&gtk_app, true);
                } else if keyval == gdk::enums::key::Left {
                    self_clone.seek(&SeekDirection::Backward, SEEK_BACKWARD_OFFSET);
                } else if keyval == gdk::enums::key::Right {
                    self_clone.seek(&SeekDirection::Forward, SEEK_FORWARD_OFFSET);
                }
            } else if keyval == gdk::enums::key::Escape {
                self_clone.toggle_fullscreen(&gtk_app, false);
            } else if keyval == gdk::enums::key::space {
                self_clone.toggle_pause();
            }
            Inhibit(false)
        }));

        let video_area_clone = SendCell::new(self.video_area.clone());
        self.player
            .connect_video_dimensions_changed(move |_, width, height| {
                let video_area = video_area_clone.borrow();
                video_area.set_size_request(width, height);
            });

        let window_clone = SendCell::new(self.window.clone());
        self.player.connect_media_info_updated(move |_, info| {
            let window = window_clone.borrow();
            if let Some(title) = info.get_title() {
                window.set_title(&*title);
            } else {
                window.set_title(&*info.get_uri());
            }
        });

        let pause_button_clone = SendCell::new(self.pause_button.clone());
        self.player.connect_state_changed(move |_, state| {
            let pause_button = pause_button_clone.borrow();
            match state {
                gst_player::PlayerState::Paused => {
                    let image = gtk::Image::new_from_icon_name(
                        "media-playback-start-symbolic",
                        gtk::IconSize::SmallToolbar.into(),
                    );
                    pause_button.set_image(&image);
                }
                gst_player::PlayerState::Playing => {
                    let image = gtk::Image::new_from_icon_name(
                        "media-playback-pause-symbolic",
                        gtk::IconSize::SmallToolbar.into(),
                    );
                    pause_button.set_image(&image);
                }
                _ => {}
            };
        });

        let range = self.progress_bar.clone().upcast::<gtk::Range>();
        let player = &self.player;
        let seek_signal_handler_id =
            range.connect_value_changed(clone_army!([player] move |range| {
                let value = range.get_value();
                player.seek(gst::ClockTime::from_seconds(value as u64));
            }));

        let progress_bar_clone = SendCell::new(self.progress_bar.clone());
        let signal_handler_id = Arc::new(Mutex::new(seek_signal_handler_id));
        self.player
            .connect_duration_changed(clone_army!([signal_handler_id] move |_, duration| {
                let progress_bar = progress_bar_clone.borrow();
                let range = progress_bar.clone().upcast::<gtk::Range>();
                let seek_signal_handler_id = signal_handler_id.lock().unwrap();
                glib::signal_handler_block(&range, &seek_signal_handler_id);
                range.set_range(0.0, duration.seconds().unwrap() as f64);
                glib::signal_handler_unblock(&range, &seek_signal_handler_id);
            }));

        let progress_bar_clone = SendCell::new(self.progress_bar.clone());
        self.player
            .connect_position_updated(clone_army!([signal_handler_id] move |_, position| {
                let progress_bar = progress_bar_clone.borrow();
                let range = progress_bar.clone().upcast::<gtk::Range>();
                let seek_signal_handler_id = signal_handler_id.lock().unwrap();
                glib::signal_handler_block(&range, &seek_signal_handler_id);
                range.set_value(position.seconds().unwrap() as f64);
                glib::signal_handler_unblock(&range, &seek_signal_handler_id);
            }));

        self.window
            .connect_delete_event(clone_army!([gtk_app] move |_, _| {
            gtk_app.quit();
            Inhibit(false)
        }));

        self.window.connect_map_event(move |widget, _| {
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

        {
            let app_clone = SendCell::new(gtk_app.clone());
            self.player.connect_error(move |_, error| {
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

    pub fn seek(&self, direction: &SeekDirection, offset: u64) {
        let position = self.player.get_position();
        let offset = gst::ClockTime::from_mseconds(offset);
        let destination = match *direction {
            SeekDirection::Backward => {
                if position >= offset {
                    Some(position - offset)
                } else {
                    None
                }
            }
            SeekDirection::Forward => {
                let duration = self.player.get_duration();
                if duration != gst::ClockTime::none() && position + offset <= duration {
                    Some(position + offset)
                } else {
                    None
                }
            }
        };
        if let Some(destination) = destination {
            self.player.seek(destination);
        }
    }

    pub fn toggle_pause(&self) {
        let pause_action = &self.pause_action;
        let player = &self.player;
        if let Some(is_paused) = pause_action.get_state() {
            let paused = is_paused.get::<bool>().unwrap();
            if paused {
                player.play();
            } else {
                player.pause();
            }
            pause_action.change_state(&(!paused).to_variant());
        }
    }

    pub fn toggle_fullscreen(&self, app: &gtk::Application, allowed: bool) {
        let fullscreen_action = &self.fullscreen_action;
        let window = &self.window;
        if let Some(is_fullscreen) = fullscreen_action.get_state() {
            let fullscreen = is_fullscreen.get::<bool>().unwrap();
            if fullscreen {
                if let Ok(mut cookie) = INHIBIT_COOKIE.lock() {
                    app.uninhibit(cookie.unwrap());
                    *cookie = None;
                }
                window.unfullscreen();
                self.toolbar_box.set_visible(true);
                window.present();
                fullscreen_action.change_state(&(!fullscreen).to_variant());
            } else if allowed {
                let flags =
                    gtk::ApplicationInhibitFlags::SUSPEND | gtk::ApplicationInhibitFlags::IDLE;
                *INHIBIT_COOKIE.lock().unwrap() = Some(app.inhibit(window, flags, None));
                *INITIAL_SIZE.lock().unwrap() = Some(window.get_size());
                *INITIAL_POSITION.lock().unwrap() = Some(window.get_position());
                window.fullscreen();
                self.toolbar_box.set_visible(false);
                fullscreen_action.change_state(&(!fullscreen).to_variant());
            }
        }
    }

    pub fn prepare_video_overlay(&self) {
        let video_window = &self.video_area;
        let gdk_window = video_window.get_window().unwrap();
        let video_overlay = &self.renderer;
        if !gdk_window.ensure_native() {
            println!("Can't create native window for widget");
            process::exit(-1);
        }

        let display_type_name = gdk_window.get_display().get_type().name();

        // Check if we're using X11 or ...
        if cfg!(target_os = "linux") {
            // Check if we're using X11 or ...
            if display_type_name == "GdkX11Display" {
                extern "C" {
                    pub fn gdk_x11_window_get_xid(
                        window: *mut glib::object::GObject,
                    ) -> *mut c_void;
                }

                unsafe {
                    let xid = gdk_x11_window_get_xid(gdk_window.to_glib_none().0);
                    video_overlay.set_window_handle(xid as usize);
                }
            } else {
                println!("Add support for display type '{}'", display_type_name);
                process::exit(-1);
            }
        } else if cfg!(target_os = "macos") {
            if display_type_name == "GdkQuartzDisplay" {
                extern "C" {
                    pub fn gdk_quartz_window_get_nsview(
                        window: *mut glib::object::GObject,
                    ) -> *mut c_void;
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

    fn draw_video_area(&self, cairo_context: &CairoContext) {
        let video_window = &self.video_area;
        let width = video_window.get_allocated_width();
        let height = video_window.get_allocated_height();

        // Paint some black borders
        cairo_context.rectangle(0., 0., f64::from(width), f64::from(height));
        cairo_context.fill();
    }

    fn resize_video_area(&self, event: &gdk::EventConfigure) {
        let video_overlay = &self.renderer;
        let (width, height) = event.get_size();
        let (x, y) = event.get_position();

        let player = &self.player;
        if let Ok(video_track) = player.get_property("current-video-track") {
            if let Some(video_track) = video_track.get::<gst_player::PlayerVideoInfo>() {
                let video_width = video_track.get_width();
                let video_height = video_track.get_height();
                let src_rect = gst_video::VideoRectangle::new(0, 0, video_width, video_height);

                let dst_rect = gst_video::VideoRectangle::new(x, y, width as i32, height as i32);
                let rect = gst_video::center_video_rectangle(src_rect, dst_rect, true);
                video_overlay.set_render_rectangle(rect.x, rect.y, rect.w, rect.h);
                video_overlay.expose();
                let video_window = &self.video_area;
                video_window.queue_draw();
            }
        }
    }

    pub fn play_asset(&self, asset: &str) {
        self.player
            .set_property("uri", &glib::Value::from(&asset))
            .unwrap();
        self.player.play();
    }

    pub fn open_files(&mut self, files: &[gio::File]) {
        let mut playlist = vec![];
        for file in files.to_vec() {
            if let Some(uri) = file.get_uri() {
                playlist.push(std::string::String::from(uri.as_str()));
            }
        }

        assert!(!files.is_empty());
        self.play_asset(&*playlist[0]);

        let inner_clone = SendCell::new(self.clone());
        let index_cell = RefCell::new(AtomicUsize::new(0));
        self.player.connect_end_of_stream(move |_| {
            let mut cell = index_cell.borrow_mut();
            let index = cell.get_mut();
            *index += 1;
            if *index < playlist.len() {
                let inner_clone = inner_clone.borrow();
                inner_clone.play_asset(&*playlist[*index]);
            }
            // TODO: else quit?
        });
    }
}

fn main() {
    #[cfg(not(unix))]
    {
        println!("Add support for target platform");
        process::exit(-1);
    }

    gst::init().expect("Failed to initialize GStreamer.");
    gtk::init().expect("Failed to initialize GTK.");

    let gtk_app = gtk::Application::new(None, gio::ApplicationFlags::HANDLES_OPEN).unwrap();

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
