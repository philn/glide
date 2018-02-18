extern crate cairo;
#[macro_use]
extern crate closet;
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
extern crate send_cell;
#[macro_use]
extern crate serde_derive;

use cairo::Context as CairoContext;
#[allow(unused_imports)]
use gdk::prelude::*;
use gio::prelude::*;
use gio::MenuExt;
use gio::MenuItemExt;
use gst::prelude::*;
use gtk::prelude::*;
use send_cell::SendCell;
use std::cell::RefCell;
use std::cmp;
use std::env;
#[allow(unused_imports)]
use std::os::raw::c_void;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;

mod player_context;
use player_context::PlayerContext;

mod ui_context;
use ui_context::UIContext;

mod common;
use common::{SeekDirection, INITIAL_POSITION, INITIAL_SIZE};

#[cfg(target_os = "macos")]
mod iokit_sleep_disabler;

#[derive(Clone)]
struct VideoPlayerInner {
    player_context: Option<PlayerContext>,
    ui_context: Option<UIContext>,
    fullscreen_action: gio::SimpleAction,
    restore_action: gio::SimpleAction,
    pause_action: gio::SimpleAction,
    seek_forward_action: gio::SimpleAction,
    seek_backward_action: gio::SimpleAction,
    subtitle_action: gio::SimpleAction,
    audio_track_action: gio::SimpleAction,
    open_subtitle_file_action: gio::SimpleAction,
    audio_mute_action: gio::SimpleAction,
    volume_increase_action: gio::SimpleAction,
    volume_decrease_action: gio::SimpleAction,
    subtitle_track_menu: gio::Menu,
    audio_track_menu: gio::Menu,
}

struct VideoPlayer {
    inner: Arc<Mutex<VideoPlayerInner>>,
}

static SEEK_BACKWARD_OFFSET: u64 = 2000;
static SEEK_FORWARD_OFFSET: u64 = 5000;

impl VideoPlayer {
    pub fn new(gtk_app: &gtk::Application) -> Self {
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

        let subtitle_track_menu = gio::Menu::new();
        let subtitle_action = gio::SimpleAction::new_stateful(
            "subtitle",
            glib::VariantTy::new("s").unwrap(),
            &"".to_variant(),
        );
        gtk_app.add_action(&subtitle_action);

        let audio_track_menu = gio::Menu::new();
        let audio_track_action = gio::SimpleAction::new_stateful(
            "audio-track",
            glib::VariantTy::new("s").unwrap(),
            &"audio-0".to_variant(),
        );
        gtk_app.add_action(&audio_track_action);

        let video_player = VideoPlayerInner {
            player_context: None,
            ui_context: None,
            fullscreen_action,
            restore_action,
            pause_action,
            seek_forward_action,
            seek_backward_action,
            subtitle_action,
            audio_track_action,
            open_subtitle_file_action,
            audio_mute_action,
            volume_increase_action,
            volume_decrease_action,
            subtitle_track_menu,
            audio_track_menu,
        };
        let inner = Arc::new(Mutex::new(video_player));

        let about = gio::SimpleAction::new("about", None);
        about.connect_activate(clone_army!([inner] move |_, _| {
            let dialog = gtk::AboutDialog::new();
            dialog.set_authors(&["Philippe Normand"]);
            dialog.set_website_label(Some("base-art.net"));
            dialog.set_website(Some("http://base-art.net"));
            dialog.set_title("About");
            if let Ok(ref inner) = inner.lock() {
                if let Some(ref ui_ctx) = inner.ui_context {
                    dialog.set_transient_for(Some(&ui_ctx.window));
                }
            }
            dialog.run();
            dialog.destroy();
        }));
        gtk_app.add_action(&about);

        gtk_app.connect_startup(clone_army!([inner] move |app| {

            let quit = gio::SimpleAction::new("quit", None);
            quit.connect_activate(clone_army!([app, inner] move |_, _| {
                if let Ok(inner) = inner.lock() {
                    if let Some(ref player_ctx) = inner.player_context {
                        player_ctx.write_last_known_media_position();
                    }
                }
                app.quit();
            }));
            app.add_action(&quit);

            app.set_accels_for_action("app.quit", &*vec!["<Meta>q", "<Ctrl>q"]);
            app.set_accels_for_action("app.fullscreen", &*vec!["<Meta>f", "<Alt>f"]);
            app.set_accels_for_action("app.restore", &*vec!["Escape"]);
            app.set_accels_for_action("app.pause", &*vec!["space"]);
            app.set_accels_for_action("app.seek-forward", &*vec!["<Meta>Right", "<Alt>Right"]);
            app.set_accels_for_action("app.seek-backward", &*vec!["<Meta>Left", "<Alt>Left"]);
            app.set_accels_for_action("app.open-subtitle-file", &*vec!["<Meta>s", "<Alt>s"]);
            app.set_accels_for_action("app.audio-volume-increase", &*vec!["<Meta>Up", "<Alt>Up"]);
            app.set_accels_for_action("app.audio-volume-decrease", &*vec!["<Meta>Down", "<Alt>Down"]);
            app.set_accels_for_action("app.audio-mute", &*vec!["<Meta>m", "<Alt>m"]);

            let menu = gio::Menu::new();
            let audio_menu = gio::Menu::new();
            let subtitles_menu = gio::Menu::new();

            if cfg!(not(target_os = "linux")) {
                menu.append("Quit", "app.quit");
                menu.append("About", "app.about");
            }

            if let Ok(mut inner) = inner.lock() {
                subtitles_menu.append("Add subtitle file...", "app.open-subtitle-file");
                subtitles_menu.append_submenu("Subtitle track", &inner.subtitle_track_menu);
                audio_menu.append("Increase Volume", "app.audio-volume-increase");
                audio_menu.append("Decrease Volume", "app.audio-volume-decrease");
                audio_menu.append("Mute", "app.audio-mute");
                audio_menu.append_submenu("Audio track", &inner.audio_track_menu);
                inner.ui_context = Some(UIContext::new(app));
            }

            menu.append_submenu("Audio", &audio_menu);
            menu.append_submenu("Subtitles", &subtitles_menu);

            if cfg!(target_os = "linux") {
                let app_menu = gio::Menu::new();
                // Only static menus here.
                app_menu.append("Quit", "app.quit");
                app_menu.append("About", "app.about");
                app.set_app_menu(&app_menu);
            }
            app.set_menubar(&menu);
        }));

        gtk_app.connect_open(clone_army!([inner] move |app, files, _| {
                app.activate();
                if let Ok(mut inner) = inner.lock() {
                    inner.open_files(files);
                }
            }));

        gtk_app.connect_shutdown(clone_army!([inner] move |_| {
                if let Ok(inner) = inner.lock() {
                    inner.stop_player();
                }
            }));

        if let Ok(inner) = inner.lock() {
            if let Some(ref ui_ctx) = inner.ui_context {
                ui_ctx
                    .window
                    .connect_delete_event(clone_army!([inner, gtk_app] move |_, _| {
                        inner.leave_fullscreen(&gtk_app);
                        gtk_app.quit();
                        Inhibit(false)
                    }));

                ui_ctx.window.connect_map_event(move |widget, _| {
                    if let Ok(size) = INITIAL_SIZE.lock() {
                        if let Some((width, height)) = *size {
                            widget.resize(width, height);
                        }
                    }
                    if let Ok(position) = INITIAL_POSITION.lock() {
                        if let Some((x, y)) = *position {
                            widget.move_(x, y);
                        }
                    }
                    Inhibit(false)
                });
            }
        }

        VideoPlayer { inner }
    }

    pub fn start(&self, app: &gtk::Application) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.player_context = Some(PlayerContext::new());

            inner.setup(app);

            if let Some(ref ctx) = inner.player_context {
                let video_area = &ctx.video_area;

                if let Some(ref ui_ctx) = inner.ui_context {
                    ui_ctx.main_box.pack_start(&*video_area, true, true, 0);
                    ui_ctx.main_box.reorder_child(&*video_area, 0);
                    video_area.show();

                    ui_ctx
                        .progress_bar
                        .connect_format_value(clone_army!([ctx] move |_, _| -> std::string::String {
                            let position = ctx.player.get_position();
                            let duration = ctx.player.get_duration();
                            format!("{:.0} / {:.0}", position, duration)
                        }));
                }

                let inner_clone = SendCell::new(inner.clone());
                ctx.player
                    .connect_video_dimensions_changed(clone_army!([inner_clone] move |_, width, height| {
                    let inner = inner_clone.borrow();
                    let mut width = width;
                    let mut height = height;
                    if let Some(screen) = gdk::Screen::get_default() {
                        width = cmp::min(width, screen.get_width());
                        height = cmp::min(height, screen.get_height() - 100);
                    }
                    if let Some(ref ui_ctx) = inner.ui_context {
                        // FIXME: Somehow resize video_area to avoid black borders.
                        ui_ctx.window.resize(width, height);
                    }
                }));

                video_area.connect_realize(clone_army!([inner] move |_| {
                        inner.prepare_video_overlay();
                    }));

                video_area.connect_draw(clone_army!([inner] move |_, cairo_context| {
                        inner.draw_video_area(cairo_context);
                        Inhibit(false)
                    }));

                video_area.connect_configure_event(clone_army!([inner] move |_, event| -> bool {
                        inner.resize_video_area(event);
                        true
                    }));
            }

            inner
                .pause_action
                .connect_change_state(clone_army!([inner] move |_, _| {
                inner.toggle_pause();
            }));

            inner
                .seek_forward_action
                .connect_change_state(clone_army!([inner] move |_, _| {
                    inner.seek(&SeekDirection::Forward, SEEK_FORWARD_OFFSET);
                }));

            inner
                .seek_backward_action
                .connect_change_state(clone_army!([inner] move |_, _| {
                    inner.seek(&SeekDirection::Backward, SEEK_BACKWARD_OFFSET);
                }));

            inner
                .fullscreen_action
                .connect_change_state(clone_army!([inner, app] move |_, _| {
                    inner.enter_fullscreen(&app);
                }));

            inner
                .restore_action
                .connect_change_state(clone_army!([inner, app] move |_, _| {
                    inner.leave_fullscreen(&app);
                }));

            inner
                .volume_decrease_action
                .connect_change_state(clone_army!([inner] move |_, _| {
                    inner.decrease_volume();
                }));

            inner
                .volume_increase_action
                .connect_change_state(clone_army!([inner] move |_, _| {
                    inner.increase_volume();
                }));

            inner
                .audio_mute_action
                .connect_change_state(clone_army!([inner] move |_, _| {
                inner.toggle_mute();
            }));

            inner
                .subtitle_action
                .connect_change_state(clone_army!([inner] move |action, value| {
                    if let Some(val) = value.clone() {
                        if let Some(idx) = val.get::<std::string::String>() {
                            let (_prefix, idx) = idx.split_at(4);
                            let idx = idx.parse::<i32>().unwrap();
                            if let Some(ref ctx) = inner.player_context {
                                ctx.player.set_subtitle_track_enabled(idx > -1);
                                if idx >= 0 {
                                    ctx.player.set_subtitle_track(idx).unwrap();
                                }
                                action.set_state(&val);
                            }
                        }
                    }
                }));

            inner
                .audio_track_action
                .connect_change_state(clone_army!([inner] move |action, value| {
                    if let Some(val) = value.clone() {
                        if let Some(idx) = val.get::<std::string::String>() {
                            let (_prefix, idx) = idx.split_at(6);
                            let idx = idx.parse::<i32>().unwrap();
                            if let Some(ref ctx) = inner.player_context {
                                ctx.player.set_audio_track_enabled(idx > -1);
                                if idx >= 0 {
                                    ctx.player.set_audio_track(idx).unwrap();
                                }
                                action.set_state(&val);
                            }
                        }
                    }
                }));

            inner
                .open_subtitle_file_action
                .connect_activate(clone_army!([inner] move |_, _| {
                        if let Some(ref ui_ctx) = inner.ui_context {
                            let dialog = gtk::FileChooserDialog::new(Some("Choose a file"), Some(&ui_ctx.window),
                                                                     gtk::FileChooserAction::Open);
                            dialog.add_buttons(&[
                                ("Open", gtk::ResponseType::Ok.into()),
                                ("Cancel", gtk::ResponseType::Cancel.into())
                            ]);

                            dialog.set_select_multiple(true);
                            dialog.run();

                            if let Some(uri) = dialog.get_uri() {
                                if let Some(ref player_ctx) = inner.player_context {
                                    player_ctx.player.set_subtitle_uri(&uri);
                                    player_ctx.player.set_subtitle_track_enabled(true);
                                }
                            }
                            dialog.destroy();
                        }
                }));

            inner.start();

            match inner.check_update() {
                Ok(o) => {
                    match o {
                        self_update::Status::UpToDate(_version) => {}
                        _ => println!("Update succeeded: {}", o),
                    };
                }
                Err(e) => eprintln!("Update failed: {}", e),
            };
        }
    }
}

impl VideoPlayerInner {
    pub fn setup(&self, gtk_app: &gtk::Application) {
        if let Some(ref ctx) = self.player_context {
            let file_list = Arc::new(Mutex::new(vec![]));
            let inner = SendCell::new(self.clone());
            if let Some(ref ui_ctx) = self.ui_context {
                let window_clone = SendCell::new(ui_ctx.window.clone());
                ctx.player
                    .connect_media_info_updated(clone_army!([file_list, inner] move |_, info| {
                        let uri = info.get_uri();
                        let mut file_list = file_list.lock().unwrap();
                        // Call this only once per asset.
                        if !&file_list.contains(&uri) {
                            file_list.push(uri.clone());
                            let window = window_clone.borrow();
                            if let Some(title) = info.get_title() {
                                window.set_title(&*title);
                            } else {
                                window.set_title(&*info.get_uri());
                            }

                            let inner = inner.borrow();
                            inner.fill_subtitle_track_menu(info);
                            inner.fill_audio_track_menu(info);
                        }
                    }));

                let pause_button_clone = SendCell::new(ui_ctx.pause_button.clone());
                ctx.player.connect_state_changed(move |_, state| {
                    let pause_button = pause_button_clone.borrow();
                    match state {
                        gst_player::PlayerState::Paused => {
                            let image = gtk::Image::new_from_icon_name(
                                "media-playback-start-symbolic",
                                gtk::IconSize::SmallToolbar.into(),
                            );
                            pause_button.set_image(&image);
                        }
                        gst_player::PlayerState::Playing => {
                            let image = gtk::Image::new_from_icon_name(
                                "media-playback-pause-symbolic",
                                gtk::IconSize::SmallToolbar.into(),
                            );
                            pause_button.set_image(&image);
                        }
                        _ => {}
                    };
                });

                let volume_scale = ui_ctx.volume_button.clone().upcast::<gtk::ScaleButton>();
                let player = &ctx.player;
                let volume_signal_handler_id =
                    volume_scale.connect_value_changed(clone_army!([player] move |_, value| {
                    player.set_volume(value);
                }));

                let volume_button_clone = SendCell::new(ui_ctx.volume_button.clone());
                let v_signal_handler_id = Arc::new(Mutex::new(volume_signal_handler_id));
                ctx.player
                    .connect_volume_changed(clone_army!([v_signal_handler_id] move |player| {
                    let button = volume_button_clone.borrow();
                    let scale = button.clone().upcast::<gtk::ScaleButton>();
                    let volume_signal_handler_id = v_signal_handler_id.lock().unwrap();
                    glib::signal_handler_block(&scale, &volume_signal_handler_id);
                    scale.set_value(player.get_volume());
                    glib::signal_handler_unblock(&scale, &volume_signal_handler_id);
                }));

                let range = ui_ctx.progress_bar.clone().upcast::<gtk::Range>();
                let player = &ctx.player;
                let seek_signal_handler_id = range.connect_value_changed(clone_army!([player] move |range| {
                    let value = range.get_value();
                    player.seek(gst::ClockTime::from_seconds(value as u64));
                }));

                let progress_bar_clone = SendCell::new(ui_ctx.progress_bar.clone());
                let signal_handler_id = Arc::new(Mutex::new(seek_signal_handler_id));
                ctx.player
                    .connect_duration_changed(clone_army!([signal_handler_id] move |_, duration| {
                        let progress_bar = progress_bar_clone.borrow();
                        let range = progress_bar.clone().upcast::<gtk::Range>();
                        let seek_signal_handler_id = signal_handler_id.lock().unwrap();
                        glib::signal_handler_block(&range, &seek_signal_handler_id);
                        range.set_range(0.0, duration.seconds().unwrap() as f64);
                        glib::signal_handler_unblock(&range, &seek_signal_handler_id);
                    }));

                let progress_bar_clone = SendCell::new(ui_ctx.progress_bar.clone());
                ctx.player
                    .connect_position_updated(clone_army!([signal_handler_id] move |_, position| {
                        let progress_bar = progress_bar_clone.borrow();
                        let range = progress_bar.clone().upcast::<gtk::Range>();
                        let seek_signal_handler_id = signal_handler_id.lock().unwrap();
                        glib::signal_handler_block(&range, &seek_signal_handler_id);
                        range.set_value(position.seconds().unwrap() as f64);
                        glib::signal_handler_unblock(&range, &seek_signal_handler_id);
                    }));
            }

            let app_clone = SendCell::new(gtk_app.clone());
            ctx.player.connect_error(move |_, error| {
                // FIXME: display some GTK error dialog...
                eprintln!("Error! {}", error);
                let app = &app_clone.borrow();
                app.quit();
            });
        }
    }

    pub fn start(&mut self) {
        if let Some(ref ui_ctx) = self.ui_context {
            ui_ctx.window.show_all();
        }
    }

    pub fn stop_player(&self) {
        if let Some(ref ctx) = self.player_context {
            ctx.player.stop();
        }
    }

    pub fn seek(&self, direction: &SeekDirection, offset: u64) {
        if let Some(ref ctx) = self.player_context {
            ctx.seek(direction, offset);
        }
    }

    pub fn increase_volume(&self) {
        if let Some(ref ctx) = self.player_context {
            ctx.increase_volume();
        }
    }

    pub fn decrease_volume(&self) {
        if let Some(ref ctx) = self.player_context {
            ctx.decrease_volume();
        }
    }

    pub fn toggle_mute(&self) {
        if let Some(ref ctx) = self.player_context {
            let mute_action = &self.audio_mute_action;
            if let Some(is_enabled) = mute_action.get_state() {
                let enabled = is_enabled.get::<bool>().unwrap();
                ctx.toggle_mute(!enabled);
                mute_action.set_state(&(!enabled).to_variant());
            }
        }
    }

    pub fn toggle_pause(&self) {
        if let Some(ref ctx) = self.player_context {
            let pause_action = &self.pause_action;
            if let Some(is_paused) = pause_action.get_state() {
                let paused = is_paused.get::<bool>().unwrap();
                ctx.toggle_pause(paused);
                pause_action.set_state(&(!paused).to_variant());
            }
        }
    }

    pub fn enter_fullscreen(&self, app: &gtk::Application) {
        let fullscreen_action = &self.fullscreen_action;
        if let Some(is_fullscreen) = fullscreen_action.get_state() {
            let fullscreen = is_fullscreen.get::<bool>().unwrap();
            if !fullscreen {
                if let Some(ref ui_ctx) = self.ui_context {
                    ui_ctx.enter_fullscreen(app);

                    #[cfg(target_os = "linux")]
                    ui_ctx.start_autohide_toolbar(&self.fullscreen_action);

                    fullscreen_action.set_state(&true.to_variant());
                }
            }
        }
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

    pub fn prepare_video_overlay(&self) {
        if let Some(ref ctx) = self.player_context {
            ctx.prepare_video_overlay();
        }
    }

    fn draw_video_area(&self, cairo_context: &CairoContext) {
        if let Some(ref ctx) = self.player_context {
            ctx.draw_video_overlay(cairo_context);
        }
    }

    fn resize_video_area(&self, event: &gdk::EventConfigure) {
        if let Some(ref ctx) = self.player_context {
            let (width, height) = event.get_size();
            let (x, y) = event.get_position();
            let rect = gst_video::VideoRectangle::new(x, y, width as i32, height as i32);
            ctx.resize_video_area(&rect);
        }
    }

    pub fn fill_subtitle_track_menu(&self, info: &gst_player::PlayerMediaInfo) {
        let mut i = 0;
        let section = gio::Menu::new();

        let item = gio::MenuItem::new(&*"Disable", &*"subtitle");
        item.set_detailed_action("app.subtitle::sub--1");
        section.append_item(&item);

        for sub_stream in info.get_subtitle_streams() {
            if let Some(lang) = sub_stream.get_language() {
                let action_id = format!("app.subtitle::sub-{}", i);
                let item = gio::MenuItem::new(&*lang, &*action_id);
                item.set_detailed_action(&*action_id);
                section.append_item(&item);
                i += 1;
            }
        }
        self.subtitle_track_menu.append_section(None, &section);
        self.subtitle_action.change_state(&("sub--1").to_variant());
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
        self.audio_track_menu.append_section(None, &section);
    }

    pub fn play_uri(&self, uri: &str) {
        if let Some(ref ctx) = self.player_context {
            ctx.play_uri(uri);
        }
    }

    pub fn open_files(&mut self, files: &[gio::File]) {
        let mut playlist = vec![];
        for file in files.to_vec() {
            if let Some(uri) = file.get_uri() {
                playlist.push(std::string::String::from(uri.as_str()));
            }
        }

        assert!(!files.is_empty());
        self.play_uri(&*playlist[0]);

        let inner_clone = SendCell::new(self.clone());
        let index_cell = RefCell::new(AtomicUsize::new(0));
        if let Some(ref ctx) = self.player_context {
            let player = &ctx.player;
            player.connect_end_of_stream(move |_| {
                let mut cell = index_cell.borrow_mut();
                let index = cell.get_mut();
                *index += 1;
                if *index < playlist.len() {
                    let inner_clone = inner_clone.borrow();
                    inner_clone.play_uri(&*playlist[*index]);
                }
                // TODO: else quit?
            });
        }
    }

    pub fn check_update(&self) -> Result<self_update::Status, self_update::errors::Error> {
        let target = self_update::get_target()?;
        if let Ok(mut b) = self_update::backends::github::Update::configure() {
            return b.repo_owner("philn")
                .repo_name("glide")
                .bin_name("glide")
                .target(&target)
                .current_version(cargo_crate_version!())
                .build()?
                .update();
        }

        Ok(self_update::Status::UpToDate(std::string::String::from(
            "OK",
        )))
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

    let gtk_app = gtk::Application::new("net.base-art.glide", gio::ApplicationFlags::HANDLES_OPEN)
        .expect("Application initialization failed");

    if let Some(settings) = gtk::Settings::get_default() {
        settings
            .set_property("gtk-application-prefer-dark-theme", &true)
            .unwrap();
    }

    glib::set_application_name("Glide");
    let app = VideoPlayer::new(&gtk_app);
    gtk_app.connect_activate(move |gtk_app| {
        app.start(gtk_app);
    });

    let args = env::args().collect::<Vec<_>>();
    gtk_app.run(&args);
}
