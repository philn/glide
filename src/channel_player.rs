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
use failure::Error;
use gdk::prelude::*;
use glib::translate::ToGlibPtr;
use gst::prelude::*;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::default::Default;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::os::raw::c_void;
use std::path;
use std::process;
use std::string;
use std::sync::mpsc;

#[derive(Serialize, Deserialize, Clone)]
pub enum PlaybackState {
    Stopped,
    Paused,
    Playing,
}

pub enum SeekDirection {
    Backward,
    Forward,
}

pub enum SubtitleTrack {
    Inband(i32),
    External(String),
}

pub struct AudioVisualization(pub string::String);

#[derive(Serialize, Deserialize, Clone)]
pub enum PlayerEvent {
    MediaInfoUpdated,
    PositionUpdated,
    EndOfStream(string::String),
    EndOfPlaylist,
    StateChanged(PlaybackState),
    VideoDimensionsChanged(i32, i32),
    VolumeChanged(f64),
    Error,
}

pub struct ChannelPlayer {
    player: gst_player::Player,
    video_area: gtk::Widget,
    cache_file_path: path::PathBuf,
}

struct PlayerDataHolder {
    subscribers: Vec<mpsc::Sender<PlayerEvent>>,
    playlist: Vec<string::String>,
    current_uri: string::String,
    index: usize,
}

thread_local!(
    static PLAYER_REGISTRY: RefCell<HashMap<string::String, PlayerDataHolder>> = RefCell::new(HashMap::new());
);

#[derive(Serialize, Deserialize)]
struct MediaCache {
    files: HashMap<string::String, u64>,
}

fn parse_media_cache(path: &path::PathBuf) -> Result<MediaCache, Error> {
    let mut file = File::open(path)?;
    let mut data = String::new();
    file.read_to_string(&mut data).unwrap();

    let json: MediaCache = serde_json::from_str(&data)?;
    Ok(json)
}

fn write_cache_to_file(path: &path::PathBuf, data: &MediaCache) -> Result<(), Error> {
    let mut file = File::create(path)?;

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

fn find_last_position(path: &path::PathBuf, uri: &str) -> gst::ClockTime {
    let id = uri_to_sha256(&string::String::from(uri));
    if let Ok(data) = parse_media_cache(path) {
        if let Some(position) = data.files.get(&id) {
            return gst::ClockTime::from_nseconds(*position);
        }
    }
    gst::ClockTime::none()
}

fn prepare_video_overlay(
    video_area: &gtk::Widget,
    video_overlay: &gst_player::PlayerVideoOverlayVideoRenderer,
    has_gtkgl: bool,
) {
    let gdk_window = video_area.get_window().unwrap();
    if !gdk_window.ensure_native() {
        println!("Can't create native window for widget");
        process::exit(-1);
    }

    let display_type_name = gdk_window.get_display().get_type().name();

    // Check if we're using X11 or ...
    if cfg!(target_os = "linux") {
        if !has_gtkgl {
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

impl PlayerDataHolder {
    fn set_playlist(&mut self, playlist: Vec<string::String>) {
        self.playlist = playlist;
        self.index = 0;
    }

    #[allow(dead_code)]
    fn register_event_handler(&mut self, sender: mpsc::Sender<PlayerEvent>) {
        self.subscribers.push(sender);
    }

    fn notify(&self, event: &PlayerEvent) {
        for sender in &*self.subscribers {
            sender.send(event.clone()).unwrap();
        }
    }

    fn media_info_updated(&mut self, info: &gst_player::PlayerMediaInfo) {
        let uri = info.get_uri();

        // Call this only once per asset.
        if self.current_uri != *uri {
            self.current_uri = uri.clone();
            self.notify(&PlayerEvent::MediaInfoUpdated);
        }
    }

    fn end_of_stream(&mut self, player: &gst_player::Player) {
        if let Some(uri) = player.get_uri() {
            self.notify(&PlayerEvent::EndOfStream(uri));
            self.index += 1;

            if self.index < self.playlist.len() {
                let next_uri = &*self.playlist[self.index];
                player.set_property("uri", &glib::Value::from(&next_uri)).unwrap();
            } else {
                self.notify(&PlayerEvent::EndOfPlaylist);
            }
        }
    }
}

impl ChannelPlayer {
    pub fn new(sender: mpsc::Sender<PlayerEvent>, cache_file_path: path::PathBuf) -> Self {
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

        // Get position updates every 250ms.
        let mut config = player.get_config();
        config.set_position_update_interval(250);
        player.set_config(config).unwrap();

        let cache_file_path_clone = cache_file_path.clone();
        player.connect_uri_loaded(move |player, uri| {
            player.pause();
            let position = find_last_position(&cache_file_path_clone, uri);
            if position.is_some() {
                player.seek(position);
            }
            player.play();
        });

        let renderer_weak = renderer.downgrade();
        video_area.connect_realize(move |video_area| {
            let renderer = match renderer_weak.upgrade() {
                Some(renderer) => renderer,
                None => return,
            };
            prepare_video_overlay(video_area, &renderer, has_gtkgl);
        });

        video_area.connect_draw(move |video_area, cairo_context| {
            let width = video_area.get_allocated_width();
            let height = video_area.get_allocated_height();

            // Paint some black borders
            cairo_context.rectangle(0., 0., f64::from(width), f64::from(height));
            cairo_context.fill();

            Inhibit(false)
        });

        let player_weak = player.downgrade();
        let renderer_weak = renderer.downgrade();
        video_area.connect_configure_event(move |video_area, event| -> bool {
            let (width, height) = event.get_size();
            let (x, y) = event.get_position();
            let rect = gst_video::VideoRectangle::new(x, y, width as i32, height as i32);

            let player = match player_weak.upgrade() {
                Some(player) => player,
                None => return true,
            };
            if let Ok(video_track) = player.get_property("current-video-track") {
                if let Some(video_track) = video_track.get::<gst_player::PlayerVideoInfo>() {
                    let video_width = video_track.get_width();
                    let video_height = video_track.get_height();
                    let src_rect = gst_video::VideoRectangle::new(0, 0, video_width, video_height);

                    let rect = gst_video::center_video_rectangle(&src_rect, &rect, true);
                    let renderer = match renderer_weak.upgrade() {
                        Some(renderer) => renderer,
                        None => return true,
                    };
                    renderer.set_render_rectangle(rect.x, rect.y, rect.w, rect.h);
                    renderer.expose();
                    video_area.queue_draw();
                }
            }
            true
        });

        player.connect_end_of_stream(move |player| {
            let player_id = player.get_name();
            PLAYER_REGISTRY.with(|registry| {
                if let Some(ref mut player_data) = registry.borrow_mut().get_mut(&player_id) {
                    player_data.end_of_stream(player);
                }
            });
        });

        player.connect_media_info_updated(move |player, info| {
            let player_id = player.get_name();
            PLAYER_REGISTRY.with(|registry| {
                if let Some(ref mut player_data) = registry.borrow_mut().get_mut(&player_id) {
                    player_data.media_info_updated(&info);
                }
            });
        });

        player.connect_position_updated(|player, _| {
            let player_id = player.get_name();
            PLAYER_REGISTRY.with(|registry| {
                if let Some(ref player_data) = registry.borrow().get(&player_id) {
                    player_data.notify(&PlayerEvent::PositionUpdated);
                }
            });
        });

        player.connect_video_dimensions_changed(|player, width, height| {
            let player_id = player.get_name();
            PLAYER_REGISTRY.with(|registry| {
                if let Some(ref player_data) = registry.borrow().get(&player_id) {
                    player_data.notify(&PlayerEvent::VideoDimensionsChanged(width, height));
                }
            });
        });

        player.connect_state_changed(|player, state| {
            let state = match state {
                gst_player::PlayerState::Playing => Some(PlaybackState::Playing),
                gst_player::PlayerState::Paused => Some(PlaybackState::Paused),
                gst_player::PlayerState::Stopped => Some(PlaybackState::Stopped),
                _ => None,
            };
            if let Some(s) = state {
                let player_id = player.get_name();
                PLAYER_REGISTRY.with(|registry| {
                    if let Some(ref player_data) = registry.borrow().get(&player_id) {
                        player_data.notify(&PlayerEvent::StateChanged(s));
                    }
                });
            }
        });

        player.connect_volume_changed(|player| {
            let player_id = player.get_name();
            PLAYER_REGISTRY.with(|registry| {
                if let Some(ref player_data) = registry.borrow().get(&player_id) {
                    player_data.notify(&PlayerEvent::VolumeChanged(player.get_volume()));
                }
            });
        });

        player.connect_error(|player, _error| {
            let player_id = player.get_name();
            PLAYER_REGISTRY.with(|registry| {
                if let Some(ref player_data) = registry.borrow().get(&player_id) {
                    // FIXME: Pass error to enum.
                    player_data.notify(&PlayerEvent::Error);
                }
            });
        });

        let player_id = player.get_name();
        let mut subscribers = Vec::new();
        subscribers.push(sender);
        let player_data = PlayerDataHolder {
            subscribers,
            playlist: vec![],
            current_uri: "".to_string(),
            index: 0,
        };

        PLAYER_REGISTRY.with(move |registry| {
            registry.borrow_mut().insert(player_id, player_data);
        });

        Self {
            player,
            video_area,
            cache_file_path,
        }
    }

    #[allow(dead_code)]
    pub fn register_event_handler(&mut self, sender: mpsc::Sender<PlayerEvent>) {
        let player_id = self.player.get_name();
        PLAYER_REGISTRY.with(|registry| {
            if let Some(ref mut player_data) = registry.borrow_mut().get_mut(&player_id) {
                player_data.register_event_handler(sender);
            }
        });
    }

    pub fn load_playlist(&self, playlist: Vec<string::String>) {
        assert!(!playlist.is_empty());

        let player_id = self.player.get_name();
        PLAYER_REGISTRY.with(|registry| {
            if let Some(ref mut player_data) = registry.borrow_mut().get_mut(&player_id) {
                self.load_uri(&*playlist[0]);
                player_data.set_playlist(playlist);
            }
        });
    }

    pub fn video_area(&self) -> &gtk::Widget {
        &self.video_area
    }

    pub fn load_uri(&self, uri: &str) {
        self.player.set_property("uri", &glib::Value::from(&uri)).unwrap();
    }

    pub fn get_current_uri(&self) -> Option<string::String> {
        self.player.get_uri()
    }

    pub fn stop(&self) {
        self.player.stop();
    }

    pub fn get_media_info(&self) -> Option<gst_player::PlayerMediaInfo> {
        self.player.get_media_info()
    }

    pub fn set_volume(&self, volume: f64) {
        self.player.set_volume(volume);
    }

    pub fn toggle_pause(&self, currently_paused: bool) {
        if currently_paused {
            self.player.play();
        } else {
            self.player.pause();
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

    pub fn dump_pipeline(&self, label: &str) {
        let element = self.player.get_pipeline();
        if let Ok(pipeline) = element.downcast::<gst::Pipeline>() {
            gst::debug_bin_to_dot_file_with_ts(&pipeline, gst::DebugGraphDetails::all(), label);
        }
    }

    pub fn seek(&self, direction: &SeekDirection, offset: gst::ClockTime) {
        let position = self.player.get_position();
        if position.is_none() || offset.is_none() {
            return;
        }

        let duration = self.player.get_duration();
        let destination = match direction {
            SeekDirection::Backward if position >= offset => Some(position - offset),
            SeekDirection::Forward if !duration.is_none() && position + offset <= duration => Some(position + offset),
            _ => None,
        };
        if let Some(d) = destination {
            self.player.seek(d)
        }
    }

    pub fn seek_to(&self, position: gst::ClockTime) {
        self.player.seek(position);
    }

    pub fn get_position(&self) -> gst::ClockTime {
        self.player.get_position()
    }

    pub fn configure_subtitle_track(&self, track: Option<SubtitleTrack>) {
        let enabled = match track {
            Some(track) => match track {
                SubtitleTrack::External(uri) => {
                    self.player.set_subtitle_uri(&uri);
                    true
                }
                SubtitleTrack::Inband(idx) => {
                    self.player.set_subtitle_track(idx).unwrap();
                    true
                }
            },
            None => false,
        };
        self.player.set_subtitle_track_enabled(enabled);
    }

    pub fn get_subtitle_uri(&self) -> Option<string::String> {
        self.player.get_subtitle_uri()
    }

    pub fn set_audio_track_index(&self, idx: i32) {
        self.player.set_audio_track_enabled(idx > -1);
        if idx >= 0 {
            self.player.set_audio_track(idx).unwrap();
        }
    }

    pub fn set_video_track_index(&self, idx: i32) {
        self.player.set_video_track_enabled(idx > -1);
        if idx >= 0 {
            self.player.set_video_track(idx).unwrap();
        }
    }

    pub fn set_audio_visualization(&self, vis: Option<AudioVisualization>) {
        match vis {
            Some(v) => {
                self.player.set_visualization(Some(v.0.as_str())).unwrap();
                self.player.set_visualization_enabled(true);
            }
            None => {
                self.player.set_visualization_enabled(false);
            }
        };
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
            if let Ok(d) = parse_media_cache(&self.cache_file_path) {
                data = Some(d);
            } else {
                data = Some(MediaCache { files: HashMap::new() });
            }
            if let Some(mut d) = data {
                d.files.insert(id, position);
                write_cache_to_file(&self.cache_file_path, &d).unwrap();
            }
        }
    }
}
