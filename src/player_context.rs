extern crate cairo;
extern crate gdk;
extern crate glib;
extern crate gstreamer as gst;
extern crate gstreamer_player as gst_player;
extern crate gstreamer_video as gst_video;
extern crate gtk;
extern crate send_cell;

use cairo::Context as CairoContext;
use gdk::prelude::*;
use glib::translate::ToGlibPtr;
use gtk::prelude::*;
use send_cell::SendCell;
use std::cmp;
use std::os::raw::c_void;
use std::process;

use common::SeekDirection;

#[derive(Clone)]
pub struct PlayerContext {
    pub player: gst_player::Player,
    pub renderer: gst_player::PlayerVideoOverlayVideoRenderer,
    pub video_area: gtk::Widget,
    pub has_gtkgl: bool,
}

pub fn resize_video_area(video_area: &gtk::Widget, width: i32, height: i32) {
    let mut width = width;
    let mut height = height;
    if let Some(screen) = gdk::Screen::get_default() {
        width = cmp::max(width, screen.get_width());
        height = cmp::max(height, screen.get_height() - 100);
    }
    video_area.set_size_request(width, height);
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

        let video_area_clone = SendCell::new(video_area.clone());
        player.connect_video_dimensions_changed(move |_, width, height| {
            let video_area = video_area_clone.borrow();
            resize_video_area(&video_area, width, height);
        });

        PlayerContext {
            player,
            renderer,
            video_area,
            has_gtkgl,
        }
    }

    pub fn play_uri(&self, uri: &str) {
        self.player.connect_uri_loaded(move |player, _| {
            player.play();
        });

        self.player
            .set_property("uri", &glib::Value::from(&uri))
            .unwrap();
    }

    pub fn toggle_pause(&self, currently_paused: bool) {
        if currently_paused {
            self.player.play();
        } else {
            self.player.pause();
        }
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
            },
            SeekDirection::Forward => {
                let duration = self.player.get_duration();
                if duration != gst::ClockTime::none() && position + offset <= duration {
                    Some(position + offset)
                } else {
                    None
                }
            },
        };
        if let Some(destination) = destination {
            self.player.seek(destination);
        }
    }

    pub fn prepare_video_overlay(&self) {
        let gdk_window = self.video_area.get_window().unwrap();
        let video_overlay = &self.renderer;
        if !gdk_window.ensure_native() {
            println!("Can't create native window for widget");
            process::exit(-1);
        }

        let display_type_name = gdk_window.get_display().get_type().name();

        // Check if we're using X11 or ...
        if cfg!(target_os = "linux") {
            if !self.has_gtkgl {
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

    pub fn draw_video_overlay(&self, cairo_context: &CairoContext) {
        let width = self.video_area.get_allocated_width();
        let height = self.video_area.get_allocated_height();

        // Paint some black borders
        cairo_context.rectangle(0., 0., f64::from(width), f64::from(height));
        cairo_context.fill();
    }

    pub fn resize_video_area(&self, dst_rect: &gst_video::VideoRectangle) {
        if let Ok(video_track) = self.player.get_property("current-video-track") {
            if let Some(video_track) = video_track.get::<gst_player::PlayerVideoInfo>() {
                let video_width = video_track.get_width();
                let video_height = video_track.get_height();
                let src_rect = gst_video::VideoRectangle::new(0, 0, video_width, video_height);

                let rect = gst_video::center_video_rectangle(&src_rect, dst_rect, true);
                self.renderer
                    .set_render_rectangle(rect.x, rect.y, rect.w, rect.h);
                self.renderer.expose();
                self.video_area.queue_draw();
            }
        }
    }
}
