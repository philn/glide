extern crate crossbeam_channel as channel;
extern crate glib;
extern crate gstreamer as gst;
extern crate gstreamer_player as gst_player;
extern crate serde_json;
extern crate sha2;

use crate::gtk_video_renderer;
use crate::video_renderer::VideoRenderer;

use self::sha2::{Digest, Sha256};
use failure::Error;
use gst::prelude::*;
use std::boxed::Box;
use std::cell::RefCell;
use std::collections::HashMap;
use std::default::Default;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path;
use std::string;

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
    Error,
}

pub struct ChannelPlayer {
    player: gst_player::Player,
    video_renderer: Box<VideoRenderer>,
}

#[derive(Serialize, Deserialize)]
struct MediaCacheData(pub HashMap<string::String, u64>);

struct MediaCache {
    path: path::PathBuf,
    data: MediaCacheData,
}

pub struct PlayerDataHolder {
    subscribers: Vec<channel::Sender<PlayerEvent>>,
    playlist: Vec<string::String>,
    current_uri: glib::GString,
    index: usize,
    cache: Option<MediaCache>,
}

thread_local!(
    pub static PLAYER_REGISTRY: RefCell<HashMap<glib::GString, PlayerDataHolder>> = RefCell::new(HashMap::new());
);

#[macro_export]
macro_rules! with_player {
    ($player:ident $code:block) => {
        with_player!($player $player $code)
    };
    ($player_id:ident $player:ident $code:block) => {
        let player_id = $player_id.get_name();
        PLAYER_REGISTRY.with(|registry| {
            if let Some(ref $player) = registry.borrow().get(&player_id) $code
        })
    };
}

#[macro_export]
macro_rules! with_mut_player {
    ($player_id:ident $player_data:ident $code:block) => (
        let player_id = $player_id.get_name();
        PLAYER_REGISTRY.with(|registry| {
            if let Some(ref mut $player_data) = registry.borrow_mut().get_mut(&player_id) $code
        })
    )
}

impl MediaCache {
    fn open<T: Copy + Into<path::PathBuf>>(path: T) -> Result<Self, Error> {
        MediaCache::read(path.into()).or_else(|_| {
            Ok(Self {
                path: path.into(),
                data: MediaCacheData(HashMap::new()),
            })
        })
    }

    fn read<T: AsRef<path::Path> + Into<path::PathBuf>>(path: T) -> Result<Self, Error> {
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

    fn write(&self) -> Result<(), Error> {
        let mut file = File::create(&self.path)?;

        let json = serde_json::to_string(&self.data)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    fn find_last_position(&self, uri: &str) -> gst::ClockTime {
        let id = uri_to_sha256(uri);
        if let Some(position) = self.data.0.get(&id) {
            return gst::ClockTime::from_nseconds(*position);
        }

        gst::ClockTime::none()
    }
}

fn uri_to_sha256(uri: &str) -> string::String {
    let mut sh = Sha256::default();
    sh.input(uri.as_bytes());
    sh.result()
        .into_iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .concat()
}

impl PlayerDataHolder {
    fn set_playlist(&mut self, playlist: Vec<string::String>) {
        self.playlist = playlist;
        self.index = 0;
    }

    #[allow(dead_code)]
    fn register_event_handler(&mut self, sender: channel::Sender<PlayerEvent>) {
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
            self.current_uri = uri;
            self.notify(&PlayerEvent::MediaInfoUpdated);
        }
    }

    fn end_of_stream(&mut self, player: &gst_player::Player) {
        if let Some(uri) = player.get_uri() {
            self.notify(&PlayerEvent::EndOfStream(uri.into()));
            self.index += 1;

            if self.index < self.playlist.len() {
                let next_uri = &*self.playlist[self.index];
                player.set_property("uri", &glib::Value::from(&next_uri)).unwrap();
            } else {
                self.notify(&PlayerEvent::EndOfPlaylist);
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

impl ChannelPlayer {
    pub fn new(sender: channel::Sender<PlayerEvent>, cache_file_path: Option<path::PathBuf>) -> Self {
        let dispatcher = gst_player::PlayerGMainContextSignalDispatcher::new(None);
        let video_renderer = gtk_video_renderer::GtkVideoRenderer::new();
        let player = gst_player::Player::new(
            video_renderer.gst_video_renderer(),
            Some(&dispatcher.upcast::<gst_player::PlayerSignalDispatcher>()),
        );

        video_renderer.set_player(&player);

        // Get position updates every 250ms.
        let mut config = player.get_config();
        config.set_position_update_interval(250);
        player.set_config(config).unwrap();

        player.connect_uri_loaded(|player, uri| {
            player.pause();
            with_mut_player!(player player_data {
                if let Some(ref cache) = player_data.cache {
                    let position = cache.find_last_position(uri);
                    if position.is_some() {
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
                    player_data.media_info_updated(&info);
            });
        });

        player.connect_position_updated(|player, _| {
            with_player!(player {
                player.notify(&PlayerEvent::PositionUpdated);
            });
        });

        player.connect_video_dimensions_changed(|player, width, height| {
            with_player!(player {
                player.notify(&PlayerEvent::VideoDimensionsChanged(width, height));
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
                    player.notify(&PlayerEvent::StateChanged(s));
                });
            }
        });

        player.connect_volume_changed(|player| {
            with_player!(player player_data {
                player_data.notify(&PlayerEvent::VolumeChanged(player.get_volume()));
            });
        });

        player.connect_error(|player, _error| {
            with_player!(player {
                // FIXME: Pass error to enum.
                player.notify(&PlayerEvent::Error);
            });
        });

        let player_id = player.get_name();
        let mut subscribers = Vec::new();
        subscribers.push(sender);
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

        Self {
            player,
            video_renderer: Box::new(video_renderer),
        }
    }

    pub fn video_renderer(&self) -> &VideoRenderer {
        &*self.video_renderer
    }

    #[allow(dead_code)]
    pub fn register_event_handler(&mut self, sender: channel::Sender<PlayerEvent>) {
        let player = &self.player;
        with_mut_player!(player player_data {
            player_data.register_event_handler(sender);
        });
    }

    pub fn load_playlist(&self, playlist: Vec<string::String>) {
        assert!(!playlist.is_empty());
        let player = &self.player;
        with_mut_player!(player player_data {
            self.load_uri(&*playlist[0]);
            player_data.set_playlist(playlist);
        });
    }

    pub fn load_uri(&self, uri: &str) {
        self.player.set_property("uri", &glib::Value::from(&uri)).unwrap();
    }

    pub fn get_current_uri(&self) -> Option<glib::GString> {
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

    pub fn seek(&self, direction: &SeekDirection) {
        let position = self.player.get_position();
        if position.is_none() {
            return;
        }

        let duration = self.player.get_duration();
        let destination = match direction {
            SeekDirection::Backward(offset) if position >= *offset => Some(position - *offset),
            SeekDirection::Forward(offset) if !duration.is_none() && position + *offset <= duration => {
                Some(position + *offset)
            }
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

    pub fn get_subtitle_uri(&self) -> Option<glib::GString> {
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
            if let Some(scheme) = glib::uri_parse_scheme(&uri) {
                if scheme == "fd" {
                    return;
                }
            }
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

            let player = &self.player;
            with_mut_player!(player player_data {
                player_data.update_cache_and_write(id, position);
            });
        }
    }
}
