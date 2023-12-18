extern crate anyhow;
#[cfg(target_os = "macos")]
extern crate core_foundation;
extern crate directories;
extern crate gio;
extern crate gstreamer as gst;
extern crate gstreamer_play as gst_play;
extern crate gstreamer_video as gst_video;
extern crate gtk4 as gtk;
#[macro_use]
extern crate lazy_static;
#[cfg(feature = "self-updater")]
#[macro_use]
extern crate self_update;

#[macro_use]
extern crate serde_derive;

use crate::gst_play::prelude::PlayStreamInfoExt;
use gstreamer::glib;

use clap::Parser;
use directories::ProjectDirs;
#[allow(unused_imports)]
use gdk::prelude::*;
use gio::prelude::*;
use glib::ToVariant;
use gtk::gdk;
use std::cell::RefCell;
use std::env;
use std::fs::create_dir_all;
use std::path::PathBuf;

mod channel_player;
mod constants;
mod debug_infos;
use channel_player::{AudioVisualization, ChannelPlayer, PlaybackState, PlayerEvent, SeekDirection, SubtitleTrack};
mod ui_context;
use ui_context::{create_app, UIContext};

#[cfg(target_os = "macos")]
mod iokit_sleep_disabler;

#[derive(clap::Parser, Debug)]
#[clap(about, version, author)]
struct Opt {
    /// Activate incognito mode. Playback position won't be recorded/loaded to/from the media cache
    #[clap(short, long)]
    incognito: bool,

    /// Files to play
    #[clap(name = "FILE", value_parser)]
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
    show_shortcuts_action: gio::SimpleAction,
    audio_offset_reset_action: gio::SimpleAction,
    subtitle_offset_reset_action: gio::SimpleAction,
    video_frame_step_action: gio::SimpleAction,
    speed_increase_action: gio::SimpleAction,
    speed_decrease_action: gio::SimpleAction,
    player_receiver: Option<async_channel::Receiver<PlayerEvent>>,
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

macro_rules! with_optional_video_player {
    ($player:ident $code: block $else_code: block) => (
        GLOBAL.with(|global| {
            if let Some(ref $player) = *global.borrow() $code else $else_code
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

impl VideoPlayer {
    pub fn new(gtk_app: adw::Application, options: &Opt) -> anyhow::Result<Self> {
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

        let show_shortcuts_action = gio::SimpleAction::new("show-shortcuts", None);
        gtk_app.add_action(&show_shortcuts_action);

        let audio_offset_reset_action = gio::SimpleAction::new("audio-offset-reset", None);
        gtk_app.add_action(&audio_offset_reset_action);

        let subtitle_offset_reset_action = gio::SimpleAction::new("subtitle-offset-reset", None);
        gtk_app.add_action(&subtitle_offset_reset_action);

        let video_frame_step_action = gio::SimpleAction::new("video-frame-step", None);
        gtk_app.add_action(&video_frame_step_action);

        let speed_increase_action = gio::SimpleAction::new("speed-increase", None);
        gtk_app.add_action(&speed_increase_action);

        let speed_decrease_action = gio::SimpleAction::new("speed-decrease", None);
        gtk_app.add_action(&speed_decrease_action);

        let about = gio::SimpleAction::new("about", None);
        about.connect_activate(move |_, _| {
            with_video_player!(video_player {
                video_player.ui_context.display_about_dialog();
            });
        });
        gtk_app.add_action(&about);

        gtk_app.connect_startup(|_| {
            with_mut_video_player!(player {
                player.start();
            });
            with_video_player!(player {
                player.configure();
            })
        });

        gtk_app.connect_activate(|_| {});

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

        let (player_sender, player_receiver) = async_channel::unbounded();

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
            show_shortcuts_action,
            audio_offset_reset_action,
            subtitle_offset_reset_action,
            video_frame_step_action,
            speed_increase_action,
            speed_decrease_action,
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
        glib::MainContext::default().spawn_local(async move {
            while let Ok(event) = player_receiver.recv().await {
                with_video_player!(player {
                    player.dispatch_event(event);
                });
            }
        });

        self.pause_action.connect_change_state(|pause_action, _| {
            if let Some(is_paused) = pause_action.state() {
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
                video_player.player.seek(&SeekDirection::Forward(constants::SEEK_FORWARD_OFFSET));
            });
        });

        self.seek_backward_action.connect_change_state(|_, _| {
            with_video_player!(video_player {
                video_player.player.seek(&SeekDirection::Backward(constants::SEEK_BACKWARD_OFFSET));
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
                if let Some(is_enabled) = mute_action.state() {
                    let enabled = is_enabled.get::<bool>().unwrap();
                    video_player.player.toggle_mute(!enabled);
                    mute_action.set_state(&(!enabled).to_variant());
                }
            });
        });

        self.fullscreen_action.connect_change_state(|fullscreen_action, _| {
            if let Some(is_fullscreen) = fullscreen_action.state() {
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
                        let fallback_to_first_vis = || {
                            if let Some(vis) = gst_play::Play::visualizations_get().first() {
                                video_player.player
                                    .set_audio_visualization(Some(AudioVisualization(vis.name().to_string())));
                            } else {
                                video_player.player.set_audio_visualization(None);
                            }
                        };

                        if name == "none" {
                            match video_player.player.get_audio_track_cover() {
                                Some(sample) => {
                                    match video_player.ui_context.set_audio_cover_art(&sample) {
                                        Ok(_) => {}
                                        Err(_) => {
                                            fallback_to_first_vis();
                                        }
                                    };
                                }
                                None => {
                                    fallback_to_first_vis();
                                }
                            };
                        } else {
                            let paintable = video_player.player.paintable();
                            video_player.ui_context.set_video_paintable(&paintable);
                            video_player.player.set_audio_visualization(Some(AudioVisualization(name)));
                        }
                        action.set_state(val);
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
                        action.set_state(val);
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
                        action.set_state(val);
                    });
                }
            }
        });

        self.open_media_action.connect_activate(|_, _| {
            with_video_player!(video_player {
                video_player.ui_context.open_dialog(video_player.player.get_current_uri(), |uri| {
                    with_video_player!(video_player {
                        println!("loading {}", &uri);
                        video_player.player.stop();
                        video_player.player.load_uri(&uri);
                    });
                });
            });
        });

        self.open_subtitle_file_action.connect_activate(|_, _| {
            with_video_player!(video_player {
                video_player.ui_context.open_dialog(video_player.player.get_current_uri(), |uri| {
                    with_video_player!(video_player {
                        video_player.player.configure_subtitle_track(Some(SubtitleTrack::External(uri)));
                        video_player.refresh_subtitle_track_menu();
                    });
                });
            });
        });

        self.open_sync_window_action.connect_activate(|_, _| {
            with_video_player!(video_player {
                video_player.ui_context.open_track_synchronization_window();
            });
        });

        self.show_shortcuts_action.connect_activate(|_, _| {
            with_video_player!(video_player {
                video_player.ui_context.show_shortcuts();
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

        self.video_frame_step_action.connect_activate(|_, _| {
            with_video_player!(video_player {
                if let Some(is_paused) = video_player.pause_action.state() {
                    if !is_paused.get::<bool>().unwrap() {
                        video_player.player.toggle_pause(false);
                        video_player.pause_action.set_state(&(true).to_variant());
                    }
                }
                video_player.player.video_frame_step();
            })
        });

        self.speed_decrease_action.connect_activate(|_, _| {
            with_video_player!(video_player {
                video_player.player.decrease_speed();
            });
        });

        self.speed_increase_action.connect_activate(|_, _| {
            with_video_player!(video_player {
                video_player.player.increase_speed();
            });
        });

        let paintable = self.player.paintable();
        paintable.connect_invalidate_contents(|p| {
            with_video_player!(video_player {
                video_player.player.update_render_rectangle(p);
            })
        });
        self.ui_context.set_video_paintable(&paintable);

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
                if let Ok((path, _)) = glib::filename_from_uri(uri) {
                    if let Some(extension) = path.extension() {
                        if constants::SUB_FILE_EXTENSIONS.contains(&extension.to_str().unwrap()) {
                            video_player.player
                                .configure_subtitle_track(Some(SubtitleTrack::External(uri.into())));
                            return;
                        }
                    }
                }
                println!("loading {}", &uri);
                video_player.player.stop();
                video_player.player.load_uri(uri);
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

    pub fn configure(&self) {
        self.ui_context.set_progress_bar_format_callback(|value| {
            let position = gst::ClockTime::from_seconds(value as u64);
            with_optional_video_player!(video_player {
                let status = if let Some(duration) = video_player.player.duration() {
                    format!("{position:.0} / {duration:.0}")
                } else {
                    format!("{position:.0}")
                };
                // FIXME: Ideally it'd be nice to show this on an OSD overlay?
                let playback_rate = video_player.player.playback_rate();
                if playback_rate != 1.0 {
                    format!("{status} @ {playback_rate}.x")
                } else {
                    status
                }
            } {
                format!("{position:.0}")
            })
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
        eprintln!("Internal player error: {msg}");
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

    pub fn video_dimensions_changed(&self, width: u32, height: u32) {
        self.ui_context.resize_window(width, height);
    }

    pub fn media_info_updated(&self) {
        if let Some(info) = self.player.get_media_info() {
            if let Some(uri) = self.player.get_current_uri() {
                if let Some(title) = info.title() {
                    self.ui_context.set_window_title(&title);
                } else if let Ok((filename, _)) = glib::filename_from_uri(&uri) {
                    self.ui_context
                        .set_window_title(&filename.as_os_str().to_string_lossy());
                } else {
                    self.ui_context.set_window_title(&uri);
                }

                if let Some(duration) = info.duration() {
                    self.ui_context.set_position_range_end(duration.seconds() as f64);
                }

                // Look for a matching subtitle file in same directory.
                if let Ok((mut path, _)) = glib::filename_from_uri(&uri) {
                    for extension in constants::SUB_FILE_EXTENSIONS.iter() {
                        path.set_extension(extension);
                        let subfile = path.as_path();
                        if subfile.is_file() {
                            if let Ok(suburi) = glib::filename_to_uri(subfile, None) {
                                self.player
                                    .configure_subtitle_track(Some(SubtitleTrack::External(suburi)));
                                break;
                            }
                        }
                    }
                }
            }
            self.refresh_subtitle_track_menu();
            self.fill_audio_track_menu(&info);
            self.fill_video_track_menu(&info);

            if info.number_of_video_streams() == 0 {
                self.fill_audio_visualization_menu();
                self.audio_visualization_action.set_enabled(true);
                self.audio_visualization_action.activate(Some(&"none".to_variant()));
            } else {
                self.ui_context.clear_audio_visualization_menu();
                self.audio_visualization_action.set_enabled(false);
            }
        }
    }

    pub fn position_updated(&self) {
        if let Some(position) = self.player.get_position() {
            self.ui_context.set_position_range_value(position.seconds());
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
            self.subtitle_action.set_state(val);
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
            for (i, sub_stream) in info.subtitle_streams().into_iter().enumerate() {
                let default_title = format!("Track {}", i + 1);
                let title = match sub_stream.tags() {
                    Some(tags) => match tags.get::<gst::tags::Title>() {
                        Some(val) => std::string::String::from(val.get()),
                        None => default_title,
                    },
                    None => default_title,
                };
                let lang = sub_stream.language().map(|l| {
                    if l == title {
                        "".to_string()
                    } else {
                        format!(" - [{l}]")
                    }
                });

                let action_label = format!("{}{}", title, lang.unwrap_or_default());
                let action_id = format!("app.subtitle::sub-{i}");
                let item = gio::MenuItem::new(Some(&action_label), Some(&action_id));
                item.set_detailed_action(&action_id);
                section.append_item(&item);

                if selected_action.is_none() {
                    if let Some(ref track) = current_subtitle_track {
                        if track.language() == sub_stream.language() {
                            selected_action = Some(format!("sub-{i}"));
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
                        let v = format!("ext-{uri}");
                        let action_id = format!("app.subtitle::{v}");
                        let item = gio::MenuItem::new(Some(f), Some(&action_id));
                        item.set_detailed_action(&action_id);
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

        for vis in gst_play::Play::visualizations_get() {
            let action_id = format!("app.audio-visualization::{}", vis.name());
            let item = gio::MenuItem::new(Some(vis.description()), Some(&action_id));
            item.set_detailed_action(&action_id);
            section.append_item(&item);
        }

        self.ui_context.update_audio_visualization_menu(&section);
    }

    pub fn fill_audio_track_menu(&self, info: &gst_play::PlayMediaInfo) {
        let section = gio::Menu::new();

        let item = gio::MenuItem::new(Some("Disable"), Some("subtitle"));
        item.set_detailed_action("app.audio-track::audio--1");
        section.append_item(&item);

        for (i, audio_stream) in info.audio_streams().iter().enumerate() {
            let mut label = format!("{} channels", audio_stream.channels());
            if let Some(l) = audio_stream.language() {
                label = format!("{label} - [{l}]");
            }
            let action_id = format!("app.audio-track::audio-{i}");
            let item = gio::MenuItem::new(Some(&label), Some(&action_id));
            item.set_detailed_action(&action_id);
            section.append_item(&item);
        }
        self.ui_context.update_audio_track_menu(&section);
    }

    pub fn fill_video_track_menu(&self, info: &gst_play::PlayMediaInfo) {
        let section = gio::Menu::new();

        let item = gio::MenuItem::new(Some("Disable"), Some("subtitle"));
        item.set_detailed_action("app.video-track::video--1");
        section.append_item(&item);

        for (i, video_stream) in info.video_streams().iter().enumerate() {
            let action_id = format!("app.video-track::video-{i}");
            let description = format!("{}x{}", video_stream.width(), video_stream.height());
            let item = gio::MenuItem::new(Some(&description), Some(&action_id));
            item.set_detailed_action(&action_id);
            section.append_item(&item);
        }
        self.ui_context.update_video_track_menu(&section);
    }

    pub fn open_files(&mut self, files: &[gio::File]) {
        let mut playlist = vec![];
        for file in files {
            let uri = if let Some(_path) = file.path() {
                Some(std::string::String::from(file.uri().as_str()))
            } else {
                // Gio built an invalid URI, so try to find the original CLI
                // argument based on the URI scheme.
                let uri_scheme = file.uri_scheme().unwrap();
                let args = env::args().collect::<Vec<_>>();
                let mut args_iter = args.iter();
                let item = args_iter.find(|&i| i.starts_with(uri_scheme.as_str()));
                item.map(std::string::String::from)
            };
            if let Some(uri) = uri {
                playlist.push(uri);
            }
        }

        self.player.load_playlist(playlist);
    }

    #[cfg(feature = "self-updater")]
    pub fn check_update(&self) -> Result<self_update::Status, self_update::errors::Error> {
        let target = self_update::get_target();
        self_update::backends::github::Update::configure()
            .repo_owner("philn")
            .repo_name("glide")
            .bin_name("glide")
            .target(&target)
            .current_version(cargo_crate_version!())
            .build()?
            .update()
    }

    pub fn leave_fullscreen(&self) {
        let fullscreen_action = &self.fullscreen_action;
        if let Some(is_fullscreen) = fullscreen_action.state() {
            let fullscreen = is_fullscreen.get::<bool>().unwrap();

            if fullscreen {
                self.ui_context.leave_fullscreen();
                fullscreen_action.set_state(&false.to_variant());
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    #[cfg(not(unix))]
    {
        return Err(anyhow::anyhow!("Add support for target platform"));
    }

    gst::init().expect("Failed to initialize GStreamer.");
    gtk::init().expect("Failed to initialize GTK.");
    gstgtk4::plugin_register_static().expect("Failed to register gstgtk4 plugin.");

    glib::set_application_name("Glide");

    let gtk_app = create_app();

    let gtk_app_clone = gtk_app.clone();
    let opt = Opt::parse();
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
    gtk_app_clone.run_with_args(&args);

    Ok(())
    // unsafe {
    //     gst::deinit();
    // }
}
