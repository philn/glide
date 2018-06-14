extern crate cairo;
extern crate dirs;
extern crate gdk;
extern crate glib;
extern crate gstreamer as gst;
extern crate gstreamer_player as gst_player;
extern crate gstreamer_video as gst_video;
extern crate gtk;
extern crate serde_json;
extern crate sha2;

use self::sha2::{Digest, Sha256};
use cairo::Context as CairoContext;
use dirs::Directories;
use failure::Error;
use gdk::prelude::*;
use glib::translate::ToGlibPtr;
use gtk::prelude::*;
use std::collections::HashMap;
use std::default::Default;
use std::fs::create_dir_all;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::os::raw::c_void;
use std::process;
use std::string;

use common::SeekDirection;

#[derive(Clone)]
pub struct PlayerContext {
    pub player: gst_player::Player,
    pub renderer: gst_player::PlayerVideoOverlayVideoRenderer,
    pub video_area: gtk::Widget,
    pub has_gtkgl: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaCache {
    files: HashMap<string::String, u64>,
}

fn parse_media_cache() -> Result<MediaCache, Error> {
    let d = Directories::with_prefix("glide", "Glide")?;
    create_dir_all(d.cache_home())?;
    let mut file = File::open(d.cache_home().join("media-cache.json"))?;
    let mut data = String::new();
    file.read_to_string(&mut data).unwrap();

    let json: MediaCache = serde_json::from_str(&data)?;
    Ok(json)
}

fn write_cache_to_file(data: &MediaCache) -> Result<(), Error> {
    let d = Directories::with_prefix("glide", "Glide")?;
    create_dir_all(d.cache_home())?;
    let mut file = File::create(d.cache_home().join("media-cache.json"))?;

    let json = serde_json::to_string(&data)?;
    file.write_all(json.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn uri_to_sha256(uri: &str) -> string::String {
    let mut sh = Sha256::default();
    sh.input(uri.as_bytes());
    let mut s = string::String::new();
    for byte in sh.result() {
        s.push_str(&*format!("{:02x}", byte));
    }
    s
}

fn find_last_position(uri: &str) -> gst::ClockTime {
    let id = uri_to_sha256(&string::String::from(uri));
    if let Ok(mut data) = parse_media_cache() {
        if let Some(position) = data.files.get_mut(&id) {
            return gst::ClockTime::from_nseconds(*position);
        }
    }
    gst::ClockTime::none()
}

impl PlayerContext {
    pub fn new() -> Self {
        let dispatcher = gst_player::PlayerGMainContextSignalDispatcher::new(None);
        let (sink, video_area, has_gtkgl) = if let Some(gtkglsink) = gst::ElementFactory::make("gtkglsink", None) {
            let glsinkbin = gst::ElementFactory::make("glsinkbin", None).unwrap();
            glsinkbin.set_property("sink", &gtkglsink.to_value()).unwrap();

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

        player.connect_end_of_stream(move |player| {
            if let Some(uri) = player.get_uri() {
                let id = uri_to_sha256(&uri);
                if let Ok(mut d) = parse_media_cache() {
                    if d.files.contains_key(&id) {
                        d.files.remove(&id);
                        write_cache_to_file(&d).unwrap();
                    }
                }
            }
        });

        // Get position updates every 250ms.
        let mut config = player.get_config();
        config.set_position_update_interval(250);
        player.set_config(config).unwrap();

        player.connect_uri_loaded(move |player, uri| {
            let position = find_last_position(uri);
            if position.is_some() {
                player.seek(position);
            }
            player.play();
        });

        PlayerContext {
            player,
            renderer,
            video_area,
            has_gtkgl,
        }
    }

    pub fn play_uri(&self, uri: &str) {
        self.player.set_property("uri", &glib::Value::from(&uri)).unwrap();
    }

    pub fn get_current_uri(&self) -> Option<string::String> {
        self.player.get_uri()
    }

    pub fn write_last_known_media_position(&self) {
        if let Some(uri) = self.player.get_uri() {
            let id = uri_to_sha256(&uri);
            let mut position = 0;
            if let Some(p) = self.player.get_position().nanoseconds() {
                position = p;
            }
            if let Some(duration) = self.player.get_duration().nanoseconds() {
                if position == duration {
                    return;
                }
            } else {
                // This likely is a live stream. Seek to last known
                // position will likely fail.
                return;
            }
            #[allow(unused_assignments)]
            let mut data = None;
            if let Ok(mut d) = parse_media_cache() {
                if d.files.contains_key(&id) {
                    if let Some(item) = d.files.get_mut(&id) {
                        *item = position;
                    }
                } else {
                    d.files.insert(id, position);
                }
                data = Some(d);
            } else {
                let mut cache = MediaCache { files: HashMap::new() };
                cache.files.insert(id, position);
                data = Some(cache);
            }
            if let Some(d) = data {
                write_cache_to_file(&d).unwrap();
            }
        }
    }

    pub fn increase_volume(&self) {
        let value = self.player.get_volume();
        let offset = 0.07;
        if value + offset < 1.0 {
            self.player.set_volume(value + offset);
        } else {
            self.player.set_volume(1.0);
        }
    }

    pub fn decrease_volume(&self) {
        let value = self.player.get_volume();
        let offset = 0.07;
        if value >= offset {
            self.player.set_volume(value - offset);
        } else {
            self.player.set_volume(0.0);
        }
    }

    pub fn toggle_mute(&self, enabled: bool) {
        self.player.set_mute(enabled);
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
        if position.is_none() {
            return;
        }
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
                if duration.is_some() && position + offset <= duration {
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
                self.renderer.set_render_rectangle(rect.x, rect.y, rect.w, rect.h);
                self.renderer.expose();
                self.video_area.queue_draw();
            }
        }
    }
}
