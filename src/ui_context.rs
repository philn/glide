extern crate gdk;
extern crate gio;
extern crate glib;
extern crate gstreamer as gst;
extern crate gtk;

use gdk::prelude::*;
#[allow(unused_imports)]
use gio::prelude::*;
use gio::MenuExt;
#[allow(unused_imports)]
use glib::SendWeakRef;
use gtk::prelude::*;
use std::cmp;
#[allow(unused_imports)]
use std::os::raw::c_void;
use std::string;
use std::sync::Mutex;

use channel_player::PlaybackState;

lazy_static! {
    pub static ref INHIBIT_COOKIE: Mutex<Option<u32>> = { Mutex::new(None) };
    pub static ref INITIAL_POSITION: Mutex<Option<(i32, i32)>> = { Mutex::new(None) };
    pub static ref INITIAL_SIZE: Mutex<Option<(i32, i32)>> = { Mutex::new(None) };
    pub static ref MOUSE_NOTIFY_SIGNAL_ID: Mutex<Option<glib::SignalHandlerId>> = { Mutex::new(None) };
    pub static ref AUTOHIDE_SOURCE: Mutex<Option<glib::SourceId>> = { Mutex::new(None) };
}

#[cfg(target_os = "macos")]
use iokit_sleep_disabler;

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

    let gtk_app = gtk::Application::new("net.baseart.Glide", gio::ApplicationFlags::HANDLES_OPEN)
        .expect("Application initialization failed");

    if let Some(settings) = gtk::Settings::get_default() {
        settings
            .set_property("gtk-application-prefer-dark-theme", &true)
            .unwrap();
    }

    gtk_app
}

pub struct UIContext {
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
    volume_signal_handler_id: Option<glib::SignalHandlerId>,
    position_signal_handler_id: Option<glib::SignalHandlerId>,
    app: gtk::Application,
}

const MINIMAL_WINDOW_SIZE: (i32, i32) = (640, 480);
const VERSION: &str = env!("CARGO_PKG_VERSION");

impl UIContext {
    pub fn new(gtk_app: gtk::Application) -> Self {
        let builder = gtk::Builder::new_from_string(include_str!("../data/net.baseart.Glide.ui"));

        let pause_button = {
            let button: gtk::Button = builder.get_object("pause-button").unwrap();
            button.clone().upcast::<gtk::Actionable>().set_action_name("app.pause");
            button
        };

        let button: gtk::Button = builder.get_object("seek-backward-button").unwrap();
        button.upcast::<gtk::Actionable>().set_action_name("app.seek-backward");

        let button: gtk::Button = builder.get_object("seek-forward-button").unwrap();
        button.upcast::<gtk::Actionable>().set_action_name("app.seek-forward");

        let button: gtk::Button = builder.get_object("fullscreen-button").unwrap();
        button.upcast::<gtk::Actionable>().set_action_name("app.fullscreen");

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
            menu.append("Quit", "app.quit");
            menu.append("About", "app.about");
        }

        let window_weak = SendWeakRef::from(window.downgrade());
        gtk_app.connect_startup(move |app| {
            let accels_per_action = [
                ("<Primary>q", "quit"),
                ("<Primary>f", "fullscreen"),
                ("Escape", "restore"),
                ("space", "pause"),
                ("<Primary>Right", "seek-forward"),
                ("<Primary>Left", "seek-backward"),
                ("<Ctrl>d", "dump-pipeline"),
            ];
            for (accel, action) in accels_per_action.iter() {
                app.add_accelerator(accel, &format!("app.{}", action), None);
            }

            if let Some(window) = window_weak.upgrade() {
                window.set_application(app);

                #[cfg(target_os = "linux")]
                {
                    let header_bar = gtk::HeaderBar::new();
                    header_bar.set_show_close_button(true);

                    let main_menu = gtk::MenuButton::new();
                    let main_menu_image = gtk::Image::new_from_icon_name("open-menu-symbolic", 1);
                    main_menu.add(&main_menu_image);
                    main_menu.set_menu_model(&menu);

                    header_bar.pack_end(&main_menu);
                    window.set_titlebar(&header_bar);
                }
            }

            #[cfg(not(target_os = "linux"))]
            {
                app.set_menubar(&menu);
            }
        });

        Self {
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
            volume_signal_handler_id: None,
            position_signal_handler_id: None,
            app: gtk_app,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn start_autohide_toolbar(&self) {
        let toolbar_weak = self.toolbar_box.downgrade();
        let notify_signal_id = self.window.connect_motion_notify_event(move |window, _| {
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
                            let cursor = gdk::Cursor::new(gdk::CursorType::BlankCursor);
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

    pub fn enter_fullscreen(&self) {
        let window = &self.window;
        #[cfg(target_os = "macos")]
        {
            *INHIBIT_COOKIE.lock().unwrap() = Some(iokit_sleep_disabler::prevent_display_sleep("Glide full-screen"));
        }
        #[cfg(not(target_os = "macos"))]
        {
            let flags = gtk::ApplicationInhibitFlags::SUSPEND | gtk::ApplicationInhibitFlags::IDLE;
            *INHIBIT_COOKIE.lock().unwrap() = Some(self.app.inhibit(window, flags, Some("Glide full-screen")));
        }
        *INITIAL_SIZE.lock().unwrap() = Some(window.get_size());
        *INITIAL_POSITION.lock().unwrap() = Some(window.get_position());
        window.set_show_menubar(false);
        self.toolbar_box.set_visible(false);
        window.fullscreen();
        let cursor = gdk::Cursor::new(gdk::CursorType::BlankCursor);
        let gdk_window = window.get_window().unwrap();
        gdk_window.set_cursor(Some(&cursor));

        #[cfg(target_os = "linux")]
        self.start_autohide_toolbar();
    }

    pub fn leave_fullscreen(&self) {
        let window = &self.window;
        let gdk_window = window.get_window().unwrap();
        if let Ok(mut cookie) = INHIBIT_COOKIE.lock() {
            #[cfg(target_os = "macos")]
            iokit_sleep_disabler::release_sleep_assertion(cookie.unwrap());
            #[cfg(not(target_os = "macos"))]
            self.app.uninhibit(cookie.unwrap());
            *cookie = None;
        }
        if let Ok(mut signal_handler_id) = MOUSE_NOTIFY_SIGNAL_ID.lock() {
            if let Some(handler) = signal_handler_id.take() {
                window.disconnect(handler);
            }
        }
        window.unfullscreen();
        self.toolbar_box.set_visible(true);
        window.set_show_menubar(true);
        gdk_window.set_cursor(None);
    }

    pub fn dialog_result(&self, relative_uri: Option<string::String>) -> Option<string::String> {
        let dialog =
            gtk::FileChooserDialog::new(Some("Choose a file"), Some(&self.window), gtk::FileChooserAction::Open);
        let ok = gtk::ResponseType::Ok.into();
        dialog.add_buttons(&[("Open", ok), ("Cancel", gtk::ResponseType::Cancel.into())]);

        dialog.set_select_multiple(true);
        if let Some(uri) = relative_uri {
            if let Ok((filename, _)) = glib::filename_from_uri(&uri) {
                if let Some(folder) = filename.parent() {
                    dialog.set_current_folder(folder);
                }
            }
        }

        let mut result_uri: Option<string::String> = None;
        let response = dialog.run();
        if response == ok {
            if let Some(uri) = dialog.get_uri() {
                result_uri = Some(uri);
            }
        }
        dialog.destroy();
        result_uri
    }

    pub fn start<F: Fn() + Send + Sync + 'static>(&self, f: F) {
        self.window.show_all();

        self.window.connect_delete_event(move |_, _| {
            f();
            Inhibit(false)
        });
    }

    pub fn stop(&self) {
        self.app.quit();
    }

    pub fn set_progress_bar_format_callback<F>(&self, f: F)
    where
        F: Fn(f64, f64) -> string::String + Send + Sync + 'static,
    {
        self.progress_bar
            .connect_format_value(move |widget, value| -> string::String {
                let range = widget.clone().upcast::<gtk::Range>();
                f(value, range.get_adjustment().get_upper())
            });
    }

    pub fn set_volume_value_changed_callback<F: Fn(f64) + Send + Sync + 'static>(&mut self, f: F) {
        let volume_scale = self.volume_button.clone().upcast::<gtk::ScaleButton>();
        self.volume_signal_handler_id = Some(volume_scale.connect_value_changed(move |_, value| {
            f(value);
        }));
    }

    pub fn set_position_changed_callback<F: Fn(u64) + Send + Sync + 'static>(&mut self, f: F) {
        let range = self.progress_bar.clone().upcast::<gtk::Range>();
        self.position_signal_handler_id = Some(range.connect_value_changed(move |range| {
            f(range.get_value() as u64);
        }));
    }

    pub fn volume_changed(&self, volume: f64) {
        let button = &self.volume_button;
        let scale = button.clone().upcast::<gtk::ScaleButton>();
        if let Some(ref handler_id) = self.volume_signal_handler_id {
            glib::signal_handler_block(&scale, &handler_id);
            scale.set_value(volume);
            glib::signal_handler_unblock(&scale, &handler_id);
        }
    }

    pub fn set_position_range_value(&self, position: u64) {
        let range = self.progress_bar.clone().upcast::<gtk::Range>();
        if let Some(ref handler_id) = self.position_signal_handler_id {
            glib::signal_handler_block(&range, &handler_id);
            range.set_value(position as f64);
            glib::signal_handler_unblock(&range, &handler_id);
        }
    }

    pub fn set_video_area(&self, video_area: &gtk::Widget) {
        self.main_box.pack_start(&*video_area, true, true, 0);
        self.main_box.reorder_child(&*video_area, 0);
        video_area.show();
    }

    pub fn resize_window(&self, width: i32, height: i32) {
        let mut width = width;
        let mut height = height;
        if let Some(screen) = gdk::Screen::get_default() {
            width = cmp::min(width, screen.get_width());
            height = cmp::min(height, screen.get_height() - 100);
        }
        // FIXME: Somehow resize video_area to avoid black borders.
        if width > MINIMAL_WINDOW_SIZE.0 && height > MINIMAL_WINDOW_SIZE.1 {
            self.window.resize(width, height);
        }
    }

    pub fn set_window_title(&self, title: &str) {
        self.window.set_title(title);
    }

    pub fn set_position_range_end(&self, end: f64) {
        let progress_bar = &self.progress_bar;
        let range = progress_bar.clone().upcast::<gtk::Range>();
        if let Some(ref handler_id) = self.position_signal_handler_id {
            glib::signal_handler_block(&range, &handler_id);
            range.set_range(0.0, end);
            glib::signal_handler_unblock(&range, &handler_id);
        }

        // Force the GtkScale to recompute its label widget size.
        progress_bar.set_draw_value(false);
        progress_bar.set_draw_value(true);
    }

    pub fn display_about_dialog(&self) {
        let dialog = gtk::AboutDialog::new();
        dialog.set_authors(&["Philippe Normand"]);
        dialog.set_website_label(Some("base-art.net"));
        dialog.set_website(Some("http://base-art.net"));
        dialog.set_title("About");
        dialog.set_version(VERSION);
        let s = format!(
            "Multimedia playback support provided by {}.\nUser interface running on GTK {}.{}.{}",
            gst::version_string(),
            gtk::get_major_version(),
            gtk::get_minor_version(),
            gtk::get_micro_version()
        );
        dialog.set_comments(Some(s.as_str()));
        dialog.set_transient_for(Some(&self.window));
        dialog.run();
        dialog.destroy();
    }

    pub fn playback_state_changed(&self, playback_state: &PlaybackState) {
        match playback_state {
            PlaybackState::Paused => {
                let image =
                    gtk::Image::new_from_icon_name("media-playback-start-symbolic", gtk::IconSize::SmallToolbar.into());
                self.pause_button.set_image(&image);
            }
            PlaybackState::Playing => {
                let image =
                    gtk::Image::new_from_icon_name("media-playback-pause-symbolic", gtk::IconSize::SmallToolbar.into());
                self.pause_button.set_image(&image);
            }
            _ => {}
        };
    }

    pub fn update_subtitle_track_menu(&self, section: gio::Menu) {
        // TODO: Would be nice to keep previous external subs in the menu.
        self.subtitle_track_menu.remove_all();
        self.subtitle_track_menu.append_section(None, &section);
    }

    pub fn update_audio_track_menu(&self, section: gio::Menu) {
        self.audio_track_menu.remove_all();
        self.audio_track_menu.append_section(None, &section);
    }

    pub fn update_video_track_menu(&self, section: gio::Menu) {
        self.video_track_menu.remove_all();
        self.video_track_menu.append_section(None, &section);
    }

    pub fn clear_audio_visualization_menu(&self) {
        self.audio_visualization_menu.remove_all();
    }

    pub fn update_audio_visualization_menu(&self, section: gio::Menu) {
        self.audio_visualization_menu.append_section(None, &section);
        self.audio_visualization_menu.freeze();
    }

    pub fn mutable_audio_visualization_menu(&self) -> bool {
        self.audio_visualization_menu.is_mutable()
    }
}
