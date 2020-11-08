extern crate gdk;
extern crate gstreamer as gst;
extern crate gstreamer_player as gst_player;
extern crate gstreamer_video as gst_video;
extern crate gtk;

use gdk::prelude::*;
use glib::SendWeakRef;
use gst::prelude::*;
use gtk::prelude::*;
use std::os::raw::c_void;

use crate::channel_player::PLAYER_REGISTRY;
use crate::with_player;

use crate::video_renderer;

fn prepare_video_overlay(video_area: &gtk::Widget, video_overlay: &gst_player::PlayerVideoOverlayVideoRenderer) {
    let gdk_window = video_area.get_window().unwrap();
    if !gdk_window.ensure_native() {
        println!("Can't create native window for widget");
        std::process::exit(-1);
    }

    let display_type_name = gdk_window.get_display().get_type().name();

    // Check if we're using X11 or ...
    #[cfg(target_os = "linux")]
    {
        // Check if we're using X11 or ...
        if display_type_name == "GdkX11Display" {
            extern "C" {
                pub fn gdk_x11_window_get_xid(window: *mut glib::object::GObject) -> *mut c_void;
            }

            unsafe {
                let xid = gdk_x11_window_get_xid(gdk_window.as_ptr() as *mut _);
                video_overlay.set_window_handle(xid as usize);
            }
        } else {
            eprintln!("Add support for display type '{}'", display_type_name);
            std::process::exit(-1);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if display_type_name == "GdkQuartzDisplay" {
            extern "C" {
                pub fn gdk_quartz_window_get_nsview(window: *mut glib::object::GObject) -> *mut c_void;
            }

            unsafe {
                let window = gdk_quartz_window_get_nsview(gdk_window.as_ptr() as *mut _);
                video_overlay.set_window_handle(window as usize);
            }
        } else {
            eprintln!("Unsupported display type '{}", display_type_name);
            std::process::exit(-1);
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    unimplemented!();
}

pub struct GtkVideoRenderer {
    video_area: gtk::Widget,
    gst_renderer: gst_player::PlayerVideoOverlayVideoRenderer,
    video_renderer: gst_player::PlayerVideoRenderer,
}

impl GtkVideoRenderer {
    pub fn new() -> Self {
        let (sink, video_area) = if let Ok(gtkglsink) = gst::ElementFactory::make("gtkglsink", None) {
            let glsinkbin = gst::ElementFactory::make("glsinkbin", None).unwrap();
            glsinkbin.set_property("sink", &gtkglsink.to_value()).unwrap();

            let widget = gtkglsink.get_property("widget").unwrap();
            (
                glsinkbin,
                widget
                    .get::<gtk::Widget>()
                    .expect("Widget property should be a Widget...")
                    .unwrap(),
            )
        } else {
            let sink = gst::ElementFactory::make("glimagesink", None).unwrap();
            let widget = gtk::DrawingArea::new();
            (sink, widget.upcast::<gtk::Widget>())
        };

        let renderer1 = gst_player::PlayerVideoOverlayVideoRenderer::new_with_sink(&sink);

        let renderer = renderer1.clone();
        let renderer_weak = renderer.downgrade();
        video_area.connect_realize(move |video_area| {
            let renderer = match renderer_weak.upgrade() {
                Some(renderer) => renderer,
                None => return,
            };
            prepare_video_overlay(video_area, &renderer);
        });

        video_area.connect_draw(move |video_area, cairo_context| {
            let (allocated, _) = video_area.get_allocated_size();

            // Paint some black borders
            cairo_context.rectangle(
                f64::from(allocated.x),
                f64::from(allocated.y),
                f64::from(allocated.width),
                f64::from(allocated.height),
            );
            cairo_context.fill();

            Inhibit(false)
        });

        let renderer2 = renderer1.clone();
        let video_renderer = renderer2.upcast::<gst_player::PlayerVideoRenderer>();
        Self {
            video_area: video_area,
            gst_renderer: renderer1,
            video_renderer: video_renderer,
        }
    }
}

impl video_renderer::VideoRenderer for GtkVideoRenderer {
    fn gst_video_renderer(&self) -> Option<&gst_player::PlayerVideoRenderer> {
        Some(&self.video_renderer)
    }

    fn set_player(&self, player: &gst_player::Player) {
        let renderer = self.gst_renderer.clone();
        let renderer_weak = renderer.downgrade();
        let player_weak = player.downgrade();

        self.video_area
            .connect_configure_event(move |video_area, event| -> bool {
                let (width, height) = event.get_size();
                let (x, y) = event.get_position();
                let rect = gst_video::VideoRectangle::new(x, y, width as i32, height as i32);

                if let Some(player) = player_weak.upgrade() {
                    if let Ok(video_track) = player.get_property("current-video-track") {
                        let video_track = video_track
                            .get::<gst_player::PlayerVideoInfo>()
                            .expect("current-video-track should be a PlayerVideoInfo");
                        if let Some(video_track) = video_track {
                            let mut v: Option<(i32, i32)> = None;
                            with_player!(player player_data {
                                v = player_data.video_dimensions();
                            });
                            let video_width = video_track.get_width();
                            let video_height = video_track.get_height();
                            let video_rect = Some(gst_video::VideoRectangle::new(0, 0, video_width, video_height));

                            if let Some(src_rect) = video_rect {
                                let rect = gst_video::center_video_rectangle(&src_rect, &rect, true);
                                video_area.queue_draw();
                                match renderer_weak.upgrade() {
                                    Some(renderer) => {
                                        renderer.set_render_rectangle(rect.x, rect.y, rect.w, rect.h);
                                        renderer.expose();
                                    }
                                    None => {}
                                };
                            }
                        }
                    }
                }
                true
            });
    }

    fn implementation(&self) -> Option<video_renderer::VideoWidgetImpl> {
        Some(video_renderer::VideoWidgetImpl::GTK(SendWeakRef::from(
            self.video_area.downgrade(),
        )))
    }

    fn refresh(&self, video_width: i32, video_height: i32) {
        let (allocated, _) = self.video_area.get_allocated_size();
        let rect = gst_video::VideoRectangle::new(allocated.x, allocated.y, allocated.width, allocated.height);

        let src_rect = gst_video::VideoRectangle::new(0, 0, video_width, video_height);
        let rect = gst_video::center_video_rectangle(&src_rect, &rect, true);

        self.video_area.queue_draw();
        let renderer = &self.gst_renderer;
        renderer.set_render_rectangle(rect.x, rect.y, rect.w, rect.h);
        renderer.expose();
    }
}
