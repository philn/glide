extern crate glib;
extern crate gstreamer as gst;
extern crate gstreamer_player as gst_player;
extern crate gstreamer_video as gst_video;
extern crate gtk;
extern crate serde_json;
extern crate sha2;

use self::sha2::{Digest, Sha256};
use gst::prelude::*;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::os::raw::c_void;
use std::path;
use std::process;
use std::string;
use thiserror::Error;

#[derive(Serialize, Deserialize, Clone)]
pub enum PlaybackState {
    Stopped,
    Paused,
    Playing,
}

pub enum SeekDirection {
    Backward(gst::ClockTime),
    Forward(gst::ClockTime),
}

pub enum SubtitleTrack {
    Inband(i32),
    External(glib::GString),
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
    Error(string::String),
    AudioVideoOffsetChanged(i64),
    SubtitleVideoOffsetChanged(i64),
}

pub struct ChannelPlayer {
    player: gst_player::Player,
    video_area: gtk::Widget,
}

#[derive(Serialize, Deserialize)]
struct MediaCacheData(pub HashMap<string::String, u64>);

struct MediaCache {
    path: path::PathBuf,
    data: MediaCacheData,
}

struct PlayerDataHolder {
    subscribers: Vec<glib::Sender<PlayerEvent>>,
    playlist: Vec<string::String>,
    current_uri: glib::GString,
    index: usize,
    cache: Option<MediaCache>,
}

thread_local!(
    static PLAYER_REGISTRY: RefCell<HashMap<glib::GString, PlayerDataHolder>> = RefCell::new(HashMap::new());
);

macro_rules! with_player {
    ($player:ident $code:block) => {
        with_player!($player $player $code)
    };
    ($player_id:ident $player:ident $code:block) => {
        let player_id = $player_id.name();
        PLAYER_REGISTRY.with(|registry| {
            if let Some(ref $player) = registry.borrow().get(&player_id) $code
        })
    };
}

macro_rules! with_mut_player {
    ($player_id:ident $player_data:ident $code:block) => (
        let player_id = $player_id.name();
        PLAYER_REGISTRY.with(|registry| {
            if let Some(ref mut $player_data) = registry.borrow_mut().get_mut(&player_id) $code
        })
    )
}

impl MediaCache {
    fn open<T: Copy + Into<path::PathBuf>>(path: T) -> anyhow::Result<Self> {
        MediaCache::read(path.into()).or_else(|_| {
            Ok(Self {
                path: path.into(),
                data: MediaCacheData(HashMap::new()),
            })
        })
    }

    fn read<T: AsRef<path::Path> + Into<path::PathBuf>>(path: T) -> anyhow::Result<Self> {
        let mut file = File::open(path.as_ref())?;
        let mut data = String::new();
        file.read_to_string(&mut data).unwrap();

        let json: MediaCacheData = serde_json::from_str(&data)?;
        Ok(Self {
            path: path.into(),
            data: json,
        })
    }

    fn update<K: Into<String>>(&mut self, id: K, value: u64) {
        self.data.0.insert(id.into(), value);
    }

    fn write(&self) -> anyhow::Result<()> {
        let mut file = File::create(&self.path)?;

        let json = serde_json::to_string(&self.data)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    fn find_last_position(&self, uri: &str) -> Option<gst::ClockTime> {
        let id = uri_to_sha256(uri);
        if let Some(position) = self.data.0.get(&id) {
            return Some(gst::ClockTime::from_nseconds(*position));
        }

        None
    }
}

fn uri_to_sha256(uri: &str) -> string::String {
    let mut sh = Sha256::new();
    sh.update(uri.as_bytes());
    sh.finalize()
        .into_iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .concat()
}

fn prepare_video_overlay(video_area: &gtk::DrawingArea, video_overlay: &gst_player::PlayerVideoOverlayVideoRenderer) {
    let gdk_window = video_area.window().unwrap();
    if !gdk_window.ensure_native() {
        println!("Can't create native window for widget");
        process::exit(-1);
    }

    let display_type_name = gdk_window.display().type_().name();

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
            eprintln!("Add support for display type '{display_type_name}'");
            process::exit(-1);
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
            process::exit(-1);
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    unimplemented!();
}

impl PlayerDataHolder {
    fn set_playlist(&mut self, playlist: Vec<string::String>) {
        self.playlist = playlist;
        self.index = 0;
    }

    #[allow(dead_code)]
    fn register_event_handler(&mut self, sender: glib::Sender<PlayerEvent>) {
        self.subscribers.push(sender);
    }

    fn notify(&self, event: PlayerEvent) {
        for sender in &*self.subscribers {
            sender.send(event.clone()).unwrap();
        }
    }

    fn media_info_updated(&mut self, info: &gst_player::PlayerMediaInfo) {
        let uri = info.uri();

        // Call this only once per asset.
        if self.current_uri != *uri {
            self.current_uri = uri;
            self.notify(PlayerEvent::MediaInfoUpdated);
        }
    }

    fn end_of_stream(&mut self, player: &gst_player::Player) {
        if let Some(uri) = player.uri() {
            self.notify(PlayerEvent::EndOfStream(uri.into()));
            self.index += 1;

            if self.index < self.playlist.len() {
                let next_uri = &*self.playlist[self.index];
                player.set_property("uri", &next_uri);
            } else {
                self.notify(PlayerEvent::EndOfPlaylist);
            }
        }
    }

    fn update_cache_and_write(&mut self, id: string::String, position: u64) {
        if let Some(ref mut cache) = self.cache {
            cache.update(id, position);
            cache.write().unwrap();
        }
    }
}

fn create_renderer() -> (Option<gst_player::PlayerVideoOverlayVideoRenderer>, Option<gtk::Widget>) {
    if let Ok(gtkglsink) = gst::ElementFactory::make("gtkglsink", None) {
        let glsinkbin = gst::ElementFactory::make("glsinkbin", None).unwrap();
        glsinkbin.set_property("sink", &gtkglsink);

        let widget = gtkglsink.property::<gtk::Widget>("widget");
        (
            Some(gst_player::PlayerVideoOverlayVideoRenderer::with_sink(&glsinkbin)),
            Some(widget),
        )
    } else if let Ok(sink) = gst::ElementFactory::make("glimagesink", None) {
        let video_area = gtk::DrawingArea::new();

        let renderer = gst_player::PlayerVideoOverlayVideoRenderer::with_sink(&sink);
        let renderer_weak = renderer.downgrade();
        video_area.connect_realize(move |video_area| {
            let renderer = match renderer_weak.upgrade() {
                Some(renderer) => renderer,
                None => return,
            };
            prepare_video_overlay(video_area, &renderer);
        });

        (Some(renderer), Some(video_area.upcast::<gtk::Widget>()))
    } else {
        (None, None)
    }
}

#[derive(Error, Debug)]
pub enum PlayerError {
    #[error("Neither gtkglsink nor glimagesink found. Make sure to install gst-plugins-good with GTK support enabled, or gst-plugins-base")]
    NoRendererFound,
}

impl ChannelPlayer {
    pub fn new(sender: glib::Sender<PlayerEvent>, cache_file_path: Option<path::PathBuf>) -> anyhow::Result<Self> {
        let (renderer, video_area) = create_renderer();
        if renderer.is_none() {
            return Err(anyhow::anyhow!(PlayerError::NoRendererFound));
        }
        let renderer1 = Some(
            renderer
                .as_ref()
                .unwrap()
                .clone()
                .upcast::<gst_player::PlayerVideoRenderer>(),
        );
        let dispatcher = gst_player::PlayerGMainContextSignalDispatcher::new(None);
        let player = gst_player::Player::new(
            renderer1.as_ref(),
            Some(&dispatcher.upcast::<gst_player::PlayerSignalDispatcher>()),
        );

        // Get position updates every 250ms.
        let mut config = player.config();
        config.set_position_update_interval(250);
        player.set_config(config).unwrap();

        let video_area = video_area.unwrap();
        video_area.connect_draw(move |video_area, cairo_context| {
            let width = video_area.allocated_width();
            let height = video_area.allocated_height();

            // Paint some black borders
            cairo_context.rectangle(0., 0., f64::from(width), f64::from(height));
            cairo_context.fill().unwrap();

            Inhibit(false)
        });

        let player_weak = player.downgrade();
        let renderer_weak = renderer.unwrap().downgrade();
        video_area.connect_configure_event(move |video_area, event| -> bool {
            let (width, height) = event.size();
            let (x, y) = event.position();
            let rect = gst_video::VideoRectangle::new(x, y, width as i32, height as i32);

            let player = match player_weak.upgrade() {
                Some(player) => player,
                None => return true,
            };

            let video_track = player.property::<gst_player::PlayerVideoInfo>("current-video-track");
            let video_width = video_track.width();
            let video_height = video_track.height();
            let src_rect = gst_video::VideoRectangle::new(0, 0, video_width, video_height);

            let rect = gst_video::center_video_rectangle(&src_rect, &rect, true);
            let renderer = match renderer_weak.upgrade() {
                Some(renderer) => renderer,
                None => return true,
            };
            renderer.set_render_rectangle(rect.x, rect.y, rect.w, rect.h);
            renderer.expose();
            video_area.queue_draw();
            true
        });

        player.connect_uri_loaded(|player, uri| {
            player.pause();
            with_mut_player!(player player_data {
                if let Some(ref cache) = player_data.cache {
                    if let Some(position) = cache.find_last_position(uri) {
                        player.seek(position);
                    }
                }
            });
            player.play();
        });

        player.connect_end_of_stream(|player| {
            with_mut_player!(player player_data {
                player_data.end_of_stream(player);
            });
        });

        player.connect_media_info_updated(|player, info| {
            with_mut_player!(player player_data {
                player_data.media_info_updated(info);
            });
        });

        player.connect_position_updated(|player, _| {
            with_player!(player {
                player.notify(PlayerEvent::PositionUpdated);
            });
        });

        player.connect_video_dimensions_changed(|player, width, height| {
            with_player!(player {
                player.notify(PlayerEvent::VideoDimensionsChanged(width, height));
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
                with_player!(player {
                    player.notify(PlayerEvent::StateChanged(s));
                });
            }
        });

        player.connect_volume_changed(|player| {
            with_player!(player player_data {
                player_data.notify(PlayerEvent::VolumeChanged(player.volume()));
            });
        });

        player.connect_error(|player, error| {
            with_player!(player {
                // FIXME: Pass error to enum.
                player.notify(PlayerEvent::Error(error.to_string()));
            });
        });

        player.connect_audio_video_offset_notify(|player| {
            with_player!(player player_data {
                player_data.notify(PlayerEvent::AudioVideoOffsetChanged(player.audio_video_offset()));
            });
        });

        player.connect_subtitle_video_offset_notify(|player| {
            with_player!(player player_data {
                player_data.notify(PlayerEvent::SubtitleVideoOffsetChanged(player.subtitle_video_offset()));
            });
        });

        let player_id = player.name();
        let subscribers = vec![sender];
        let mut cache = None;
        if let Some(ref path) = cache_file_path {
            cache = Some(MediaCache::open(path).unwrap());
        }
        let player_data = PlayerDataHolder {
            subscribers,
            playlist: vec![],
            current_uri: "".into(),
            index: 0,
            cache,
        };

        PLAYER_REGISTRY.with(move |registry| {
            registry.borrow_mut().insert(player_id, player_data);
        });

        Ok(Self { player, video_area })
    }

    #[allow(dead_code)]
    pub fn register_event_handler(&mut self, sender: glib::Sender<PlayerEvent>) {
        let player = &self.player;
        with_mut_player!(player player_data {
            player_data.register_event_handler(sender);
        });
    }

    pub fn load_playlist(&self, playlist: Vec<string::String>) {
        assert!(!playlist.is_empty());
        let player = &self.player;
        with_mut_player!(player player_data {
            self.load_uri(&playlist[0]);
            player_data.set_playlist(playlist);
        });
    }

    pub fn video_area(&self) -> &gtk::Widget {
        &self.video_area
    }

    pub fn load_uri(&self, uri: &str) {
        self.player.set_property("uri", &uri);
    }

    pub fn get_current_uri(&self) -> Option<glib::GString> {
        self.player.uri()
    }

    pub fn stop(&self) {
        self.player.stop();
    }

    pub fn get_media_info(&self) -> Option<gst_player::PlayerMediaInfo> {
        self.player.media_info()
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
        let value = self.player.volume();
        let offset = 0.07;
        if value + offset < 1.0 {
            self.player.set_volume(value + offset);
        } else {
            self.player.set_volume(1.0);
        }
    }

    pub fn decrease_volume(&self) {
        let value = self.player.volume();
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
        let element = self.player.pipeline();
        if let Ok(pipeline) = element.downcast::<gst::Pipeline>() {
            gst::debug_bin_to_dot_file_with_ts(&pipeline, gst::DebugGraphDetails::all(), label);
        }
    }

    pub fn seek(&self, direction: &SeekDirection) {
        let position = self.player.position();
        if position.is_none() {
            return;
        }

        let position = position.unwrap();
        let duration = self.player.duration();
        let destination = match direction {
            SeekDirection::Backward(offset) if position >= *offset => Some(position - *offset),
            SeekDirection::Forward(offset) => match duration {
                Some(duration) if position + *offset <= duration => Some(position + *offset),
                _ => None,
            },
            _ => None,
        };
        if let Some(d) = destination {
            self.player.seek(d)
        }
    }

    pub fn seek_to(&self, position: gst::ClockTime) {
        self.player.seek(position);
    }

    pub fn get_position(&self) -> Option<gst::ClockTime> {
        self.player.position()
    }

    pub fn configure_subtitle_track(&self, track: Option<SubtitleTrack>) {
        let enabled = match track {
            Some(track) => match track {
                SubtitleTrack::External(uri) => {
                    self.player.set_subtitle_uri(Some(&uri));
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

    pub fn get_current_subtitle_track(&self) -> Option<gst_player::PlayerSubtitleInfo> {
        self.player.current_subtitle_track()
    }

    pub fn get_subtitle_uri(&self) -> Option<glib::GString> {
        self.player.subtitle_uri()
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
        if let Some(uri) = self.player.uri() {
            if let Some(scheme) = glib::uri_parse_scheme(&uri) {
                if scheme == "fd" {
                    return;
                }
            }
            let id = uri_to_sha256(&uri);
            let mut position = 0;
            if let Some(p) = self.player.position() {
                position = p.nseconds();
            }
            if let Some(duration) = self.player.duration() {
                if position == duration.nseconds() {
                    return;
                }
            } else {
                // This likely is a live stream. Seek to last known
                // position will likely fail.
                return;
            }

            let player = &self.player;
            with_mut_player!(player player_data {
                player_data.update_cache_and_write(id, position);
            });
        }
    }

    pub fn set_audio_offset(&self, offset: i64) {
        self.player.set_property("audio-video-offset", &offset);
    }

    pub fn set_subtitle_offset(&self, offset: i64) {
        self.player.set_property("subtitle-video-offset", &offset);
    }
}
