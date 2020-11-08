extern crate failure;
extern crate gdk;
extern crate gio;
extern crate glib;
extern crate gstreamer as gst;
extern crate gtk;

use gdk::prelude::*;
#[allow(unused_imports)]
use gio::prelude::*;
#[allow(unused_imports)]
use glib::SendWeakRef;
use gtk::prelude::*;
use std::boxed::Box;
use std::cmp;
#[allow(unused_imports)]
use std::os::raw::c_void;
use std::string;
use std::sync::Mutex;

lazy_static! {
    pub static ref INHIBIT_COOKIE: Mutex<Option<u32>> = { Mutex::new(None) };
    pub static ref INITIAL_POSITION: Mutex<Option<(i32, i32)>> = { Mutex::new(None) };
    pub static ref INITIAL_SIZE: Mutex<Option<(i32, i32)>> = { Mutex::new(None) };
    pub static ref MOUSE_NOTIFY_SIGNAL_ID: Mutex<Option<glib::SignalHandlerId>> = { Mutex::new(None) };
    pub static ref AUTOHIDE_SOURCE: Mutex<Option<glib::SourceId>> = { Mutex::new(None) };
}

use crate::app;
use crate::channel_player::PlaybackState;
use crate::video_player;
use crate::video_player::GLOBAL;
use crate::video_renderer;
use crate::video_renderer_factory;
use crate::{with_mut_video_player, with_video_player};

#[cfg(target_os = "macos")]
use crate::iokit_sleep_disabler;

pub fn initialize_and_create_app() -> gtk::Application {
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

    gtk::init().expect("Failed to initialize GTK.");

    let gtk_app = gtk::Application::new(Some("net.baseart.Glide"), gio::ApplicationFlags::HANDLES_OPEN)
        .expect("Application initialization failed");

    if let Some(settings) = gtk::Settings::get_default() {
        settings
            .set_property("gtk-application-prefer-dark-theme", &true)
            .unwrap();
    }

    gtk_app
}

pub struct AppData {
    app: Box<gtk::Application>,
    window: gtk::ApplicationWindow,
    main_box: gtk::Box,
    pause_button: gtk::Button,
    progress_bar: gtk::Scale,
    volume_button: gtk::VolumeButton,
    toolbar_box: gtk::Box,
    subtitle_track_menu: gio::Menu,
    audio_track_menu: gio::Menu,
    video_track_menu: gio::Menu,
    audio_visualization_menu: gio::Menu,
    volume_signal_handler_id: glib::SignalHandlerId,
    position_signal_handler_id: glib::SignalHandlerId,
}

pub struct GlideGTKApp {
    data: Box<AppData>,
}

const MINIMAL_WINDOW_SIZE: (i32, i32) = (640, 480);
const VERSION: &str = env!("CARGO_PKG_VERSION");

impl AppData {
    pub fn new() -> Self {
        let gtk_app = initialize_and_create_app();

        let builder = gtk::Builder::new_from_string(include_str!("../data/net.baseart.Glide.ui"));

        let pause_button = {
            let button: gtk::Button = builder.get_object("pause-button").unwrap();
            button
                .clone()
                .upcast::<gtk::Actionable>()
                .set_action_name(Some("app.pause"));
            button
        };
        let image = gtk::Image::new_from_icon_name(Some("media-playback-start-symbolic"), gtk::IconSize::SmallToolbar);
        pause_button.set_image(Some(&image));

        let button: gtk::Button = builder.get_object("seek-backward-button").unwrap();
        button
            .upcast::<gtk::Actionable>()
            .set_action_name(Some("app.seek-backward"));

        let button: gtk::Button = builder.get_object("seek-forward-button").unwrap();
        button
            .upcast::<gtk::Actionable>()
            .set_action_name(Some("app.seek-forward"));

        let button: gtk::Button = builder.get_object("fullscreen-button").unwrap();
        button
            .upcast::<gtk::Actionable>()
            .set_action_name(Some("app.fullscreen"));

        let main_box: gtk::Box = builder.get_object("main-box").unwrap();
        let toolbar_box: gtk::Box = builder.get_object("toolbar-box").unwrap();
        let progress_bar: gtk::Scale = builder.get_object("progress-bar").unwrap();
        let volume_button: gtk::VolumeButton = builder.get_object("volume-button").unwrap();

        let window: gtk::ApplicationWindow = builder.get_object("application-window").unwrap();
        window.connect_map_event(move |widget, _| {
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

        let subtitle_track_menu: gio::Menu = builder.get_object("subtitle-track-menu").unwrap();
        let audio_track_menu: gio::Menu = builder.get_object("audio-track-menu").unwrap();
        let video_track_menu: gio::Menu = builder.get_object("video-track-menu").unwrap();
        let audio_visualization_menu: gio::Menu = builder.get_object("audio-visualization-menu").unwrap();

        let menu: gio::Menu = builder.get_object("main-menu").unwrap();

        #[cfg(not(target_os = "linux"))]
        {
            menu.append(Some("Quit"), Some("app.quit"));
            menu.append(Some("About"), Some("app.about"));
        }

        window.connect_delete_event(move |_, _| {
            with_video_player!(video_player {
                video_player.quit();
            });
            Inhibit(false)
        });

        progress_bar.connect_format_value(move |widget, value| -> string::String {
            let range = widget.clone().upcast::<gtk::Range>();
            let duration = range.get_adjustment().get_upper();
            let position = gst::ClockTime::from_seconds(value as u64);
            let duration = gst::ClockTime::from_seconds(duration as u64);
            if duration.is_some() {
                format!("{:.0} / {:.0}", position, duration)
            } else {
                format!("{:.0}", position)
            }
        });

        let volume_scale = volume_button.clone().upcast::<gtk::ScaleButton>();
        let volume_signal_handler_id = volume_scale.connect_value_changed(move |_, value| {
            with_video_player!(video_player {
                video_player.player.set_volume(value);
            });
        });

        let range = progress_bar.clone().upcast::<gtk::Range>();
        let position_signal_handler_id = range.connect_value_changed(move |range| {
            with_video_player!(video_player {
                video_player.player.seek_to(gst::ClockTime::from_seconds(range.get_value() as u64));
            });
        });

        let window_weak = SendWeakRef::from(window.downgrade());
        gtk_app.connect_startup(move |gtk_app| {
            let accels_per_action = [
                ("open-media", "<Primary>o"),
                ("quit", "<Primary>q"),
                ("fullscreen", "<Primary>f"),
                ("restore", "Escape"),
                ("pause", "space"),
                ("seek-forward", "<Primary>Right"),
                ("seek-backward", "<Primary>Left"),
                ("audio-volume-increase", "<Primary>Up"),
                ("audio-volume-decrease", "<Primary>Down"),
                ("audio-mute", "<Primary>m"),
                ("open-subtitle-file", "<Primary>s"),
                ("dump-pipeline", "<Ctrl>d"),
            ];
            for (action, accel) in accels_per_action.iter() {
                gtk_app.set_accels_for_action(&format!("app.{}", action), &[*accel]);
            }

            if let Some(window) = window_weak.upgrade() {
                window.set_application(Some(gtk_app));

                #[cfg(target_os = "linux")]
                {
                    let header_bar = gtk::HeaderBar::new();
                    header_bar.set_show_close_button(true);

                    let main_menu = gtk::MenuButton::new();
                    let main_menu_image =
                        gtk::Image::new_from_icon_name(Some("open-menu-symbolic"), gtk::IconSize::Menu);
                    main_menu.add(&main_menu_image);
                    main_menu.set_menu_model(Some(&data.menu));

                    header_bar.pack_end(&main_menu);
                    window.set_titlebar(Some(&header_bar));
                }
            }

            #[cfg(not(target_os = "linux"))]
            {
                gtk_app.set_menubar(Some(&menu));
            }
        });

        gtk_app.connect_activate(|_| {
            with_mut_video_player!(player {
                player.start();
            })
        });

        Self {
            app: Box::new(gtk_app),
            window,
            main_box,
            pause_button,
            progress_bar,
            volume_button,
            toolbar_box,
            subtitle_track_menu,
            audio_track_menu,
            video_track_menu,
            audio_visualization_menu,
            volume_signal_handler_id,
            position_signal_handler_id,
        }
    }
}

impl GlideGTKApp {
    pub fn new() -> Self {
        Self {
            data: Box::new(AppData::new()),
        }
    }

    #[cfg(target_os = "linux")]
    pub fn start_autohide_toolbar(&self) {
        let toolbar_weak = self.data.toolbar_box.downgrade();
        let notify_signal_id = self.data.window.connect_motion_notify_event(move |window, _| {
            if let Some(source) = AUTOHIDE_SOURCE.lock().unwrap().take() {
                glib::source_remove(source);
            }

            let gdk_window = window.get_window().unwrap();
            gdk_window.set_cursor(None);

            let toolbar = match toolbar_weak.upgrade() {
                Some(t) => t,
                None => return gtk::Inhibit(false),
            };
            toolbar.set_visible(true);

            let window_weak = SendWeakRef::from(window.downgrade());
            let toolbar_weak = SendWeakRef::from(toolbar.downgrade());
            *AUTOHIDE_SOURCE.lock().unwrap() = Some(glib::timeout_add_seconds(5, move || {
                if let Ok(cookie) = INHIBIT_COOKIE.lock() {
                    if cookie.is_some() {
                        if let Some(toolbar) = toolbar_weak.upgrade() {
                            toolbar.set_visible(false);
                        }
                        if let Some(window) = window_weak.upgrade() {
                            let gdk_window = window.get_window().unwrap();
                            let cursor =
                                gdk::Cursor::new_for_display(&gdk_window.get_display(), gdk::CursorType::BlankCursor);
                            gdk_window.set_cursor(Some(&cursor));
                        }
                    }
                }
                *AUTOHIDE_SOURCE.lock().unwrap() = None;
                glib::Continue(false)
            }));
            gtk::Inhibit(false)
        });
        *MOUSE_NOTIFY_SIGNAL_ID.lock().unwrap() = Some(notify_signal_id);
    }
}

impl app::Application for GlideGTKApp {
    fn enter_fullscreen(&self) {
        let window = &self.data.window;
        #[cfg(target_os = "macos")]
        {
            *INHIBIT_COOKIE.lock().unwrap() = Some(iokit_sleep_disabler::prevent_display_sleep("Glide full-screen"));
        }
        #[cfg(not(target_os = "macos"))]
        {
            let flags = gtk::ApplicationInhibitFlags::SUSPEND | gtk::ApplicationInhibitFlags::IDLE;
            *INHIBIT_COOKIE.lock().unwrap() =
                Some(self.data.app.inhibit(Some(window), flags, Some("Glide full-screen")));
        }
        *INITIAL_SIZE.lock().unwrap() = Some(window.get_size());
        *INITIAL_POSITION.lock().unwrap() = Some(window.get_position());
        window.set_show_menubar(false);
        self.data.toolbar_box.set_visible(false);
        window.fullscreen();
        let gdk_window = window.get_window().unwrap();
        let cursor = gdk::Cursor::new_for_display(&gdk_window.get_display(), gdk::CursorType::BlankCursor);
        gdk_window.set_cursor(Some(&cursor));

        #[cfg(target_os = "linux")]
        self.start_autohide_toolbar();
    }

    fn leave_fullscreen(&self) {
        let window = &self.data.window;
        let gdk_window = window.get_window().unwrap();
        if let Ok(mut cookie) = INHIBIT_COOKIE.lock() {
            #[cfg(target_os = "macos")]
            iokit_sleep_disabler::release_sleep_assertion(cookie.unwrap());
            #[cfg(not(target_os = "macos"))]
            self.data.app.uninhibit(cookie.unwrap());
            *cookie = None;
        }
        if let Ok(mut signal_handler_id) = MOUSE_NOTIFY_SIGNAL_ID.lock() {
            if let Some(handler) = signal_handler_id.take() {
                window.disconnect(handler);
            }
        }
        window.unfullscreen();
        self.data.toolbar_box.set_visible(true);
        window.set_show_menubar(true);
        gdk_window.set_cursor(None);
    }

    fn dialog_result(&self, relative_uri: Option<glib::GString>) -> Option<glib::GString> {
        let dialog = gtk::FileChooserDialog::with_buttons(
            Some("Choose a file"),
            Some(&self.data.window),
            gtk::FileChooserAction::Open,
            &[("Open", gtk::ResponseType::Ok), ("Cancel", gtk::ResponseType::Cancel)],
        );

        dialog.set_select_multiple(true);
        if let Some(uri) = relative_uri {
            if let Ok((filename, _)) = glib::filename_from_uri(&uri) {
                if let Some(folder) = filename.parent() {
                    dialog.set_current_folder(folder);
                }
            }
        }

        let result_uri = if dialog.run() == gtk::ResponseType::Ok {
            dialog.get_uri()
        } else {
            None
        };
        dialog.destroy();
        result_uri
    }

    fn implementation(&self) -> Option<app::ApplicationImpl> {
        Some(app::ApplicationImpl::GTK(*self.data.app.clone()))
    }

    fn glib_context(&self) -> Option<&glib::MainContext> {
        None
    }

    fn start(&self) {
        self.data.window.show_all();

        self.data.app.connect_open(|gtk_app, files, _| {
            gtk_app.activate();
            let mut file_list = vec![];
            for file in files.to_vec() {
                let uri = file.get_uri();
                file_list.push(std::string::String::from(uri.as_str()));
            }
            with_mut_video_player!(player {
                player.load_playlist(file_list);
            });
        });
    }

    fn stop(&self) {
        self.data.app.quit();
    }

    fn set_video_renderer(&self, renderer: &video_renderer::VideoRenderer) {
        if let Some(implementation) = renderer.implementation() {
            match implementation {
                video_renderer::VideoWidgetImpl::GTK(widget) => {
                    if let Some(video_area) = widget.upgrade() {
                        self.data.main_box.pack_start(&video_area, true, true, 0);
                        self.data.main_box.reorder_child(&video_area, 0);
                        video_area.show();
                    }
                }
            }
        }
    }

    fn volume_changed(&self, volume: f64) {
        let button = &self.data.volume_button;
        let scale = button.clone().upcast::<gtk::ScaleButton>();
        glib::signal_handler_block(&scale, &self.data.volume_signal_handler_id);
        scale.set_value(volume);
        glib::signal_handler_unblock(&scale, &self.data.volume_signal_handler_id);
    }

    fn set_position_range_value(&self, position: u64) {
        let range = self.data.progress_bar.clone().upcast::<gtk::Range>();
        glib::signal_handler_block(&range, &self.data.position_signal_handler_id);
        range.set_value(position as f64);
        glib::signal_handler_unblock(&range, &self.data.position_signal_handler_id);
    }

    fn resize_window(&self, width: i32, height: i32) {
        let mut width = width;
        let mut height = height;
        if let Some(screen) = gdk::Screen::get_default() {
            width = cmp::min(width, screen.get_width());
            height = cmp::min(height, screen.get_height() - 100);
        }
        // FIXME: Somehow resize video_area to avoid black borders.
        if width > MINIMAL_WINDOW_SIZE.0 && height > MINIMAL_WINDOW_SIZE.1 {
            self.data.window.resize(width, height);
        }
    }

    fn set_window_title(&self, title: &str) {
        self.data.window.set_title(title);
    }

    fn set_position_range_end(&self, end: f64) {
        let progress_bar = &self.data.progress_bar;
        let range = progress_bar.clone().upcast::<gtk::Range>();
        glib::signal_handler_block(&range, &self.data.position_signal_handler_id);
        range.set_range(0.0, end);
        glib::signal_handler_unblock(&range, &self.data.position_signal_handler_id);

        // Force the GtkScale to recompute its label widget size.
        progress_bar.set_draw_value(false);
        progress_bar.set_draw_value(true);
    }

    fn display_about_dialog(&self) {
        let dialog = gtk::AboutDialog::new();
        dialog.set_authors(&["Philippe Normand"]);
        dialog.set_website_label(Some("base-art.net"));
        dialog.set_website(Some("http://base-art.net"));
        dialog.set_title("About");
        dialog.set_version(Some(VERSION));
        let s = format!(
            "Multimedia playback support provided by {}.\nUser interface running on GTK {}.{}.{}",
            gst::version_string(),
            gtk::get_major_version(),
            gtk::get_minor_version(),
            gtk::get_micro_version()
        );
        dialog.set_comments(Some(s.as_str()));
        dialog.set_transient_for(Some(&self.data.window));
        dialog.run();
        dialog.destroy();
    }

    fn playback_state_changed(&self, playback_state: &PlaybackState) {
        match playback_state {
            PlaybackState::Paused => {
                let image =
                    gtk::Image::new_from_icon_name(Some("media-playback-start-symbolic"), gtk::IconSize::SmallToolbar);
                self.data.pause_button.set_image(Some(&image));
            }
            PlaybackState::Playing => {
                let image =
                    gtk::Image::new_from_icon_name(Some("media-playback-pause-symbolic"), gtk::IconSize::SmallToolbar);
                self.data.pause_button.set_image(Some(&image));
            }
            _ => {}
        };
    }

    fn refresh_video_renderer(&self) {}

    fn update_subtitle_track_menu(&self, section: &gio::Menu) {
        // TODO: Would be nice to keep previous external subs in the menu.
        self.data.subtitle_track_menu.remove_all();
        self.data.subtitle_track_menu.append_section(None, section);
    }

    fn update_audio_track_menu(&self, section: &gio::Menu) {
        self.data.audio_track_menu.remove_all();
        self.data.audio_track_menu.append_section(None, section);
    }

    fn update_video_track_menu(&self, section: &gio::Menu) {
        self.data.video_track_menu.remove_all();
        self.data.video_track_menu.append_section(None, section);
    }

    fn clear_audio_visualization_menu(&self) {
        self.data.audio_visualization_menu.remove_all();
    }

    fn update_audio_visualization_menu(&self, section: &gio::Menu) {
        self.data.audio_visualization_menu.append_section(None, section);
        self.data.audio_visualization_menu.freeze();
    }

    fn mutable_audio_visualization_menu(&self) -> bool {
        self.data.audio_visualization_menu.is_mutable()
    }

    fn add_action(&self, action: &gio::SimpleAction) {
        //*self.gtk_app.lock().unwrap().add_action(action);
        self.data.app.add_action(action);
    }
}
