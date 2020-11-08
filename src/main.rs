#[cfg(target_os = "macos")]
extern crate core_foundation;
extern crate directories;
extern crate failure;
extern crate gdk;
extern crate gio;
extern crate glib;
extern crate gstreamer as gst;
extern crate gstreamer_player as gst_player;
extern crate gstreamer_video as gst_video;
extern crate gtk;
#[macro_use]
extern crate lazy_static;
#[cfg(feature = "self-updater")]
#[macro_use]
extern crate self_update;

#[macro_use]
extern crate serde_derive;

use directories::ProjectDirs;
use failure::Error;
#[allow(unused_imports)]
use gdk::prelude::*;
use gio::prelude::*;
use glib::ToVariant;
use std::cell::RefCell;
use std::env;
use std::fs::create_dir_all;
use std::path::PathBuf;
use structopt::StructOpt;

mod channel_player;
use channel_player::{AudioVisualization, ChannelPlayer, PlaybackState, PlayerEvent, SeekDirection, SubtitleTrack};

use gst_player::PlayerStreamInfoExt;

mod ui_context;
use ui_context::{initialize_and_create_app, UIContext};

#[cfg(target_os = "macos")]
mod iokit_sleep_disabler;

#[derive(StructOpt, Debug)]
#[structopt(name = "Glide")]
struct Opt {
    /// Activate incognito mode. Playback position won't be recorded/loaded to/from the media cache
    #[structopt(short, long)]
    incognito: bool,

    /// Files to play
    #[structopt(name = "FILE", parse(from_os_str))]
    files: Vec<PathBuf>,
}

struct VideoPlayer {
    player: ChannelPlayer,
    ui_context: UIContext,
    fullscreen_action: gio::SimpleAction,
    restore_action: gio::SimpleAction,
    pause_action: gio::SimpleAction,
    seek_forward_action: gio::SimpleAction,
    seek_backward_action: gio::SimpleAction,
    subtitle_action: gio::SimpleAction,
    audio_visualization_action: gio::SimpleAction,
    audio_track_action: gio::SimpleAction,
    video_track_action: gio::SimpleAction,
    open_media_action: gio::SimpleAction,
    open_subtitle_file_action: gio::SimpleAction,
    audio_mute_action: gio::SimpleAction,
    volume_increase_action: gio::SimpleAction,
    volume_decrease_action: gio::SimpleAction,
    dump_pipeline_action: gio::SimpleAction,
    open_sync_window_action: gio::SimpleAction,
    audio_offset_reset_action: gio::SimpleAction,
    subtitle_offset_reset_action: gio::SimpleAction,
    player_receiver: Option<glib::Receiver<PlayerEvent>>,
}

thread_local!(
    static GLOBAL: RefCell<Option<VideoPlayer>> = RefCell::new(None)
);

macro_rules! with_video_player {
    ($player:ident $code: block) => (
        GLOBAL.with(|global| {
            if let Some(ref $player) = *global.borrow() $code
        })
    )
}

macro_rules! with_mut_video_player {
    ($player:ident $code: block) => (
        GLOBAL.with(|global| {
            if let Some(ref mut $player) = *global.borrow_mut() $code
        })
    )
}

// Only possible in nightly
// static SEEK_BACKWARD_OFFSET: gst::ClockTime = gst::ClockTime::from_mseconds(2000);
// static SEEK_FORWARD_OFFSET: gst::ClockTime = gst::ClockTime::from_mseconds(5000);

static SEEK_BACKWARD_OFFSET: gst::ClockTime = gst::ClockTime(Some(2_000_000_000));
static SEEK_FORWARD_OFFSET: gst::ClockTime = gst::ClockTime(Some(5_000_000_000));

impl VideoPlayer {
    pub fn new(gtk_app: gtk::Application, options: &Opt) -> Result<Self, Error> {
        let fullscreen_action = gio::SimpleAction::new_stateful("fullscreen", None, &false.to_variant());
        gtk_app.add_action(&fullscreen_action);

        let restore_action = gio::SimpleAction::new_stateful("restore", None, &true.to_variant());
        gtk_app.add_action(&restore_action);

        let pause_action = gio::SimpleAction::new_stateful("pause", None, &false.to_variant());
        gtk_app.add_action(&pause_action);

        let seek_forward_action = gio::SimpleAction::new_stateful("seek-forward", None, &false.to_variant());
        gtk_app.add_action(&seek_forward_action);

        let seek_backward_action = gio::SimpleAction::new_stateful("seek-backward", None, &false.to_variant());
        gtk_app.add_action(&seek_backward_action);

        let open_media_action = gio::SimpleAction::new("open-media", None);
        gtk_app.add_action(&open_media_action);

        let open_subtitle_file_action = gio::SimpleAction::new("open-subtitle-file", None);
        gtk_app.add_action(&open_subtitle_file_action);

        let audio_mute_action = gio::SimpleAction::new_stateful("audio-mute", None, &false.to_variant());
        gtk_app.add_action(&audio_mute_action);

        let volume_increase_action =
            gio::SimpleAction::new_stateful("audio-volume-increase", None, &false.to_variant());
        gtk_app.add_action(&volume_increase_action);

        let volume_decrease_action =
            gio::SimpleAction::new_stateful("audio-volume-decrease", None, &false.to_variant());
        gtk_app.add_action(&volume_decrease_action);

        let dump_pipeline_action = gio::SimpleAction::new_stateful("dump-pipeline", None, &false.to_variant());
        gtk_app.add_action(&dump_pipeline_action);

        let subtitle_action =
            gio::SimpleAction::new_stateful("subtitle", glib::VariantTy::new("s").ok(), &"".to_variant());
        gtk_app.add_action(&subtitle_action);

        let audio_visualization_action = gio::SimpleAction::new_stateful(
            "audio-visualization",
            glib::VariantTy::new("s").ok(),
            &"none".to_variant(),
        );
        gtk_app.add_action(&audio_visualization_action);

        let audio_track_action =
            gio::SimpleAction::new_stateful("audio-track", glib::VariantTy::new("s").ok(), &"audio-0".to_variant());
        gtk_app.add_action(&audio_track_action);

        let video_track_action =
            gio::SimpleAction::new_stateful("video-track", glib::VariantTy::new("s").ok(), &"video-0".to_variant());
        gtk_app.add_action(&video_track_action);

        let open_sync_window_action = gio::SimpleAction::new("open-sync-window", None);
        gtk_app.add_action(&open_sync_window_action);

        let audio_offset_reset_action = gio::SimpleAction::new("audio-offset-reset", None);
        gtk_app.add_action(&audio_offset_reset_action);

        let subtitle_offset_reset_action = gio::SimpleAction::new("subtitle-offset-reset", None);
        gtk_app.add_action(&subtitle_offset_reset_action);

        let about = gio::SimpleAction::new("about", None);
        about.connect_activate(move |_, _| {
            with_video_player!(video_player {
                video_player.ui_context.display_about_dialog();
            });
        });
        gtk_app.add_action(&about);

        gtk_app.connect_activate(|_| {
            with_mut_video_player!(player {
                player.start();
            })
        });

        let quit = gio::SimpleAction::new("quit", None);
        quit.connect_activate(|_, _| {
            with_video_player!(video_player {
                video_player.quit();
            });
        });
        gtk_app.add_action(&quit);

        gtk_app.connect_open(move |app, files, _| {
            app.activate();
            with_mut_video_player!(player {
                player.open_files(files);
            });
        });

        let ui_context = UIContext::new(gtk_app);

        let (player_sender, player_receiver) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);

        let mut cache_file_path = None;
        if !options.incognito {
            if let Some(d) = ProjectDirs::from("net", "baseart", "Glide") {
                create_dir_all(d.cache_dir()).unwrap();
                cache_file_path = Some(d.cache_dir().join("media-cache.json"));
            }
        }

        let player = ChannelPlayer::new(player_sender, cache_file_path)?;

        Ok(Self {
            player,
            ui_context,
            fullscreen_action,
            restore_action,
            pause_action,
            seek_forward_action,
            seek_backward_action,
            subtitle_action,
            audio_visualization_action,
            audio_track_action,
            video_track_action,
            open_media_action,
            open_subtitle_file_action,
            audio_mute_action,
            volume_increase_action,
            volume_decrease_action,
            dump_pipeline_action,
            open_sync_window_action,
            audio_offset_reset_action,
            subtitle_offset_reset_action,
            player_receiver: Some(player_receiver),
        })
    }

    pub fn quit(&self) {
        self.player.write_last_known_media_position();
        self.leave_fullscreen();
        self.ui_context.stop();
        println!("bye!")
    }

    pub fn start(&mut self) {
        let player_receiver = self.player_receiver.take().expect("No player channel receiver");
        player_receiver.attach(None, move |event| {
            with_video_player!(player {
                player.dispatch_event(event);
            });
            glib::Continue(true)
        });

        self.pause_action.connect_change_state(|pause_action, _| {
            if let Some(is_paused) = pause_action.get_state() {
                let paused = is_paused.get::<bool>().unwrap();

                with_video_player!(video_player {
                    video_player.player.toggle_pause(paused);
                });
                pause_action.set_state(&(!paused).to_variant());
            }
        });

        self.dump_pipeline_action.connect_activate(|_, _| {
            with_video_player!(video_player {
                video_player.player.dump_pipeline("glide");
            });
        });

        self.seek_forward_action.connect_change_state(|_, _| {
            with_video_player!(video_player {
                video_player.player.seek(&SeekDirection::Forward(SEEK_FORWARD_OFFSET));
            });
        });

        self.seek_backward_action.connect_change_state(|_, _| {
            with_video_player!(video_player {
                video_player.player.seek(&SeekDirection::Backward(SEEK_BACKWARD_OFFSET));
            });
        });

        self.volume_decrease_action.connect_change_state(|_, _| {
            with_video_player!(video_player {
                    video_player.player.decrease_volume();
            });
        });

        self.volume_increase_action.connect_change_state(|_, _| {
            with_video_player!(video_player {
                video_player.player.increase_volume();
            });
        });

        self.audio_mute_action.connect_change_state(|mute_action, _| {
            with_video_player!(video_player {
                if let Some(is_enabled) = mute_action.get_state() {
                    let enabled = is_enabled.get::<bool>().unwrap();
                    video_player.player.toggle_mute(!enabled);
                    mute_action.set_state(&(!enabled).to_variant());
                }
            });
        });

        self.fullscreen_action.connect_change_state(|fullscreen_action, _| {
            if let Some(is_fullscreen) = fullscreen_action.get_state() {
                with_video_player!(video_player {
                    let fullscreen = is_fullscreen.get::<bool>().unwrap();
                    if !fullscreen {
                        video_player.ui_context.enter_fullscreen();
                    } else {
                        video_player.ui_context.leave_fullscreen();
                    }
                    let new_state = !fullscreen;
                    fullscreen_action.set_state(&new_state.to_variant());
                });
            }
        });

        self.restore_action.connect_change_state(|_, _| {
            with_video_player!(video_player {
                video_player.leave_fullscreen();
            });
        });

        self.subtitle_action.connect_change_state(|_, value| {
            with_video_player!(video_player {
                video_player.update_subtitle_track(value);
            });
        });

        self.audio_visualization_action.connect_change_state(|action, value| {
            if let Some(val) = value {
                if let Some(name) = val.get::<std::string::String>() {
                    with_video_player!(video_player {
                        if name == "none" {
                            video_player.player.set_audio_visualization(None);
                        } else {
                            video_player.player.set_audio_visualization(Some(AudioVisualization(name)));
                        }
                        action.set_state(&val);
                    });
                }
            }
        });

        self.audio_track_action.connect_change_state(|action, value| {
            if let Some(val) = value {
                if let Some(idx) = val.get::<std::string::String>() {
                    let (_prefix, idx) = idx.split_at(6);
                    let idx = idx.parse::<i32>().unwrap();

                    with_video_player!(video_player {
                        video_player.player.set_audio_track_index(idx);
                        action.set_state(&val);
                    });
                }
            }
        });

        self.video_track_action.connect_change_state(|action, value| {
            if let Some(val) = value {
                if let Some(idx) = val.get::<std::string::String>() {
                    let (_prefix, idx) = idx.split_at(6);
                    let idx = idx.parse::<i32>().unwrap();

                    with_video_player!(video_player {
                        video_player.player.set_video_track_index(idx);
                        action.set_state(&val);
                    });
                }
            }
        });

        self.open_media_action.connect_activate(|_, _| {
            with_video_player!(video_player {
                if let Some(uri) = video_player.ui_context.dialog_result(video_player.player.get_current_uri()) {
                    println!("loading {}", &uri);
                    video_player.player.stop();
                    video_player.player.load_uri(&uri);
                }
            });
        });

        self.open_subtitle_file_action.connect_activate(|_, _| {
            with_video_player!(video_player {
                if let Some(uri) = video_player.ui_context.dialog_result(video_player.player.get_current_uri()) {
                    video_player.player.configure_subtitle_track(Some(SubtitleTrack::External(uri)));
                }
                video_player.refresh_subtitle_track_menu();
            });
        });

        self.open_sync_window_action.connect_activate(|_, _| {
            with_video_player!(video_player {
                video_player.ui_context.open_track_synchronization_window();
            });
        });

        self.audio_offset_reset_action.connect_activate(|_, _| {
            with_video_player!(video_player {
                video_player.player.set_audio_offset(0);
            })
        });

        self.subtitle_offset_reset_action.connect_activate(|_, _| {
            with_video_player!(video_player {
                video_player.player.set_subtitle_offset(0);
            })
        });

        self.ui_context.set_video_area(self.player.video_area());

        self.ui_context.set_progress_bar_format_callback(|value, duration| {
            let position = gst::ClockTime::from_seconds(value as u64);
            let duration = gst::ClockTime::from_seconds(duration as u64);
            if duration.is_some() {
                format!("{:.0} / {:.0}", position, duration)
            } else {
                format!("{:.0}", position)
            }
        });

        self.ui_context.set_volume_value_changed_callback(|value| {
            with_video_player!(video_player {
                video_player.player.set_volume(value);
            });
        });

        self.ui_context.set_position_changed_callback(|value| {
            with_video_player!(video_player {
                video_player.player.seek_to(gst::ClockTime::from_seconds(value));
            });
        });

        self.ui_context.set_drop_data_callback(|uri| {
            with_video_player!(video_player {
                println!("loading {}", &uri);
                video_player.player.stop();
                video_player.player.load_uri(&uri);
            })
        });

        self.ui_context.set_audio_offset_entry_updated_callback(|offset| {
            with_video_player!(video_player {
                video_player.player.set_audio_offset(offset);
            })
        });

        self.ui_context.set_subtitle_offset_entry_updated_callback(|offset| {
            with_video_player!(video_player {
                video_player.player.set_subtitle_offset(offset);
            })
        });

        #[cfg(feature = "self-updater")]
        match self.check_update() {
            Ok(o) => {
                match o {
                    self_update::Status::UpToDate(_version) => {}
                    _ => println!("Update succeeded: {}", o),
                };
            }
            Err(e) => eprintln!("Update failed: {}", e),
        };

        self.ui_context.start(|| {
            with_video_player!(video_player {
                video_player.quit();
            });
        });
    }

    pub fn dispatch_event(&self, event: PlayerEvent) {
        match event {
            PlayerEvent::MediaInfoUpdated => {
                self.media_info_updated();
            }
            PlayerEvent::PositionUpdated => {
                self.position_updated();
            }
            PlayerEvent::VideoDimensionsChanged(width, height) => {
                self.video_dimensions_changed(width, height);
            }
            PlayerEvent::StateChanged(ref s) => {
                self.playback_state_changed(s);
            }
            PlayerEvent::VolumeChanged(volume) => {
                self.volume_changed(volume);
            }
            PlayerEvent::Error(msg) => {
                self.player_error(msg);
            }
            PlayerEvent::AudioVideoOffsetChanged(offset) => {
                self.audio_video_offset_changed(offset);
            }
            PlayerEvent::SubtitleVideoOffsetChanged(offset) => {
                self.subtitle_video_offset_changed(offset);
            }
            _ => {}
        };
    }

    pub fn player_error(&self, msg: std::string::String) {
        // FIXME: display some GTK error dialog...
        eprintln!("Internal player error: {}", msg);
        with_video_player!(video_player { video_player.quit() });
    }

    pub fn volume_changed(&self, volume: f64) {
        self.ui_context.volume_changed(volume);
    }

    pub fn audio_video_offset_changed(&self, offset: i64) {
        self.ui_context.audio_video_offset_changed(offset);
    }

    pub fn subtitle_video_offset_changed(&self, offset: i64) {
        self.ui_context.subtitle_video_offset_changed(offset);
    }

    pub fn playback_state_changed(&self, playback_state: &PlaybackState) {
        self.ui_context.playback_state_changed(playback_state);
    }

    pub fn video_dimensions_changed(&self, width: i32, height: i32) {
        self.ui_context.resize_window(width, height);
    }

    pub fn media_info_updated(&self) {
        if let Some(info) = self.player.get_media_info() {
            if let Some(uri) = self.player.get_current_uri() {
                if let Some(title) = info.get_title() {
                    self.ui_context.set_window_title(&*title);
                } else if let Ok((filename, _)) = glib::filename_from_uri(&uri) {
                    self.ui_context
                        .set_window_title(&filename.as_os_str().to_string_lossy());
                } else {
                    self.ui_context.set_window_title(&uri);
                }

                if let Some(duration) = info.get_duration().seconds() {
                    self.ui_context.set_position_range_end(duration as f64);
                }

                // Look for a matching subtitle file in same directory.
                if let Ok((mut path, _)) = glib::filename_from_uri(&uri) {
                    path.set_extension("srt");
                    let subfile = path.as_path();
                    if subfile.is_file() {
                        if let Ok(suburi) = glib::filename_to_uri(subfile, None) {
                            self.player
                                .configure_subtitle_track(Some(SubtitleTrack::External(suburi)));
                        }
                    }
                }
            }
            self.refresh_subtitle_track_menu();
            self.fill_audio_track_menu(&info);
            self.fill_video_track_menu(&info);

            if info.get_number_of_video_streams() == 0 {
                self.fill_audio_visualization_menu();
                // TODO: Might be nice to enable the first audio
                // visualization by default but it doesn't work
                // yet. See also
                // https://bugzilla.gnome.org/show_bug.cgi?id=796552
                self.audio_visualization_action.set_enabled(true);
            } else {
                self.ui_context.clear_audio_visualization_menu();
                self.audio_visualization_action.set_enabled(false);
            }
        }
    }

    pub fn position_updated(&self) {
        if let Some(position) = self.player.get_position().seconds() {
            self.ui_context.set_position_range_value(position);
        }
    }

    pub fn update_subtitle_track(&self, value: Option<&glib::Variant>) {
        if let Some(val) = value {
            if let Some(val) = val.get::<std::string::String>() {
                let track = if val == "none" {
                    None
                } else {
                    let (prefix, asset) = val.split_at(4);
                    if prefix == "ext-" {
                        Some(SubtitleTrack::External(asset.into()))
                    } else {
                        let idx = asset.parse::<i32>().unwrap();
                        Some(SubtitleTrack::Inband(idx))
                    }
                };
                self.player.configure_subtitle_track(track);
            }
            self.subtitle_action.set_state(&val);
        }
    }

    pub fn refresh_subtitle_track_menu(&self) {
        let section = gio::Menu::new();
        let mut selected_action: Option<std::string::String> = None;

        if let Some(info) = self.player.get_media_info() {
            let item = gio::MenuItem::new(Some("Disable"), Some("none"));
            item.set_detailed_action("app.subtitle::none");
            section.append_item(&item);

            let current_subtitle_track = self.player.get_current_subtitle_track();
            for (i, sub_stream) in info.get_subtitle_streams().into_iter().enumerate() {
                let default_title = format!("Track {}", i + 1);
                let title = match sub_stream.get_tags() {
                    Some(tags) => match tags.get::<gst::tags::Title>() {
                        Some(val) => std::string::String::from(val.get().unwrap()),
                        None => default_title,
                    },
                    None => default_title,
                };
                let lang = sub_stream.get_language().map(|l| {
                    if l == title {
                        "".to_string()
                    } else {
                        format!(" - [{}]", l)
                    }
                });

                let action_label = format!("{}{}", title, lang.unwrap_or_else(|| "".to_string()));
                let action_id = format!("app.subtitle::sub-{}", i);
                let item = gio::MenuItem::new(Some(&action_label), Some(&action_id));
                item.set_detailed_action(&*action_id);
                section.append_item(&item);

                if selected_action.is_none() {
                    if let Some(ref track) = current_subtitle_track {
                        if track.get_language() == sub_stream.get_language() {
                            selected_action = Some(format!("sub-{}", i));
                        }
                    }
                }
            }
        }

        if let Some(uri) = self.player.get_subtitle_uri() {
            if let Ok((path, _)) = glib::filename_from_uri(&uri) {
                let subfile = path.as_path();
                if let Some(filename) = subfile.file_name() {
                    if let Some(f) = filename.to_str() {
                        let v = format!("ext-{}", uri);
                        let action_id = format!("app.subtitle::{}", v);
                        let item = gio::MenuItem::new(Some(f), Some(&action_id));
                        item.set_detailed_action(&*action_id);
                        section.append_item(&item);
                        selected_action = Some(v);
                    }
                }
            }
        }

        self.ui_context.update_subtitle_track_menu(&section);

        let v = match selected_action {
            Some(a) => a.to_variant(),
            None => ("none").to_variant(),
        };
        self.subtitle_action.change_state(&v);
    }

    pub fn fill_audio_visualization_menu(&self) {
        if !self.ui_context.mutable_audio_visualization_menu() {
            return;
        }
        let section = gio::Menu::new();

        let item = gio::MenuItem::new(Some("Disable"), Some("none"));
        item.set_detailed_action("app.audio-visualization::none");
        section.append_item(&item);

        for vis in gst_player::Player::visualizations_get() {
            let action_id = format!("app.audio-visualization::{}", vis.name());
            let item = gio::MenuItem::new(Some(vis.description()), Some(&action_id));
            item.set_detailed_action(&*action_id);
            section.append_item(&item);
        }

        self.ui_context.update_audio_visualization_menu(&section);
    }

    pub fn fill_audio_track_menu(&self, info: &gst_player::PlayerMediaInfo) {
        let section = gio::Menu::new();

        let item = gio::MenuItem::new(Some("Disable"), Some("subtitle"));
        item.set_detailed_action("app.audio-track::audio--1");
        section.append_item(&item);

        for (i, audio_stream) in info.get_audio_streams().iter().enumerate() {
            let mut label = format!("{} channels", audio_stream.get_channels());
            if let Some(l) = audio_stream.get_language() {
                label = format!("{} - [{}]", label, l);
            }
            let action_id = format!("app.audio-track::audio-{}", i);
            let item = gio::MenuItem::new(Some(&label), Some(&action_id));
            item.set_detailed_action(&*action_id);
            section.append_item(&item);
        }
        self.ui_context.update_audio_track_menu(&section);
    }

    pub fn fill_video_track_menu(&self, info: &gst_player::PlayerMediaInfo) {
        let section = gio::Menu::new();

        let item = gio::MenuItem::new(Some("Disable"), Some("subtitle"));
        item.set_detailed_action("app.video-track::video--1");
        section.append_item(&item);

        for (i, video_stream) in info.get_video_streams().iter().enumerate() {
            let action_id = format!("app.video-track::video-{}", i);
            let description = format!("{}x{}", video_stream.get_width(), video_stream.get_height());
            let item = gio::MenuItem::new(Some(&description), Some(&action_id));
            item.set_detailed_action(&*action_id);
            section.append_item(&item);
        }
        self.ui_context.update_video_track_menu(&section);
    }

    pub fn open_files(&mut self, files: &[gio::File]) {
        let mut playlist = vec![];
        for file in files.to_vec() {
            let uri = if let Some(_path) = file.get_path() {
                Some(std::string::String::from(file.get_uri().as_str()))
            } else {
                // Gio built an invalid URI, so try to find the original CLI
                // argument based on the URI scheme.
                let uri_scheme: std::string::String = file.get_uri_scheme().into();
                let args = env::args().collect::<Vec<_>>();
                let mut args_iter = args.iter();
                let item = args_iter.find(|&i| i.starts_with(uri_scheme.as_str()));
                match item {
                    Some(i) => Some(std::string::String::from(i)),
                    None => None,
                }
            };
            if let Some(uri) = uri {
                playlist.push(uri);
            }
        }

        self.player.load_playlist(playlist);
    }

    #[cfg(feature = "self-updater")]
    pub fn check_update(&self) -> Result<self_update::Status, self_update::errors::Error> {
        let target = self_update::get_target()?;
        if let Ok(mut b) = self_update::backends::github::Update::configure() {
            return b
                .repo_owner("philn")
                .repo_name("glide")
                .bin_name("glide")
                .target(&target)
                .current_version(cargo_crate_version!())
                .build()?
                .update();
        }

        Ok(self_update::Status::UpToDate(std::string::String::from("OK")))
    }

    pub fn leave_fullscreen(&self) {
        let fullscreen_action = &self.fullscreen_action;
        if let Some(is_fullscreen) = fullscreen_action.get_state() {
            let fullscreen = is_fullscreen.get::<bool>().unwrap();

            if fullscreen {
                self.ui_context.leave_fullscreen();
                fullscreen_action.set_state(&false.to_variant());
            }
        }
    }
}

fn main() -> Result<(), Error> {
    #[cfg(not(unix))]
    {
        return Err(failure::err_msg("Add support for target platform"));
    }

    gst::init().expect("Failed to initialize GStreamer.");

    glib::set_application_name("Glide");

    let opt = Opt::from_args();

    let gtk_app = initialize_and_create_app();

    let gtk_app_clone = gtk_app.clone();
    let app = VideoPlayer::new(gtk_app, &opt)?;

    GLOBAL.with(move |global| {
        *global.borrow_mut() = Some(app);
    });

    let files: Vec<std::string::String> = opt
        .files
        .iter()
        .map(|p| std::string::String::from(p.to_str().unwrap()))
        .collect();

    let mut args = vec![env::args().next().unwrap()];
    args.extend(files);
    gtk_app_clone.run(&args);

    Ok(())
    // unsafe {
    //     gst::deinit();
    // }
}
