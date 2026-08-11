extern crate gstreamer as gst;
extern crate gstreamer_play as gst_play;
extern crate gstreamer_video as gst_video;
extern crate gtk4 as gtk;
extern crate serde_json;
extern crate sha2;
extern crate tar;

use self::sha2::{Digest, Sha256};
use crate::debug_infos::DebugInfos;
use crate::gio::prelude::ActionExt;
use crate::gio::prelude::ApplicationExt;
use crate::gio::prelude::FileExt;
use crate::gio::prelude::OutputStreamExt;
use crate::gst_play::prelude::PlayStreamInfoExt;
use crate::gtk::prelude::PaintableExt;
use async_lock::OnceCell as AsyncOnceCell;
use gio::prelude::{ActionGroupExt, ActionMapExt};
use graphviz_rust::{
    cmd::{CommandArg, Format},
    exec_dot, parse,
    printer::{DotPrinter, PrinterContext},
};
use gst::prelude::*;
use gst_play::PlayMessage;
use gstreamer::format::Buffers;
use gstreamer::glib;
use gstreamer_pbutils::{Discoverer, DiscovererResult};
use gtk::gdk;
use gtk::glib::clone;
use mpris_server::{
    zbus::{self, fdo},
    LocalPlayerInterface, LocalRootInterface, LocalServer, LoopStatus, Metadata, PlaybackRate, PlaybackStatus,
    Property, Signal, Time, TrackId, Volume,
};
use std::borrow::BorrowMut;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path;
use std::string;
use tar::Builder;

#[derive(Serialize, Deserialize, Clone, Copy)]
pub enum PlaybackState {
    Stopped,
    Buffering,
    Paused,
    Playing,
}

impl PlaybackState {
    fn to_playback_status(self) -> PlaybackStatus {
        match self {
            Self::Stopped | Self::Buffering => PlaybackStatus::Stopped,
            Self::Playing => PlaybackStatus::Playing,
            Self::Paused => PlaybackStatus::Paused,
        }
    }
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
    DurationChanged(Option<gst::ClockTime>),
    PositionUpdated,
    EndOfStream(string::String),
    EndOfPlaylist,
    StateChanged(PlaybackState),
    VideoDimensionsChanged(u32, u32),
    VolumeChanged(f64),
    Error(String, Option<gst::Structure>),
    AudioVideoOffsetChanged(i64),
    SubtitleVideoOffsetChanged(i64),
    SeekDone,
}

#[derive(Clone)]
pub struct ChannelPlayer {
    player: gst_play::Play,
    renderer: gst_play::PlayVideoOverlayVideoRenderer,
    gtksink: gst::Element,
    cache_dir_path: Option<path::PathBuf>,
    gtk_app: adw::Application,
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
    state: PlaybackState,
    metadata: RefCell<Metadata>,
    seekable: bool,
}

thread_local!(
    static PLAYER_REGISTRY: RefCell<HashMap<glib::GString, PlayerDataHolder>> = RefCell::new(HashMap::new());
    static MPRIS_SERVER: AsyncOnceCell<LocalServer<ChannelPlayer>> = const { AsyncOnceCell::new() };
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

fn mpris_properties_changed(properties: impl IntoIterator<Item = Property> + 'static) {
    MPRIS_SERVER.with(|server| {
        if let Some(server) = server.get() {
            let _ = glib::MainContext::default().block_on(server.properties_changed(properties));
        }
    });
}

fn emit_mpris_signal(signal: Signal) {
    MPRIS_SERVER.with(|server| {
        if let Some(server) = server.get() {
            let _ = glib::MainContext::default().block_on(server.emit(signal));
        }
    });
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

fn cache_cover_art(data: &[u8]) -> Option<gio::File> {
    let mut cache_dir = glib::user_cache_dir();
    cache_dir.push("glide");
    cache_dir.push("covers");
    glib::mkdir_with_parents(&cache_dir, 0o755);

    let mut sh = Sha256::new();
    sh.update(data);
    let id = sh
        .finalize()
        .into_iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .concat();
    cache_dir.push(&id);

    let file = gio::File::for_path(&cache_dir);
    if file.query_exists(gio::Cancellable::NONE) {
        return Some(file);
    }

    match file.create(gio::FileCreateFlags::NONE, gio::Cancellable::NONE) {
        Ok(stream) => {
            if stream.write(data, gio::Cancellable::NONE).is_err() {
                return None;
            }
        }
        Err(_) => {
            return None;
        }
    };

    Some(file)
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

    fn set_state(&mut self, state: PlaybackState) {
        self.state = state;
        mpris_properties_changed([Property::PlaybackStatus(self.state.to_playback_status())]);
    }

    fn state(&self) -> PlaybackState {
        self.state
    }

    fn seek_done(&self, position: &gst::ClockTime) {
        emit_mpris_signal(Signal::Seeked {
            position: Time::from_micros(position.useconds() as i64),
        });
    }

    fn notify(&self, event: PlayerEvent) {
        for sender in &*self.subscribers {
            let _ = sender.send_blocking(event.clone());
        }
    }

    fn update_mpris_metadata(&mut self, info: &gst_play::PlayMediaInfo) {
        let mut builder = Metadata::builder().url(info.uri());
        if let Some(audio_info) = info.audio_streams().first() {
            if let Some(tags) = audio_info.tags() {
                if let Some(album_title) = tags.get::<gst::tags::Album>() {
                    builder = builder.album(album_title.get());
                }
                if let Some(artist) = tags.get::<gst::tags::Artist>() {
                    builder = builder.artist([artist.get()]);
                }
                if let Some(album_artist) = tags.get::<gst::tags::AlbumArtist>() {
                    builder = builder.album_artist([album_artist.get()]);
                }
                if let Some(track_number) = tags.get::<gst::tags::TrackNumber>() {
                    builder = builder.track_number(track_number.get() as i32);
                }
                if let Some(composer) = tags.get::<gst::tags::Composer>() {
                    builder = builder.composer([composer.get()]);
                }
                if let Some(date) = tags.get::<gst::tags::DateTime>() {
                    if let Ok(date) = date.get().to_iso8601_string() {
                        builder = builder.content_created(date);
                    }
                }
                if let Some(comment) = tags.get::<gst::tags::Comment>() {
                    builder = builder.comment([comment.get()]);
                }
                if let Some(audio_bpm) = tags.get::<gst::tags::BeatsPerMinute>() {
                    builder = builder.audio_bpm(audio_bpm.get() as i32);
                }
                if let Some(sample) = tags.get::<gst::tags::Image>() {
                    let sample = sample.get();
                    let buffer = sample.buffer().expect("Sample without buffer");
                    let mapped_buffer = buffer.map_readable().expect("Buffer un-readable");
                    let data = mapped_buffer.as_slice();
                    if let Some(path) = cache_cover_art(data) {
                        builder = builder.art_url(path.uri());
                    }
                }
            }
        }
        if let Some(title) = info.title() {
            builder = builder.title(title);
        }
        if let Some(duration) = info.duration() {
            builder = builder.length(Time::from_micros(duration.useconds() as i64));
        }
        let metadata = builder.build();
        self.metadata.replace(metadata.clone());
        mpris_properties_changed([Property::Metadata(metadata)]);
    }

    fn media_info_updated(&mut self, info: &gst_play::PlayMediaInfo) {
        let uri = info.uri();

        // Call this only once per asset.
        if self.current_uri != *uri {
            self.current_uri = uri;
            self.notify(PlayerEvent::MediaInfoUpdated);
            self.update_mpris_metadata(info);
            self.seekable = info.is_seekable();
        }
    }

    fn duration_changed(&mut self, duration: Option<gst::ClockTime>) {
        self.notify(PlayerEvent::DurationChanged(duration));
        if let Some(duration) = duration {
            self.metadata
                .borrow_mut()
                .set_length(Some(Time::from_micros(duration.useconds() as i64)));
            mpris_properties_changed([Property::Metadata(self.metadata.borrow().clone())]);
        }
    }

    fn end_of_stream(&mut self, player: &gst_play::Play) {
        if let Some(uri) = player.uri() {
            self.notify(PlayerEvent::EndOfStream(uri.into()));
            let _ = self.go_next(player);
        }
    }

    fn go_next(&mut self, player: &gst_play::Play) -> bool {
        self.index += 1;

        self.update_mpris_nav_controls();
        if self.index < self.playlist_length() {
            let next_uri = &*self.playlist[self.index];
            player.set_property("uri", next_uri);
            return true;
        }
        self.notify(PlayerEvent::EndOfPlaylist);
        false
    }

    fn go_prev(&mut self, player: &gst_play::Play) -> bool {
        if self.index < 1 {
            return false;
        }
        self.index -= 1;
        self.update_mpris_nav_controls();
        let uri = &*self.playlist[self.index];
        player.set_property("uri", uri);
        true
    }

    fn playlist_length(&self) -> usize {
        self.playlist.len()
    }

    fn can_go_next(&self) -> bool {
        self.index < self.playlist_length() - 1
    }

    fn can_go_prev(&self) -> bool {
        self.index >= 1
    }

    fn can_seek(&self) -> bool {
        self.seekable
    }

    fn update_mpris_nav_controls(&self) {
        let can_go_prev = self.can_go_prev();
        let can_go_next = self.can_go_next();
        glib::idle_add_local_once(move || {
            mpris_properties_changed([Property::CanGoNext(can_go_next), Property::CanGoPrevious(can_go_prev)]);
        });
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
        gtk_app: adw::Application,
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

        // Get position updates every 100ms.
        let mut config = player.config();
        config.set_position_update_interval(100);

        if gst::version() >= (1, 24, 0, 0) {
            config.set("pipeline-dump-in-error-details", true);
        }

        player.set_config(config).unwrap();

        if std::env::var("GST_DEBUG").is_err() {
            gst::log::remove_default_log_function();
            gst::log::add_ring_buffer_logger(2048, 60);
            let threshold = match std::env::var("GLIDE_DEBUG") {
                Ok(val) => val,
                Err(_) => "2,videodec*:5,playbin*:5".to_string(),
            };
            gst::log::set_threshold_from_string(&threshold, true);
        }

        let bus_watch = player.message_bus().add_watch_local(clone!(
            #[weak]
            player,
            #[upgrade_or]
            glib::ControlFlow::Break,
            move |_, message| {
                let play_message = if let Ok(msg) = PlayMessage::parse(message) {
                    msg
                } else {
                    return glib::ControlFlow::Continue;
                };

                match play_message {
                    PlayMessage::UriLoaded(_) => {
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
                    PlayMessage::EndOfStream(_) => {
                        with_mut_player!(player player_data {
                            player_data.end_of_stream(&player);
                        });
                    }
                    PlayMessage::MediaInfoUpdated(message) => {
                        with_mut_player!(player player_data {
                            player_data.media_info_updated(message.media_info());
                        });
                    }
                    PlayMessage::DurationChanged(message) => {
                        with_mut_player!(player player_data {
                            player_data.duration_changed(message.duration());
                        });
                    }
                    PlayMessage::PositionUpdated(_) => {
                        with_player!(player {
                            player.notify(PlayerEvent::PositionUpdated);
                        });
                    }
                    PlayMessage::VideoDimensionsChanged(message) => {
                        with_player!(player {
                            player.notify(PlayerEvent::VideoDimensionsChanged(message.width(), message.height()));
                        });
                    }
                    PlayMessage::StateChanged(message) => {
                        let state = match message.state() {
                            gst_play::PlayState::Playing => Some(PlaybackState::Playing),
                            gst_play::PlayState::Paused => Some(PlaybackState::Paused),
                            gst_play::PlayState::Stopped => Some(PlaybackState::Stopped),
                            _ => None,
                        };
                        if let Some(s) = state {
                            with_mut_player!(player player_data {
                                player_data.set_state(s);
                                player_data.notify(PlayerEvent::StateChanged(s));
                            });
                        }
                    }
                    PlayMessage::VolumeChanged(message) => {
                        with_player!(player player_data {
                            player_data.notify(PlayerEvent::VolumeChanged(message.volume()));
                        });
                    }
                    PlayMessage::Error(message) => {
                        with_player!(player {
                            let details = message.details().map(|s| s.to_owned());
                            player.notify(PlayerEvent::Error(message.error().to_string(), details));
                        });
                    }
                    PlayMessage::SeekDone(message) => {
                        with_player!(player player_data {
                            if let Some(position) = message.position() {
                                player_data.seek_done(&position);
                            }
                            player_data.notify(PlayerEvent::SeekDone);
                        });
                    }
                    _ => {}
                }

                glib::ControlFlow::Continue
            }
        ))?;

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
            state: PlaybackState::Stopped,
            metadata: RefCell::new(Metadata::new()),
            seekable: false,
        };

        PLAYER_REGISTRY.with(move |registry| {
            registry.borrow_mut().insert(player_id, player_data);
        });

        let result = Self {
            player,
            renderer,
            gtksink,
            cache_dir_path: cache_dir_path.map(|d| d.to_path_buf()),
            gtk_app,
        };

        if !incognito {
            let player = result.clone();
            glib::MainContext::default().spawn_local(async move {
                let local_server = LocalServer::new("net.base_art.Glide.Devel", player)
                    .await
                    .expect("Unable to create MPRIS server");
                glib::MainContext::default().spawn_local(local_server.run());
                MPRIS_SERVER.with(|mut server| {
                    let _ = server.borrow_mut().set_blocking(local_server);
                });
            });
        }
        Ok(result)
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
            pipeline.debug_to_dot_file_with_ts(gst::DebugGraphDetails::all(), label);
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
        let enabled = if let Some(track) = track {
            match track {
                SubtitleTrack::External(uri) => {
                    self.player.set_subtitle_uri(Some(&uri));
                }
                SubtitleTrack::Inband(idx) => {
                    let _ = self.player.set_subtitle_track(idx);
                }
            };
            true
        } else {
            false
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
        if tar_path.is_file() {
            std::fs::remove_file(&tar_path)?;
        }
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
            for log_data in gst::log::ring_buffer_logger_get_logs().iter() {
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
            Ok(pipeline.debug_to_dot_data(gst::DebugGraphDetails::all()).to_string())
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
        let mut dot_file = File::options()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(dot_path)?;
        let uri_re = regex::Regex::new(r#"uri\=(\\"[^\\"]*\\")"#)?;
        let file_re = regex::Regex::new(r#"location\=(\\"[^\\"]*\\")"#)?;
        for line in dot_data.lines() {
            let modified_line = uri_re.replace_all(line, r#"uri=\"redacted\""#);
            let modified_line2 = file_re.replace_all(&modified_line, r#"location=\"redacted\""#);
            dot_file.write_all(modified_line2.as_bytes())?;
        }
        dot_file.sync_all()?;

        // Convert pipeline dump dot graph to svg.
        let mut dot_contents = String::new();
        dot_file.seek(std::io::SeekFrom::Start(0))?;
        dot_file.read_to_string(&mut dot_contents)?;
        match parse(&dot_contents) {
            Ok(graph) => {
                let dot_string = graph.print(&mut PrinterContext::default());
                if let Ok(svg_graph) = exec_dot(dot_string, vec![CommandArg::Format(Format::Svg)]) {
                    let svg_path = tar_directory_path.join("pipeline.svg");
                    let mut svg_file = File::create(svg_path)?;
                    svg_file.write_all(&svg_graph)?;
                    svg_file.sync_all()?;
                }
            }
            Err(error) => {
                eprintln!("{}", error);
            }
        };

        // Dump media info to a file, making sure we don't leak private informations (URIs).
        let discoverer = Discoverer::new(gst::ClockTime::from_seconds(2))?;
        if let Ok(info) = discoverer.discover_uri(&uri) {
            // Look ahead for the result, in order to prevent critical warning from gst_discoverer_info_to_variant().
            match info.result() {
                DiscovererResult::Ok | DiscovererResult::MissingPlugins => {
                    let variant = info.to_variant(gstreamer_pbutils::DiscovererSerializeFlags::all());
                    let dump = variant.print(true).to_string();
                    let uri_re2 = regex::Regex::new(r#"<\(@ms ('[^']*')"#)?;
                    let modified_dump = uri_re2.replace_all(&dump, r#"<\(@ms 'redacted'"#);
                    let disco_path = tar_directory_path.join("media-info.variant");
                    let mut disco_file = File::create(disco_path)?;
                    disco_file.write_all(modified_dump.as_bytes())?;
                    disco_file.sync_all()?;
                }
                _ => {}
            };
        }

        let mut error_file = File::create(tar_directory_path.join("error.txt"))?;
        error_file.write_all(error_message.as_bytes())?;
        error_file.sync_all()?;

        let debug_info = DebugInfos::new();
        let mut debug_file = File::create(tar_directory_path.join("debug-infos.json"))?;
        debug_file.write_all(debug_info.to_json()?.as_bytes())?;
        debug_file.sync_all()?;

        a.append_dir_all(&tar_directory_name, &tar_directory_path)?;
        std::fs::remove_dir_all(&tar_directory_path)?;

        tar_path
            .into_os_string()
            .into_string()
            .map_err(|e| anyhow::anyhow!(format!("{}", e.to_str().unwrap())))
    }
}

impl LocalRootInterface for ChannelPlayer {
    async fn raise(&self) -> fdo::Result<()> {
        gio::Application::default().unwrap().activate();
        Ok(())
    }

    async fn quit(&self) -> fdo::Result<()> {
        gio::Application::default().unwrap().quit();
        Ok(())
    }

    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn fullscreen(&self) -> fdo::Result<bool> {
        let mut result: Option<bool> = None;
        if let Some(action) = self.gtk_app.lookup_action("fullscreen") {
            if let Some(state) = action.state() {
                result = state.get::<bool>();
            }
        }
        result.ok_or(fdo::Error::Failed("Unable to determine result".to_string()))
    }

    async fn set_fullscreen(&self, _fullscreen: bool) -> zbus::Result<()> {
        self.gtk_app.activate_action("fullscreen", None);
        Ok(())
    }

    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn has_track_list(&self) -> fdo::Result<bool> {
        let mut result: Option<bool> = None;
        let player_id = &self.player;
        with_player!(player_id player_data {
            result = Some(player_data.playlist_length() > 1);
        });
        result.ok_or(fdo::Error::Failed("Unable to determine result".to_string()))
    }

    async fn identity(&self) -> fdo::Result<String> {
        Ok("Glide".to_string())
    }

    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok("net.base_art.Glide.Devel".to_string())
    }

    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![])
    }

    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![])
    }
}

impl LocalPlayerInterface for ChannelPlayer {
    async fn next(&self) -> fdo::Result<()> {
        let player = &self.player;
        let mut result = false;
        with_mut_player!(player player_data {
            result = player_data.go_next(player);
        });
        if result {
            Ok(())
        } else {
            Err(fdo::Error::Failed("Unable to go to next track".into()))
        }
    }

    async fn previous(&self) -> fdo::Result<()> {
        let player = &self.player;
        let mut result = false;
        with_mut_player!(player player_data {
            result = player_data.go_prev(player);
        });
        if result {
            Ok(())
        } else {
            Err(fdo::Error::Failed("Unable to go to previous track".into()))
        }
    }

    async fn pause(&self) -> fdo::Result<()> {
        self.player.pause();
        Ok(())
    }

    async fn play_pause(&self) -> fdo::Result<()> {
        let status = self.playback_status().await?;
        self.toggle_pause(status == PlaybackStatus::Paused);
        Ok(())
    }

    async fn stop(&self) -> fdo::Result<()> {
        self.player.stop();
        Ok(())
    }

    async fn play(&self) -> fdo::Result<()> {
        self.player.play();
        Ok(())
    }

    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        let offset_abs = gst::ClockTime::from_useconds(offset.as_micros().unsigned_abs());
        let direction = if offset.is_positive() {
            SeekDirection::Forward(offset_abs)
        } else {
            SeekDirection::Backward(offset_abs)
        };
        self.seek(&direction);
        Ok(())
    }

    async fn set_position(&self, _track_id: TrackId, _position: Time) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported("SetPosition is not supported".into()))
    }

    async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported("OpenUri is not supported".into()))
    }

    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        let mut state: Option<PlaybackStatus> = None;
        let player_id = &self.player;
        with_player!(player_id player_data {
            state = Some(player_data.state().to_playback_status());
        });
        state.ok_or(fdo::Error::Failed("Playback status unknown".to_string()))
    }

    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(LoopStatus::None)
    }

    async fn set_loop_status(&self, _loop_status: LoopStatus) -> zbus::Result<()> {
        Err(zbus::Error::from(fdo::Error::NotSupported(
            "SetLoopStatus is not supported".into(),
        )))
    }

    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(self.playback_rate())
    }

    async fn set_rate(&self, rate: PlaybackRate) -> zbus::Result<()> {
        self.player.set_rate(rate);
        Ok(())
    }

    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn set_shuffle(&self, _shuffle: bool) -> zbus::Result<()> {
        Err(zbus::Error::from(fdo::Error::NotSupported(
            "SetShuffle is not supported".into(),
        )))
    }

    async fn metadata(&self) -> fdo::Result<Metadata> {
        let mut result: Option<Metadata> = None;
        let player_id = &self.player;
        with_player!(player_id player_data {
            result = Some(player_data.metadata.borrow().clone());
        });
        result.ok_or(fdo::Error::Failed("Metadata un-available".to_string()))
    }

    async fn volume(&self) -> fdo::Result<Volume> {
        Ok(self.player.volume())
    }

    async fn set_volume(&self, volume: Volume) -> zbus::Result<()> {
        self.player.set_volume(volume);
        Ok(())
    }

    async fn position(&self) -> fdo::Result<Time> {
        if let Some(position) = self.get_position() {
            Ok(Time::from_micros(position.useconds() as i64))
        } else {
            Err(fdo::Error::NotSupported("Position is unknown".into()))
        }
    }

    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(0.25)
    }

    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(2.0)
    }

    async fn can_go_next(&self) -> fdo::Result<bool> {
        let mut result: Option<bool> = None;
        let player_id = &self.player;
        with_player!(player_id player_data {
            result = Some(player_data.can_go_next());
        });
        result.ok_or(fdo::Error::Failed("Unable to determine result".to_string()))
    }

    async fn can_go_previous(&self) -> fdo::Result<bool> {
        let mut result: Option<bool> = None;
        let player_id = &self.player;
        with_player!(player_id player_data {
            result = Some(player_data.can_go_prev());
        });
        result.ok_or(fdo::Error::Failed("Unable to determine result".to_string()))
    }

    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_seek(&self) -> fdo::Result<bool> {
        let mut result: Option<bool> = None;
        let player_id = &self.player;
        with_player!(player_id player_data {
            result = Some(player_data.can_seek());
        });
        result.ok_or(fdo::Error::Failed("Unable to determine result".to_string()))
    }

    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}
