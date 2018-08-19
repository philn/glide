#[cfg(target_os = "macos")]
extern crate core_foundation;
extern crate dirs;
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
#[macro_use]
extern crate self_update;
extern crate gobject_sys;

#[macro_use]
extern crate serde_derive;

#[allow(unused_imports)]
use gdk::prelude::*;
use gio::prelude::*;
use gio::MenuExt;
use gio::MenuItemExt;
use gtk::prelude::*;
use std::cell::RefCell;
use std::cmp;
use std::env;
#[allow(unused_imports)]
use std::os::raw::c_void;
use std::sync::mpsc;
use std::{thread, time};

mod channel_player;
use channel_player::{ChannelPlayer, PlaybackState, PlayerEvent, SubtitleTrack};

use gst_player::PlayerStreamInfoExt;

mod ui_context;
use ui_context::UIContext;

mod common;
use common::SeekDirection;

#[cfg(target_os = "macos")]
mod iokit_sleep_disabler;

#[derive(Serialize, Deserialize)]
enum UIAction {
    ForwardedPlayerEvent(PlayerEvent),
    Quit,
}

struct VideoPlayer {
    player_context: Option<ChannelPlayer>,
    ui_context: Option<UIContext>,
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
    subtitle_track_menu: gio::Menu,
    audio_visualization_menu: gio::Menu,
    audio_track_menu: gio::Menu,
    video_track_menu: gio::Menu,
    volume_signal_handler_id: Option<glib::SignalHandlerId>,
    position_signal_handler_id: Option<glib::SignalHandlerId>,
}

struct AppState {
    sender: mpsc::Sender<UIAction>,
    receiver: mpsc::Receiver<UIAction>,
    app: gtk::Application,
}

thread_local!(
    static GLOBAL: RefCell<Option<(VideoPlayer, AppState)>> = RefCell::new(None)
);

// Only possible in nightly
// static SEEK_BACKWARD_OFFSET: gst::ClockTime = gst::ClockTime::from_mseconds(2000);
// static SEEK_FORWARD_OFFSET: gst::ClockTime = gst::ClockTime::from_mseconds(5000);

static SEEK_BACKWARD_OFFSET: gst::ClockTime = gst::ClockTime(Some(2_000_000_000));
static SEEK_FORWARD_OFFSET: gst::ClockTime = gst::ClockTime(Some(5_000_000_000));

fn set_dialog_folder_relative_to_uri(dialog: &gtk::FileChooserDialog, uri: &str) {
    if let Ok((filename, _)) = glib::filename_from_uri(&uri) {
        if let Some(folder) = filename.parent() {
            dialog.set_current_folder(folder);
        }
    }
}

fn ui_action_handle() -> glib::Continue {
    GLOBAL.with(|global| {
        if let Some((ref player, ref state)) = *global.borrow() {
            if let Ok(action) = &state.receiver.try_recv() {
                match action {
                    UIAction::Quit => {
                        player.quit(state);
                    }
                    UIAction::ForwardedPlayerEvent(event) => {
                        player.dispatch_event(event, state);
                    }
                }
            }
        }
    });
    glib::Continue(false)
}

impl VideoPlayer {
    pub fn new(state: &AppState) -> Self {
        let gtk_app = &state.app;

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

        let subtitle_track_menu = gio::Menu::new();
        let subtitle_action =
            gio::SimpleAction::new_stateful("subtitle", glib::VariantTy::new("s").unwrap(), &"".to_variant());
        gtk_app.add_action(&subtitle_action);

        let audio_visualization_menu = gio::Menu::new();
        let audio_visualization_action = gio::SimpleAction::new_stateful(
            "audio-visualization",
            glib::VariantTy::new("s").unwrap(),
            &"none".to_variant(),
        );
        gtk_app.add_action(&audio_visualization_action);

        let audio_track_menu = gio::Menu::new();
        let audio_track_action = gio::SimpleAction::new_stateful(
            "audio-track",
            glib::VariantTy::new("s").unwrap(),
            &"audio-0".to_variant(),
        );
        gtk_app.add_action(&audio_track_action);

        let video_track_menu = gio::Menu::new();
        let video_track_action = gio::SimpleAction::new_stateful(
            "video-track",
            glib::VariantTy::new("s").unwrap(),
            &"video-0".to_variant(),
        );
        gtk_app.add_action(&video_track_action);

        let about = gio::SimpleAction::new("about", None);
        about.connect_activate(move |_, _| {
            let dialog = gtk::AboutDialog::new();
            dialog.set_authors(&["Philippe Normand"]);
            dialog.set_website_label(Some("base-art.net"));
            dialog.set_website(Some("http://base-art.net"));
            dialog.set_title("About");

            GLOBAL.with(|global| {
                if let Some((ref player, ref _state)) = *global.borrow() {
                    if let Some(ref ui_ctx) = player.ui_context {
                        dialog.set_transient_for(Some(&ui_ctx.window));
                    }
                }
            });
            dialog.run();
            dialog.destroy();
        });
        gtk_app.add_action(&about);

        gtk_app.connect_activate(|_| {
            GLOBAL.with(|global| {
                if let Some((ref mut player, ref state)) = *global.borrow_mut() {
                    player.start(state);
                }
            });
        });

        gtk_app.connect_startup(|app| {
            let quit = gio::SimpleAction::new("quit", None);
            quit.connect_activate(|_, _| {
                GLOBAL.with(|global| {
                    if let Some((ref player, ref state)) = *global.borrow() {
                        player.quit(state);
                    }
                });
            });
            app.add_action(&quit);

            app.set_accels_for_action("app.quit", &*vec!["<Meta>q", "<Ctrl>q"]);
            app.set_accels_for_action("app.fullscreen", &*vec!["<Meta>f", "<Alt>f"]);
            app.set_accels_for_action("app.restore", &*vec!["Escape"]);
            app.set_accels_for_action("app.pause", &*vec!["space"]);
            app.set_accels_for_action("app.seek-forward", &*vec!["<Meta>Right", "<Alt>Right"]);
            app.set_accels_for_action("app.seek-backward", &*vec!["<Meta>Left", "<Alt>Left"]);
            app.set_accels_for_action("app.open-media", &*vec!["<Meta>o", "<Alt>o"]);
            app.set_accels_for_action("app.open-subtitle-file", &*vec!["<Meta>s", "<Alt>s"]);
            app.set_accels_for_action("app.audio-volume-increase", &*vec!["<Meta>Up", "<Alt>Up"]);
            app.set_accels_for_action("app.audio-volume-decrease", &*vec!["<Meta>Down", "<Alt>Down"]);
            app.set_accels_for_action("app.audio-mute", &*vec!["<Meta>m", "<Alt>m"]);
            app.set_accels_for_action("app.dump-pipeline", &*vec!["<Ctrl>d"]);

            let menu = gio::Menu::new();
            let file_menu = gio::Menu::new();
            let audio_menu = gio::Menu::new();
            let video_menu = gio::Menu::new();
            let subtitles_menu = gio::Menu::new();

            #[cfg(not(target_os = "linux"))]
            {
                menu.append("Quit", "app.quit");
                menu.append("About", "app.about");
            }

            GLOBAL.with(|global| {
                if let Some((ref mut player, ref _state)) = *global.borrow_mut() {
                    file_menu.append("Open...", "app.open-media");
                    subtitles_menu.append("Add subtitle file...", "app.open-subtitle-file");
                    subtitles_menu.append_submenu("Subtitle track", &player.subtitle_track_menu);
                    audio_menu.append("Increase Volume", "app.audio-volume-increase");
                    audio_menu.append("Decrease Volume", "app.audio-volume-decrease");
                    audio_menu.append("Mute", "app.audio-mute");
                    audio_menu.append_submenu("Audio track", &player.audio_track_menu);
                    audio_menu.append_submenu("Visualization", &player.audio_visualization_menu);
                    video_menu.append_submenu("Video track", &player.video_track_menu);
                    player.ui_context = Some(UIContext::new(app));
                }
            });

            menu.append_submenu("File", &file_menu);
            menu.append_submenu("Audio", &audio_menu);
            menu.append_submenu("Video", &video_menu);
            menu.append_submenu("Subtitles", &subtitles_menu);

            #[cfg(target_os = "linux")]
            {
                let app_menu = gio::Menu::new();
                // Only static menus here.
                app_menu.append("Quit", "app.quit");
                app_menu.append("About", "app.about");
                app.set_app_menu(&app_menu);
            }
            app.set_menubar(&menu);
        });

        gtk_app.connect_open(move |app, files, _| {
            app.activate();
            GLOBAL.with(|global| {
                if let Some((ref player, ref _state)) = *global.borrow() {
                    player.open_files(files);
                }
            });
        });

        Self {
            player_context: None,
            ui_context: None,
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
            subtitle_track_menu,
            audio_visualization_menu,
            audio_track_menu,
            video_track_menu,
            volume_signal_handler_id: None,
            position_signal_handler_id: None,
        }
    }

    pub fn quit(&self, state: &AppState) {
        if let Some(ref player_context) = self.player_context {
            player_context.write_last_known_media_position();
        }
        self.leave_fullscreen(&state.app);
        state.app.quit();
    }

    pub fn start(&mut self, state: &AppState) {
        let player = ChannelPlayer::new();
        let (sender, receiver) = mpsc::channel();
        player.register_event_handler(sender);
        self.player_context = Some(player);

        let callback = || glib::idle_add(ui_action_handle);

        let sender = state.sender.clone();
        thread::spawn(move || loop {
            if let Ok(event) = receiver.try_recv() {
                // if let PlayerEvent::EndOfPlaylist = event {
                //     sender.send(UIAction::Quit).unwrap();
                //     callback();
                //     break;
                // }
                sender.send(UIAction::ForwardedPlayerEvent(event)).unwrap();
                callback();
            }
            thread::sleep(time::Duration::from_millis(50));
        });

        if let Some(player_ctx) = &self.player_context {
            let video_area = &player_ctx.video_area;
            if let Some(ui_ctx) = &self.ui_context {
                ui_ctx.main_box.pack_start(&*video_area, true, true, 0);
                ui_ctx.main_box.reorder_child(&*video_area, 0);
                video_area.show();

                let player_weak = player_ctx.player.downgrade();
                ui_ctx
                    .progress_bar
                    .connect_format_value(move |_, value| -> std::string::String {
                        let player = match player_weak.upgrade() {
                            Some(player) => player,
                            None => return std::string::String::from(""),
                        };
                        let position = gst::ClockTime::from_seconds(value as u64);
                        let duration = player.get_duration();
                        if duration.is_some() {
                            format!("{:.0} / {:.0}", position, duration)
                        } else {
                            format!("{:.0}", position)
                        }
                    });

                let player_weak = player_ctx.player.downgrade();
                let volume_scale = ui_ctx.volume_button.clone().upcast::<gtk::ScaleButton>();
                self.volume_signal_handler_id = Some(volume_scale.connect_value_changed(move |_, value| {
                    let player = match player_weak.upgrade() {
                        Some(player) => player,
                        None => return,
                    };
                    player.set_volume(value);
                }));

                let player_weak = player_ctx.player.downgrade();
                let range = ui_ctx.progress_bar.clone().upcast::<gtk::Range>();
                self.position_signal_handler_id = Some(range.connect_value_changed(move |range| {
                    let player = match player_weak.upgrade() {
                        Some(player) => player,
                        None => return,
                    };
                    let value = range.get_value();
                    player.seek(gst::ClockTime::from_seconds(value as u64));
                }));
            }

            self.pause_action.connect_change_state(|pause_action, _| {
                if let Some(is_paused) = pause_action.get_state() {
                    let paused = is_paused.get::<bool>().unwrap();

                    GLOBAL.with(|global| {
                        if let Some((ref video_player, ref _state)) = *global.borrow() {
                            if let Some(ref player) = video_player.player_context {
                                player.toggle_pause(paused);
                            }
                        }
                    });
                    pause_action.set_state(&(!paused).to_variant());
                }
            });
        }

        self.dump_pipeline_action.connect_activate(|_, _| {
            GLOBAL.with(|global| {
                if let Some((ref video_player, ref _state)) = *global.borrow() {
                    if let Some(ref player) = video_player.player_context {
                        player.dump_pipeline("glide");
                    }
                }
            });
        });

        self.seek_forward_action.connect_change_state(|_, _| {
            GLOBAL.with(|global| {
                if let Some((ref video_player, ref _state)) = *global.borrow() {
                    if let Some(ref player) = video_player.player_context {
                        player.seek(SeekDirection::Forward, SEEK_FORWARD_OFFSET);
                    }
                }
            });
        });

        self.seek_backward_action.connect_change_state(|_, _| {
            GLOBAL.with(|global| {
                if let Some((ref video_player, ref _state)) = *global.borrow() {
                    if let Some(ref player) = video_player.player_context {
                        player.seek(SeekDirection::Backward, SEEK_BACKWARD_OFFSET);
                    }
                }
            });
        });

        self.volume_decrease_action.connect_change_state(|_, _| {
            GLOBAL.with(|global| {
                if let Some((ref video_player, ref _state)) = *global.borrow() {
                    if let Some(ref player) = video_player.player_context {
                        player.decrease_volume();
                    }
                }
            });
        });

        self.volume_increase_action.connect_change_state(|_, _| {
            GLOBAL.with(|global| {
                if let Some((ref video_player, ref _state)) = *global.borrow() {
                    if let Some(ref player) = video_player.player_context {
                        player.increase_volume();
                    }
                }
            });
        });

        self.audio_mute_action.connect_change_state(|mute_action, _| {
            GLOBAL.with(|global| {
                if let Some((ref video_player, ref _state)) = *global.borrow() {
                    if let Some(ref player) = video_player.player_context {
                        if let Some(is_enabled) = mute_action.get_state() {
                            let enabled = is_enabled.get::<bool>().unwrap();
                            player.toggle_mute(!enabled);
                            mute_action.set_state(&(!enabled).to_variant());
                        }
                    }
                }
            });
        });

        self.fullscreen_action.connect_change_state(|fullscreen_action, _| {
            if let Some(is_fullscreen) = fullscreen_action.get_state() {
                let fullscreen = is_fullscreen.get::<bool>().unwrap();
                if !fullscreen {
                    GLOBAL.with(|global| {
                        if let Some((ref video_player, ref state)) = *global.borrow() {
                            if let Some(ref ui_ctx) = video_player.ui_context {
                                ui_ctx.enter_fullscreen(&state.app);
                                fullscreen_action.set_state(&true.to_variant());
                            }
                        }
                    });
                }
            }
        });

        self.restore_action.connect_change_state(|_, _| {
            GLOBAL.with(|global| {
                if let Some((ref video_player, ref state)) = *global.borrow() {
                    video_player.leave_fullscreen(&state.app);
                }
            });
        });

        self.subtitle_action.connect_change_state(|_, value| {
            GLOBAL.with(|global| {
                if let Some((ref video_player, ref _state)) = *global.borrow() {
                    video_player.update_subtitle_track(value);
                }
            });
        });

        self.audio_visualization_action.connect_change_state(|action, value| {
            if let Some(val) = value.clone() {
                if let Some(name) = val.get::<std::string::String>() {
                    GLOBAL.with(|global| {
                        if let Some((ref video_player, ref _state)) = *global.borrow() {
                            if let Some(ref ctx) = video_player.player_context {
                                if name == "none" {
                                    ctx.player.set_visualization_enabled(false);
                                } else {
                                    ctx.player.set_visualization(Some(name.as_str())).unwrap();
                                    ctx.player.set_visualization_enabled(true);
                                }
                                action.set_state(&val);
                            }
                        }
                    });
                }
            }
        });

        self.audio_track_action.connect_change_state(|action, value| {
            if let Some(val) = value.clone() {
                if let Some(idx) = val.get::<std::string::String>() {
                    let (_prefix, idx) = idx.split_at(6);
                    let idx = idx.parse::<i32>().unwrap();

                    GLOBAL.with(|global| {
                        if let Some((ref video_player, ref _state)) = *global.borrow() {
                            if let Some(ref ctx) = video_player.player_context {
                                ctx.player.set_audio_track_enabled(idx > -1);
                                if idx >= 0 {
                                    ctx.player.set_audio_track(idx).unwrap();
                                }
                                action.set_state(&val);
                            }
                        }
                    });
                }
            }
        });

        self.video_track_action.connect_change_state(|action, value| {
            if let Some(val) = value.clone() {
                if let Some(idx) = val.get::<std::string::String>() {
                    let (_prefix, idx) = idx.split_at(6);
                    let idx = idx.parse::<i32>().unwrap();

                    GLOBAL.with(|global| {
                        if let Some((ref video_player, ref _state)) = *global.borrow() {
                            if let Some(ref ctx) = video_player.player_context {
                                ctx.player.set_video_track_enabled(idx > -1);
                                if idx >= 0 {
                                    ctx.player.set_video_track(idx).unwrap();
                                }
                                action.set_state(&val);
                            }
                        }
                    });
                }
            }
        });

        self.open_media_action.connect_activate(|_, _| {
            GLOBAL.with(|global| {
                if let Some((ref video_player, ref _state)) = *global.borrow() {
                    if let Some(ref ui_ctx) = video_player.ui_context {
                        let dialog = gtk::FileChooserDialog::new(
                            Some("Choose a file"),
                            Some(&ui_ctx.window),
                            gtk::FileChooserAction::Open,
                        );
                        let ok = gtk::ResponseType::Ok.into();
                        dialog.add_buttons(&[("Open", ok), ("Cancel", gtk::ResponseType::Cancel.into())]);

                        dialog.set_select_multiple(true);
                        if let Some(ref player_ctx) = video_player.player_context {
                            if let Some(uri) = player_ctx.get_current_uri() {
                                set_dialog_folder_relative_to_uri(&dialog, &uri);
                            }
                        }

                        let response = dialog.run();
                        if response == ok {
                            if let Some(uri) = dialog.get_uri() {
                                println!("loading {}", &uri);
                                if let Some(ref player_ctx) = video_player.player_context {
                                    player_ctx.stop();
                                    player_ctx.load_uri(&uri);
                                }
                            }
                        }
                        dialog.destroy();
                    }
                }
            });
        });

        self.open_subtitle_file_action.connect_activate(|_, _| {
            GLOBAL.with(|global| {
                if let Some((ref video_player, ref _state)) = *global.borrow() {
                    if let Some(ref ui_ctx) = video_player.ui_context {
                        let dialog = gtk::FileChooserDialog::new(
                            Some("Choose a file"),
                            Some(&ui_ctx.window),
                            gtk::FileChooserAction::Open,
                        );
                        let ok = gtk::ResponseType::Ok.into();
                        dialog.add_buttons(&[("Open", ok), ("Cancel", gtk::ResponseType::Cancel.into())]);

                        if let Some(ref player_ctx) = video_player.player_context {
                            if let Some(uri) = player_ctx.get_current_uri() {
                                set_dialog_folder_relative_to_uri(&dialog, &uri);
                            }
                        }
                        let response = dialog.run();
                        if response == ok {
                            if let Some(uri) = dialog.get_uri() {
                                if let Some(ref player_ctx) = video_player.player_context {
                                    player_ctx.configure_subtitle_track(SubtitleTrack::External(uri));
                                }
                            }
                        }
                        dialog.destroy();
                        video_player.refresh_subtitle_track_menu();
                    }
                }
            });
        });

        if let Some(ref ui_ctx) = self.ui_context {
            ui_ctx.window.show_all();

            ui_ctx.window.connect_delete_event(|_, _| {
                GLOBAL.with(|global| {
                    if let Some((ref video_player, ref state)) = *global.borrow() {
                        video_player.quit(state);
                    }
                });
                Inhibit(false)
            });
        }

        match self.check_update() {
            Ok(o) => {
                match o {
                    self_update::Status::UpToDate(_version) => {}
                    _ => println!("Update succeeded: {}", o),
                };
            }
            Err(e) => eprintln!("Update failed: {}", e),
        };
    }

    pub fn dispatch_event(&self, event: &PlayerEvent, state: &AppState) {
        match event {
            PlayerEvent::MediaInfoUpdated => {
                self.media_info_updated();
            }
            PlayerEvent::PositionUpdated => {
                self.position_updated();
            }
            PlayerEvent::VideoDimensionsChanged(width, height) => {
                self.video_dimensions_changed(*width, *height);
            }
            PlayerEvent::StateChanged(ref s) => {
                self.playback_state_changed(s);
            }
            PlayerEvent::VolumeChanged(volume) => {
                self.volume_changed(*volume);
            }
            PlayerEvent::Error => {
                self.player_error(state);
            }
            _ => {}
        };
    }

    pub fn player_error(&self, state: &AppState) {
        // FIXME: display some GTK error dialog...
        eprintln!("Error!");
        self.quit(state);
    }

    pub fn volume_changed(&self, volume: f64) {
        if let Some(ref ui_context) = self.ui_context {
            let button = &ui_context.volume_button;
            let scale = button.clone().upcast::<gtk::ScaleButton>();
            if let Some(ref handler_id) = self.volume_signal_handler_id {
                glib::signal_handler_block(&scale, &handler_id);
                scale.set_value(volume);
                glib::signal_handler_unblock(&scale, &handler_id);
            }
        }
    }
    pub fn playback_state_changed(&self, playback_state: &PlaybackState) {
        if let Some(ref ui_context) = self.ui_context {
            let pause_button = &ui_context.pause_button;
            match playback_state {
                PlaybackState::Paused => {
                    let image = gtk::Image::new_from_icon_name(
                        "media-playback-start-symbolic",
                        gtk::IconSize::SmallToolbar.into(),
                    );
                    pause_button.set_image(&image);
                }
                PlaybackState::Playing => {
                    let image = gtk::Image::new_from_icon_name(
                        "media-playback-pause-symbolic",
                        gtk::IconSize::SmallToolbar.into(),
                    );
                    pause_button.set_image(&image);
                }
                _ => {}
            };
        }
    }

    pub fn video_dimensions_changed(&self, width: i32, height: i32) {
        let mut width = width;
        let mut height = height;
        if let Some(screen) = gdk::Screen::get_default() {
            width = cmp::min(width, screen.get_width());
            height = cmp::min(height, screen.get_height() - 100);
        }
        // FIXME: Somehow resize video_area to avoid black borders.
        if width > 0 && height > 0 {
            if let Some(ref ui_context) = self.ui_context {
                let window = &ui_context.window;
                window.resize(width, height);
            }
        }
    }

    pub fn media_info_updated(&self) {
        if let Some(ref player) = self.player_context {
            if let Some(info) = player.get_media_info() {
                if let Some(uri) = player.get_current_uri() {
                    if let Some(ref ui_context) = self.ui_context {
                        let window = &ui_context.window;
                        if let Some(title) = info.get_title() {
                            window.set_title(&*title);
                        } else if let Ok((filename, _)) = glib::filename_from_uri(&uri) {
                            window.set_title(&filename.as_os_str().to_string_lossy());
                        } else {
                            window.set_title(&uri);
                        }

                        let progress_bar = &ui_context.progress_bar;
                        let range = progress_bar.clone().upcast::<gtk::Range>();
                        let duration = info.get_duration();
                        if let Some(ref handler_id) = self.position_signal_handler_id {
                            glib::signal_handler_block(&range, &handler_id);
                            range.set_range(0.0, duration.seconds().unwrap() as f64);
                            glib::signal_handler_unblock(&range, &handler_id);
                        }

                        // Force the GtkScale to recompute its label widget size.
                        progress_bar.set_draw_value(false);
                        progress_bar.set_draw_value(true);
                    }

                    // Look for a matching subtitle file in same directory.
                    if let Ok((mut path, _)) = glib::filename_from_uri(&uri) {
                        path.set_extension("srt");
                        let subfile = path.as_path();
                        if subfile.is_file() {
                            if let Ok(suburi) = glib::filename_to_uri(subfile, None) {
                                player.configure_subtitle_track(SubtitleTrack::External(suburi));
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
                    self.audio_visualization_menu.remove_all();
                    self.audio_visualization_action.set_enabled(false);
                }
            }
        }
    }

    pub fn position_updated(&self) {
        if let Some(ref player) = self.player_context {
            let position = player.player.get_position();
            if let Some(ref ui_context) = self.ui_context {
                let progress_bar = &ui_context.progress_bar;
                let range = progress_bar.clone().upcast::<gtk::Range>();
                if let Some(ref handler_id) = self.position_signal_handler_id {
                    glib::signal_handler_block(&range, &handler_id);
                    range.set_value(position.seconds().unwrap() as f64);
                    glib::signal_handler_unblock(&range, &handler_id);
                }
            }
        }
    }

    pub fn update_subtitle_track(&self, value: &Option<glib::Variant>) {
        if let Some(val) = value {
            if let Some(val) = val.get::<std::string::String>() {
                let track = if val == "none" {
                    SubtitleTrack::None
                } else {
                    let (prefix, asset) = val.split_at(4);
                    if prefix == "ext-" {
                        SubtitleTrack::External(asset.to_string())
                    } else {
                        let idx = asset.parse::<i32>().unwrap();
                        SubtitleTrack::Inband(idx)
                    }
                };
                if let Some(ref ctx) = self.player_context {
                    ctx.configure_subtitle_track(track);
                }
            }
            self.subtitle_action.set_state(&val);
        }
    }

    pub fn refresh_subtitle_track_menu(&self) {
        let section = gio::Menu::new();

        if let Some(ref player) = self.player_context {
            if let Some(info) = player.player.get_media_info() {
                let mut i = 0;
                let item = gio::MenuItem::new(&*"Disable", &*"none");
                item.set_detailed_action("app.subtitle::none");
                section.append_item(&item);

                for sub_stream in info.get_subtitle_streams() {
                    let title = match sub_stream.get_tags() {
                        Some(tags) => match tags.get::<gst::tags::Title>() {
                            Some(val) => Some(std::string::String::from(val.get().unwrap())),
                            None => sub_stream.get_language(),
                        },
                        None => sub_stream.get_language(),
                    };

                    if let Some(title) = title {
                        let action_id = format!("app.subtitle::sub-{}", i);
                        let item = gio::MenuItem::new(&*title, &*action_id);
                        item.set_detailed_action(&*action_id);
                        section.append_item(&item);
                        i += 1;
                    }
                }
            }
        }

        let mut selected_action: Option<std::string::String> = None;
        if let Some(ref ctx) = self.player_context {
            if let Some(uri) = ctx.player.get_subtitle_uri() {
                if let Ok((path, _)) = glib::filename_from_uri(&uri) {
                    let subfile = path.as_path();
                    if let Some(filename) = subfile.file_name() {
                        if let Some(f) = filename.to_str() {
                            let v = format!("ext-{}", uri);
                            let action_id = format!("app.subtitle::{}", v);
                            let item = gio::MenuItem::new(f, &*action_id);
                            item.set_detailed_action(&*action_id);
                            section.append_item(&item);
                            selected_action = Some(v);
                        }
                    }
                }
            }
        }

        // TODO: Would be nice to keep previous external subs in the menu.
        self.subtitle_track_menu.remove_all();
        self.subtitle_track_menu.append_section(None, &section);

        let v = match selected_action {
            Some(a) => a.to_variant(),
            None => ("none").to_variant(),
        };
        self.subtitle_action.change_state(&v);
    }

    pub fn fill_audio_visualization_menu(&self) {
        if !self.audio_visualization_menu.is_mutable() {
            return;
        }
        let section = gio::Menu::new();

        let item = gio::MenuItem::new(&*"Disable", &*"none");
        item.set_detailed_action("app.audio-visualization::none");
        section.append_item(&item);

        for vis in gst_player::Player::visualizations_get() {
            let action_id = format!("app.audio-visualization::{}", vis.name());
            let item = gio::MenuItem::new(vis.description(), &*action_id);
            item.set_detailed_action(&*action_id);
            section.append_item(&item);
        }

        self.audio_visualization_menu.append_section(None, &section);
        self.audio_visualization_menu.freeze();
    }

    pub fn fill_audio_track_menu(&self, info: &gst_player::PlayerMediaInfo) {
        let mut i = 0;
        let section = gio::Menu::new();

        let item = gio::MenuItem::new(&*"Disable", &*"subtitle");
        item.set_detailed_action("app.audio-track::audio--1");
        section.append_item(&item);

        for audio_stream in info.get_audio_streams() {
            if let Some(lang) = audio_stream.get_language() {
                let action_id = format!("app.audio-track::audio-{}", i);
                let lang = format!("{} {} channels", lang, audio_stream.get_channels());
                let item = gio::MenuItem::new(&*lang, &*action_id);
                item.set_detailed_action(&*action_id);
                section.append_item(&item);
                i += 1;
            }
        }
        self.audio_track_menu.remove_all();
        self.audio_track_menu.append_section(None, &section);
    }

    pub fn fill_video_track_menu(&self, info: &gst_player::PlayerMediaInfo) {
        let mut i = 0;
        let section = gio::Menu::new();

        let item = gio::MenuItem::new(&*"Disable", &*"subtitle");
        item.set_detailed_action("app.video-track::video--1");
        section.append_item(&item);

        #[cfg_attr(feature = "cargo-clippy", allow(explicit_counter_loop))]
        for video_stream in info.get_video_streams() {
            let action_id = format!("app.video-track::video-{}", i);
            let description = format!("{}x{}", video_stream.get_width(), video_stream.get_height());
            let item = gio::MenuItem::new(&*description, &*action_id);
            item.set_detailed_action(&*action_id);
            section.append_item(&item);
            i += 1;
        }
        self.video_track_menu.remove_all();
        self.video_track_menu.append_section(None, &section);
    }

    pub fn open_files(&self, files: &[gio::File]) {
        let mut playlist = vec![];
        for file in files.to_vec() {
            if let Some(uri) = file.get_uri() {
                playlist.push(std::string::String::from(uri.as_str()));
            }
        }

        assert!(!files.is_empty());
        if let Some(ref player_ctx) = self.player_context {
            player_ctx.load_playlist(playlist);
        }
    }

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

    pub fn leave_fullscreen(&self, app: &gtk::Application) {
        let fullscreen_action = &self.fullscreen_action;
        if let Some(is_fullscreen) = fullscreen_action.get_state() {
            let fullscreen = is_fullscreen.get::<bool>().unwrap();

            if fullscreen {
                if let Some(ref ui_ctx) = self.ui_context {
                    ui_ctx.leave_fullscreen(app);
                    fullscreen_action.set_state(&false.to_variant());
                }
            }
        }
    }
}

fn main() {
    #[cfg(not(unix))]
    {
        println!("Add support for target platform");
        std::process::exit(-1);
    }

    #[cfg(target_os = "linux")]
    {
        // FIXME: We should somehow detect at runtime if we're running under a
        // Wayland compositor and thus don't call this.
        extern "C" {
            pub fn XInitThreads() -> c_void;
        }

        unsafe {
            XInitThreads();
        }
    }

    gst::init().expect("Failed to initialize GStreamer.");
    gtk::init().expect("Failed to initialize GTK.");

    let gtk_app = gtk::Application::new("net.baseart.Glide", gio::ApplicationFlags::HANDLES_OPEN)
        .expect("Application initialization failed");

    if let Some(settings) = gtk::Settings::get_default() {
        settings
            .set_property("gtk-application-prefer-dark-theme", &true)
            .unwrap();
    }

    glib::set_application_name("Glide");

    let (sender, receiver) = mpsc::channel();

    let gtk_app_clone = gtk_app.clone();

    let state = AppState {
        app: gtk_app,
        sender,
        receiver,
    };
    let app = VideoPlayer::new(&state);

    GLOBAL.with(move |global| {
        *global.borrow_mut() = Some((app, state));
    });

    let args = env::args().collect::<Vec<_>>();
    gtk_app_clone.run(&args);

    // unsafe {
    //     gst::deinit();
    // }
}
