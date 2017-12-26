extern crate gstreamer as gst;
use gst::prelude::*;

extern crate glib;
use glib::translate::ToGlibPtr;

extern crate gio;
use gio::prelude::*;

extern crate gtk;
use gtk::prelude::*;

extern crate gdk;
use gdk::prelude::*;

use std::env;

use std::os::raw::c_void;

extern crate send_cell;
use send_cell::SendCell;

use std::process;

extern crate gstreamer_player as gst_player;
extern crate gstreamer_video as gst_video;

#[derive(Clone)]
struct VideoPlayer {
    app: gtk::Application,
    player: gst_player::Player,
    renderer: gst_player::PlayerVideoOverlayVideoRenderer,
}

impl VideoPlayer {
    pub fn new() -> Self {
        let dispatcher = gst_player::PlayerGMainContextSignalDispatcher::new(None);
        let sink = gst::ElementFactory::make("glimagesink", None).unwrap();
        let renderer1 = gst_player::PlayerVideoOverlayVideoRenderer::new_with_sink(&sink);
        let renderer = renderer1.clone();
        let player = gst_player::Player::new(
            Some(&renderer1.upcast::<gst_player::PlayerVideoRenderer>()),
            Some(&dispatcher.upcast::<gst_player::PlayerSignalDispatcher>()),
        );

        let app = gtk::Application::new(None, gio::ApplicationFlags::HANDLES_OPEN).unwrap();
        let app_clone = app.clone();

        let video_player = VideoPlayer {
            app,
            player,
            renderer,
        };
        app_clone.connect_startup(move |app| {
            let quit = gio::SimpleAction::new("quit", None);
            let app_clone = app.clone();
            quit.connect_activate(move |_, _| {
                app_clone.quit();
            });
            app.add_action(&quit);
        });
        let self_clone = video_player.clone();
        app_clone.connect_activate(move |_| {
            self_clone.create_ui();
        });
        let self_clone = video_player.clone();
        app_clone.connect_open(move |_, files, _| {
            self_clone.open_files(files);
        });
        video_player
    }

    pub fn run(&self) {
        let args = env::args().collect::<Vec<_>>();
        self.app.run(&args);
    }

    pub fn create_ui(&self) {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_default_size(320, 240);
        window.set_resizable(true);

        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let video_window = gtk::DrawingArea::new();
        video_window.set_double_buffered(false);

        let video_overlay = self.renderer.clone();
        video_window.connect_realize(move |video_window| {
            let video_overlay = &video_overlay;

            let gdk_window = video_window.get_window().unwrap();

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
        });

        let video_overlay_clone = self.renderer.clone();
        video_window.connect_draw(move |video_window, cairo_context| {
            let width = video_window.get_allocated_width();
            let height = video_window.get_allocated_height();

            // Paint some black borders
            cairo_context.rectangle(0., 0., width as f64, height as f64);
            cairo_context.fill();

            let video_overlay = &video_overlay_clone;
            video_overlay.expose();

            Inhibit(false)
        });

        vbox.pack_start(&video_window, true, true, 0);

        let label = gtk::Label::new("Position: 00:00:00");
        vbox.pack_start(&label, false, false, 5);
        window.add(&vbox);

        window.show_all();

        self.app.add_window(&window);

        let label_clone = SendCell::new(label.clone());
        self.player.connect_position_updated(move |_, position| {
            let label = label_clone.borrow();
            label.set_text(&format!("Position: {:.0}", position));
        });

        let video_overlay_clone = self.renderer.clone();
        let video_window_clone = SendCell::new(video_window.clone());
        let player_clone = self.player.clone();
        video_window.connect_configure_event(move |_, event| -> bool {
            let video_overlay = &video_overlay_clone;
            let (width, height) = event.get_size();
            let (x, y) = event.get_position();

            let player = &player_clone;
            let video_track = player.get_property("current-video-track").unwrap();
            let video_track = video_track.get::<gst_player::PlayerVideoInfo>().unwrap();
            let video_width = video_track.get_width();
            let video_height = video_track.get_height();
            let src_rect = gst_video::VideoRectangle::new(0, 0, video_width, video_height);

            let dst_rect = gst_video::VideoRectangle::new(x, y, width as i32, height as i32);
            let rect = gst_video::center_video_rectangle(src_rect, dst_rect, true);
            video_overlay.set_render_rectangle(rect.x, rect.y, rect.w, rect.h);
            video_overlay.expose();
            let video_window = video_window_clone.borrow();
            video_window.queue_draw();
            true
        });

        let video_window_clone = SendCell::new(video_window.clone());
        self.player
            .connect_video_dimensions_changed(move |_, width, height| {
                let video_window = video_window_clone.borrow();
                video_window.set_size_request(width, height);
            });

        let app_clone = self.app.clone();
        window.connect_delete_event(move |_, _| {
            let app = &app_clone;
            app.quit();
            Inhibit(false)
        });

        let app_clone = SendCell::new(self.app.clone());
        self.player.connect_error(move |_, error| {
            // FIXME: display some GTK error dialog...
            eprintln!("Error! {}", error);
            let app = &app_clone.borrow();
            app.quit();
        });

        let player_clone = self.player.clone();
        self.app.connect_shutdown(move |_| {
            let player = &player_clone;
            player.stop();
        });
    }

    pub fn open_files(&self, files: &[gio::File]) {
        if let Some(uri) = files[0].get_uri() {
            self.app.activate();
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

    let app = VideoPlayer::new();
    app.run();
}
