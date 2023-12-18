extern crate gstreamer as gst;
extern crate gstreamer_play as gst_play;
extern crate gstreamer_video as gst_video;
extern crate gtk4 as gtk;
extern crate serde_json;
extern crate sha2;
extern crate tar;

use self::sha2::{Digest, Sha256};
use crate::gst_play::prelude::PlayStreamInfoExt;
use crate::gtk::prelude::PaintableExt;
use gst::prelude::*;
use gst_play::PlayMessage;
use gstreamer::format::Buffers;
use gstreamer::glib;
use gtk::gdk;
use gtk::glib::clone;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path;
use std::string;
use tar::Builder;

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
    VideoDimensionsChanged(u32, u32),
    VolumeChanged(f64),
    Error(String, Option<gst::Structure>),
    AudioVideoOffsetChanged(i64),
    SubtitleVideoOffsetChanged(i64),
}

pub struct ChannelPlayer {
    player: gst_play::Play,
    renderer: gst_play::PlayVideoOverlayVideoRenderer,
    gtksink: gst::Element,
    cache_dir_path: Option<path::PathBuf>,
}

impl Drop for ChannelPlayer {
    fn drop(&mut self) {
        self.player.message_bus().set_flushing(true);
    }
}

#[derive(Serialize, Deserialize)]
struct MediaCacheData(pub HashMap<string::String, u64>);

struct MediaCache {
    path: path::PathBuf,
    data: MediaCacheData,
}

struct PlayerDataHolder {
    subscribers: Vec<async_channel::Sender<PlayerEvent>>,
    playlist: Vec<string::String>,
    current_uri: glib::GString,
    index: usize,
    cache: Option<MediaCache>,
    #[allow(dead_code)]
    bus_watch: gst::bus::BusWatchGuard,
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

impl PlayerDataHolder {
    fn set_playlist(&mut self, playlist: Vec<string::String>) {
        self.playlist = playlist;
        self.index = 0;
    }

    #[allow(dead_code)]
    fn register_event_handler(&mut self, sender: async_channel::Sender<PlayerEvent>) {
        self.subscribers.push(sender);
    }

    fn notify(&self, event: PlayerEvent) {
        for sender in &*self.subscribers {
            let _ = sender.send_blocking(event.clone());
        }
    }

    fn media_info_updated(&mut self, info: &gst_play::PlayMediaInfo) {
        let uri = info.uri();

        // Call this only once per asset.
        if self.current_uri != *uri {
            self.current_uri = uri;
            self.notify(PlayerEvent::MediaInfoUpdated);
        }
    }

    fn end_of_stream(&mut self, player: &gst_play::Play) {
        if let Some(uri) = player.uri() {
            self.notify(PlayerEvent::EndOfStream(uri.into()));
            self.index += 1;

            if self.index < self.playlist.len() {
                let next_uri = &*self.playlist[self.index];
                player.set_property("uri", next_uri);
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

impl ChannelPlayer {
    pub fn new(
        sender: async_channel::Sender<PlayerEvent>,
        incognito: bool,
        cache_dir_path: Option<path::PathBuf>,
    ) -> anyhow::Result<Self> {
        let gtksink = gst::ElementFactory::make("gtk4paintablesink").build()?;

        // Need to set state to Ready to get a GL context
        gtksink.set_state(gst::State::Ready)?;

        let paintable = gtksink.property::<gdk::Paintable>("paintable");

        let sink = if paintable.property::<Option<gdk::GLContext>>("gl-context").is_some() {
            gst::ElementFactory::make("glsinkbin")
                .property("sink", &gtksink)
                .build()?
        } else {
            gtksink.clone()
        };

        let renderer = gst_play::PlayVideoOverlayVideoRenderer::with_sink(&sink);

        let player = gst_play::Play::new(Some(renderer.clone().upcast::<gst_play::PlayVideoRenderer>()));

        // Get position updates every 250ms.
        let mut config = player.config();
        config.set_position_update_interval(250);

        // TODO: Enable pipeline_dump_in_error_details, guarded by gst 1.24 feature check.
        // https://gitlab.freedesktop.org/gstreamer/gstreamer/-/merge_requests/5828

        player.set_config(config).unwrap();

        if std::env::var("GST_DEBUG").is_err() {
            gst::debug_remove_default_log_function();
            gst::debug_add_ring_buffer_logger(2048, 60);
            let threshold = match std::env::var("GLIDE_DEBUG") {
                Ok(val) => val,
                Err(_) => "2,videodec*:5,playbin*:5".to_string(),
            };
            gst::debug_set_threshold_from_string(&threshold, true);
        }

        let bus_watch = player.message_bus().add_watch_local(
            clone!(@weak player => @default-return glib::ControlFlow::Break, move |_, message| {
                let play_message = if let Ok(msg) = PlayMessage::parse(message) {
                    msg
                } else {
                    return glib::ControlFlow::Continue;
                };

                match play_message {
                    PlayMessage::UriLoaded => {
                        player.pause();
                        let uri = player.uri().unwrap();
                        with_mut_player!(player player_data {
                            if let Some(ref cache) = player_data.cache {
                                if let Some(position) = cache.find_last_position(&uri) {
                                    player.seek(position);
                                }
                            }
                        });
                        player.play();
                    }
                    PlayMessage::EndOfStream => {
                        with_mut_player!(player player_data {
                            player_data.end_of_stream(&player);
                        });
                    }
                    PlayMessage::MediaInfoUpdated { info } => {
                        with_mut_player!(player player_data {
                            player_data.media_info_updated(&info);
                        });

                    }
                    PlayMessage::PositionUpdated { position: _ } => {
                        with_player!(player {
                            player.notify(PlayerEvent::PositionUpdated);
                        });

                    }
                    PlayMessage::VideoDimensionsChanged { width, height } => {
                        with_player!(player {
                            player.notify(PlayerEvent::VideoDimensionsChanged(width, height));
                        });

                    }
                    PlayMessage::StateChanged { state } => {
                        let state = match state {
                            gst_play::PlayState::Playing => Some(PlaybackState::Playing),
                            gst_play::PlayState::Paused => Some(PlaybackState::Paused),
                            gst_play::PlayState::Stopped => Some(PlaybackState::Stopped),
                            _ => None,
                        };
                        if let Some(s) = state {
                            with_player!(player {
                                player.notify(PlayerEvent::StateChanged(s));
                            });
                        }
                    }
                    PlayMessage::VolumeChanged { volume } => {
                        with_player!(player player_data {
                            player_data.notify(PlayerEvent::VolumeChanged(volume));
                        });

                    }
                    PlayMessage::Error { error, details } => {
                        with_player!(player {
                            player.notify(PlayerEvent::Error(error.to_string(), details));
                        });
                    }
                    _ => {}
                }

                glib::ControlFlow::Continue
            }),
        )?;

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
        if !incognito {
            if let Some(ref path) = cache_dir_path {
                let cache_path = path.join("media-cache.json");
                cache = Some(MediaCache::open(&cache_path).unwrap());
            }
        }
        let player_data = PlayerDataHolder {
            subscribers,
            playlist: vec![],
            current_uri: "".into(),
            index: 0,
            cache,
            bus_watch,
        };

        PLAYER_REGISTRY.with(move |registry| {
            registry.borrow_mut().insert(player_id, player_data);
        });

        Ok(Self {
            player,
            renderer,
            gtksink,
            cache_dir_path: cache_dir_path.map(|d| d.to_path_buf()),
        })
    }

    #[allow(dead_code)]
    pub fn register_event_handler(&mut self, sender: async_channel::Sender<PlayerEvent>) {
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

    pub fn paintable(&self) -> gdk::Paintable {
        self.gtksink.property::<gdk::Paintable>("paintable")
    }

    pub fn update_render_rectangle(&self, p: &gdk::Paintable) {
        if let Some(video_track) = self.player.current_video_track() {
            let (width, height) = (p.intrinsic_width(), p.intrinsic_height());
            let (x, y) = (0, 0);
            let rect = gst_video::VideoRectangle::new(x, y, width, height);

            let video_width = video_track.width();
            let video_height = video_track.height();
            let src_rect = gst_video::VideoRectangle::new(0, 0, video_width, video_height);

            let rect = gst_video::center_video_rectangle(&src_rect, &rect, true);
            self.renderer.set_render_rectangle(rect.x, rect.y, rect.w, rect.h);
            self.renderer.expose();
        }
    }

    pub fn load_uri(&self, uri: &str) {
        self.player.set_property("uri", uri);
    }

    pub fn get_current_uri(&self) -> Option<glib::GString> {
        self.player.uri()
    }

    pub fn stop(&self) {
        self.player.stop();
    }

    pub fn get_media_info(&self) -> Option<gst_play::PlayMediaInfo> {
        self.player.media_info()
    }

    pub fn duration(&self) -> Option<gst::ClockTime> {
        self.player.duration()
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
        let Some(position) = self.player.position() else {
            return;
        };

        let duration = self.player.duration();
        let destination = match direction {
            SeekDirection::Backward(offset) => position.saturating_sub(*offset),
            SeekDirection::Forward(offset) if duration.is_some() => (position + *offset).min(duration.unwrap()),
            _ => return,
        };

        self.player.seek(destination);
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

    pub fn get_current_subtitle_track(&self) -> Option<gst_play::PlaySubtitleInfo> {
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

    pub fn get_audio_track_cover(&self) -> Option<gst::Sample> {
        let track = self.player.current_audio_track()?;
        let tags = track.tags()?;
        let cover = tags.get::<gst::tags::Image>()?;
        Some(cover.get())
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
        self.player.set_property("audio-video-offset", offset);
    }

    pub fn set_subtitle_offset(&self, offset: i64) {
        self.player.set_property("subtitle-video-offset", offset);
    }

    pub fn video_frame_step(&self) {
        self.gtksink
            .send_event(gst::event::Step::new(Buffers::ONE, 1.0, true, false));
    }

    pub fn playback_rate(&self) -> f64 {
        self.player.rate()
    }

    pub fn increase_speed(&self) {
        let rate = self.player.rate();
        let offset = 0.25;
        if rate + offset <= 2.0 {
            self.player.set_rate(rate + offset);
        }
    }

    pub fn decrease_speed(&self) {
        let rate = self.player.rate();
        let offset = 0.25;
        if rate > offset {
            self.player.set_rate(rate - offset);
        }
    }

    pub fn write_error_report(
        &self,
        error_message: &String,
        details: Option<gst::Structure>,
    ) -> anyhow::Result<String> {
        let cache_dir = self
            .cache_dir_path
            .as_ref()
            .ok_or(anyhow::anyhow!("Unable to determine cache directory path."))?;

        let uri = self.player.uri().unwrap();
        let id = uri_to_sha256(&uri);

        let tar_directory_name = format!("glide-error-{id}");
        let tar_filename = format!("{tar_directory_name}.tar");
        let tar_path = cache_dir.join(tar_filename);
        let tar_file = File::create(&tar_path)?;
        let mut a = Builder::new(tar_file);

        let tar_directory_path = cache_dir.join(&tar_directory_name);
        std::fs::create_dir_all(&tar_directory_path)?;

        // Dump contents of the GStreamer debug ring-buffer to a file.
        if std::env::var("GST_DEBUG").is_ok() {
            eprintln!("GST_DEBUG was set. GStreamer logs will not be automatically included the report");
        } else {
            let gst_log = tar_directory_path.join("gst.log");
            let mut file = File::create(gst_log)?;
            for log_data in gst::debug_ring_buffer_logger_get_logs().iter() {
                file.write_all(log_data.as_bytes())?;
            }
            file.sync_all()?;
        }

        // Dump pipeline graph to a file, making sure we don't leak private informations (URIs).
        let dump_pipeline = || -> anyhow::Result<String> {
            let element = self.player.pipeline();
            let pipeline = element
                .downcast::<gst::Pipeline>()
                .map_err(|_| anyhow::anyhow!("Missing pipeline"))?;
            Ok(gst::debug_bin_to_dot_data(&pipeline, gst::DebugGraphDetails::all()).to_string())
        };

        let dot_data = match details {
            Some(d) => {
                if d.has_field("pipeline-dump") {
                    Ok(d.get::<String>("pipeline-dump").unwrap().to_string())
                } else {
                    dump_pipeline()
                }
            }
            None => dump_pipeline(),
        }?;

        let dot_path = tar_directory_path.join("pipeline.dot");
        let mut dot_file = File::create(dot_path)?;
        let uri_re = regex::Regex::new(r#"uri\=(\\"[^\\"]*\\")"#)?;
        let file_re = regex::Regex::new(r#"location\=(\\"[^\\"]*\\")"#)?;
        for line in dot_data.lines() {
            let modified_line = uri_re.replace_all(line, r#"uri=\"redacted\""#);
            let modified_line2 = file_re.replace_all(&modified_line, r#"location=\"redacted\""#);
            dot_file.write_all(modified_line2.as_bytes())?;
        }
        dot_file.sync_all()?;

        // Dump media info to a file, making sure we don't leak private informations (URIs).
        let discoverer = gstreamer_pbutils::Discoverer::new(gst::ClockTime::from_seconds(2))?;
        let info = discoverer.discover_uri(&uri)?;
        let variant = info.to_variant(gstreamer_pbutils::DiscovererSerializeFlags::all());
        let dump = variant.print(true).to_string();
        let uri_re2 = regex::Regex::new(r#"<\(@ms ('[^']*')"#)?;
        let modified_dump = uri_re2.replace_all(&dump, r#"<\(@ms 'redacted'"#);
        let disco_path = tar_directory_path.join("media-info.variant");
        let mut disco_file = File::create(disco_path)?;
        disco_file.write_all(modified_dump.as_bytes())?;
        disco_file.sync_all()?;

        let mut error_file = File::create(tar_directory_path.join("error.txt"))?;
        error_file.write_all(error_message.as_bytes())?;
        error_file.sync_all()?;

        a.append_dir_all(&tar_directory_name, &tar_directory_path)?;
        std::fs::remove_dir_all(&tar_directory_path)?;

        tar_path
            .into_os_string()
            .into_string()
            .map_err(|e| anyhow::anyhow!(format!("{}", e.to_str().unwrap())))
    }
}
