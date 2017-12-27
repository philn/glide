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

use cairo::Context as CairoContext;
use gdk::prelude::*;
use gio::prelude::*;
use glib::translate::ToGlibPtr;
use gst::prelude::*;
use gtk::prelude::*;
use send_cell::SendCell;
use std::env;
use std::os::raw::c_void;
use std::process;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Clone)]
struct VideoPlayer {
    player: gst_player::Player,
    renderer: gst_player::PlayerVideoOverlayVideoRenderer,
    window: gtk::Window,
    video_area: gtk::DrawingArea,
    fullscreen_action: gio::SimpleAction,
}

lazy_static! {
    static ref INHIBIT_COOKIE: Mutex<Option<u32>> = {
        Mutex::new(None)
    };
}

impl VideoPlayer {
    pub fn new(gtk_app: &gtk::Application) -> Arc<Self> {
        let dispatcher = gst_player::PlayerGMainContextSignalDispatcher::new(None);
        let sink = gst::ElementFactory::make("glimagesink", None).unwrap();
        let renderer1 = gst_player::PlayerVideoOverlayVideoRenderer::new_with_sink(&sink);
        let renderer = renderer1.clone();
        let player = gst_player::Player::new(
            Some(&renderer1.upcast::<gst_player::PlayerVideoRenderer>()),
            Some(&dispatcher.upcast::<gst_player::PlayerSignalDispatcher>()),
        );

        let video_area = gtk::DrawingArea::new();

        let fullscreen_action =
            gio::SimpleAction::new_stateful("fullscreen", None, &false.to_variant());
        gtk_app.add_action(&fullscreen_action);

        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_default_size(320, 240);
        window.set_resizable(true);

        let video_player = VideoPlayer {
            player,
            renderer,
            window,
            video_area,
            fullscreen_action,
        };
        let myself = Arc::new(video_player);

        gtk_app.connect_startup(move |app| {
            let quit = gio::SimpleAction::new("quit", None);
            let app_clone = app.clone();
            quit.connect_activate(move |_, _| {
                app_clone.quit();
            });
            app.add_action(&quit);
        });

        {
            let self_clone = Arc::clone(&myself);
            gtk_app.connect_open(move |app, files, _| {
                self_clone.open_files(app, files);
            });
        }

        {
            let self_clone = Arc::clone(&myself);
            myself.video_area.connect_realize(move |_| {
                self_clone.prepare_video_overlay();
            });
        }

        {
            let self_clone = Arc::clone(&myself);
            myself.video_area.connect_draw(move |_, cairo_context| {
                self_clone.draw_video_area(cairo_context);
                Inhibit(false)
            });
        }

        {
            let self_clone = Arc::clone(&myself);
            myself
                .video_area
                .connect_configure_event(move |_, event| -> bool {
                    self_clone.resize_video_area(event);
                    true
                });
        }

        {
            let app_clone = gtk_app.clone();
            let self_clone = Arc::clone(&myself);
            myself.window.connect_key_press_event(move |_, key| {
                let keyval = key.get_keyval();
                let keystate = key.get_state();
                let app = &app_clone;

                if keystate.intersects(gdk::ModifierType::META_MASK) {
                    if keyval == gdk::enums::key::f {
                        self_clone.toggle_fullscreen(app, true);
                    }
                } else if keyval == gdk::enums::key::Escape {
                    self_clone.toggle_fullscreen(app, false);
                }

                Inhibit(false)
            });
        }

        myself
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
                window.present();
                fullscreen_action.change_state(&(!fullscreen).to_variant());
            } else if allowed {
                let flags =
                    gtk::ApplicationInhibitFlags::SUSPEND | gtk::ApplicationInhibitFlags::IDLE;
                *INHIBIT_COOKIE.lock().unwrap() = Some(app.inhibit(window, flags, None));
                window.fullscreen();
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

        let video_overlay = &self.renderer;
        video_overlay.expose();
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

    pub fn start(&self, app: &gtk::Application) {
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        vbox.pack_start(&self.video_area, true, true, 0);

        let label = gtk::Label::new("Position: 00:00:00");
        vbox.pack_start(&label, false, false, 5);
        self.window.add(&vbox);

        self.window.show_all();
        app.add_window(&self.window);

        let label_clone = SendCell::new(label.clone());
        self.player.connect_position_updated(move |_, position| {
            let label = label_clone.borrow();
            label.set_text(&format!("Position: {:.0}", position));
        });

        let video_window_clone = SendCell::new(self.video_area.clone());
        self.player
            .connect_video_dimensions_changed(move |_, width, height| {
                let video_window = video_window_clone.borrow();
                video_window.set_size_request(width, height);
            });

        let app_clone = app.clone();
        self.window.connect_delete_event(move |_, _| {
            let app = &app_clone;
            app.quit();
            Inhibit(false)
        });

        let app_clone = SendCell::new(app.clone());
        self.player.connect_error(move |_, error| {
            // FIXME: display some GTK error dialog...
            eprintln!("Error! {}", error);
            let app = &app_clone.borrow();
            app.quit();
        });

        let player_clone = self.player.clone();
        app.connect_shutdown(move |_| {
            let player = &player_clone;
            player.stop();
        });
    }

    pub fn open_files(&self, app: &gtk::Application, files: &[gio::File]) {
        if let Some(uri) = files[0].get_uri() {
            app.activate();
            self.player
                .set_property("uri", &glib::Value::from(uri.as_str()))
                .unwrap();
            self.player.play();
        }
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
    let app = VideoPlayer::new(&gtk_app);
    gtk_app.connect_activate(move |gtk_app| {
        app.start(gtk_app);
    });

    let args = env::args().collect::<Vec<_>>();
    gtk_app.run(&args);
}
