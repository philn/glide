extern crate adw;
extern crate gio;
extern crate gstreamer as gst;
extern crate gtk4 as gtk;

#[allow(unused_imports)]
use gio::prelude::*;
#[allow(unused_imports)]
use glib::SendWeakRef;
use gstreamer::glib;
use gtk::gdk;
use gtk::prelude::*;
#[allow(unused_imports)]
use std::os::raw::c_void;
use std::string;
use std::sync::Mutex;

use crate::PlaybackState;

lazy_static! {
    pub static ref INHIBIT_COOKIE: Mutex<Option<u32>> = Mutex::new(None);
    pub static ref INITIAL_POSITION: Mutex<Option<(i32, i32)>> = Mutex::new(None);
    pub static ref INITIAL_SIZE: Mutex<Option<(i32, i32)>> = Mutex::new(None);
    pub static ref AUTOHIDE_SOURCE: Mutex<Option<glib::SourceId>> = Mutex::new(None);
}

#[cfg(target_os = "macos")]
use crate::iokit_sleep_disabler;

pub fn create_app() -> adw::Application {
    let gtk_app = adw::Application::builder()
        .application_id("net.baseart.Glide")
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    let style_manager = adw::StyleManager::default();
    style_manager.set_property("color-scheme", adw::ColorScheme::PreferDark);

    gtk_app
}

pub struct UIContext {
    window: adw::ApplicationWindow,
    header_bar: gtk::HeaderBar,
    motion_controller: gtk::EventControllerMotion,
    video_renderer: gtk::Picture,
    pause_button: gtk::Button,
    progress_bar: gtk::Scale,
    volume_button: gtk::VolumeButton,
    toolbar_revealer: gtk::Revealer,
    track_synchronization_window: adw::ApplicationWindow,
    shortcuts_window: gtk::ShortcutsWindow,
    audio_offset_entry: gtk::SpinButton,
    subtitle_offset_entry: gtk::SpinButton,
    subtitle_track_menu: gio::Menu,
    audio_track_menu: gio::Menu,
    video_track_menu: gio::Menu,
    audio_visualization_menu: gio::Menu,
    volume_signal_handler_id: Option<glib::SignalHandlerId>,
    position_signal_handler_id: Option<glib::SignalHandlerId>,
    audio_offset_entry_signal_handler_id: Option<glib::SignalHandlerId>,
    subtitle_offset_entry_signal_handler_id: Option<glib::SignalHandlerId>,
    app: adw::Application,
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

impl UIContext {
    pub fn new(gtk_app: adw::Application) -> Self {
        let builder = gtk::Builder::from_string(include_str!("../data/net.baseart.Glide.ui"));

        let header_bar: gtk::HeaderBar = builder.object("header-bar").unwrap();

        let pause_button = {
            let button: gtk::Button = builder.object("pause-button").unwrap();
            button
                .clone()
                .upcast::<gtk::Actionable>()
                .set_action_name(Some("app.pause"));
            button
        };

        pause_button.set_icon_name("media-playback-start-symbolic");

        let button: gtk::Button = builder.object("seek-backward-button").unwrap();
        button
            .upcast::<gtk::Actionable>()
            .set_action_name(Some("app.seek-backward"));

        let button: gtk::Button = builder.object("seek-forward-button").unwrap();
        button
            .upcast::<gtk::Actionable>()
            .set_action_name(Some("app.seek-forward"));

        let button: gtk::Button = builder.object("fullscreen-button").unwrap();
        button
            .upcast::<gtk::Actionable>()
            .set_action_name(Some("app.fullscreen"));

        let video_renderer: gtk::Picture = builder.object("video-renderer").unwrap();
        let progress_bar: gtk::Scale = builder.object("progress-bar").unwrap();
        let volume_button: gtk::VolumeButton = builder.object("volume-button").unwrap();

        let toolbar_revealer: gtk::Revealer = builder.object("toolbar-revealer").unwrap();

        video_renderer.set_content_fit(gtk::ContentFit::Fill);

        let window: adw::ApplicationWindow = builder.object("application-window").unwrap();

        let track_synchronization_window: adw::ApplicationWindow = builder.object("synchronization-window").unwrap();

        let action = gio::SimpleAction::new("close", None);
        track_synchronization_window.add_action(&action);
        let window_weak = SendWeakRef::from(track_synchronization_window.downgrade());
        action.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                window.hide();
            }
        });

        let button: gtk::Button = builder.object("synchronization-window-close-button").unwrap();
        button.upcast::<gtk::Actionable>().set_action_name(Some("win.close"));

        let button: gtk::Button = builder.object("audio-offset-reset-button").unwrap();
        button
            .upcast::<gtk::Actionable>()
            .set_action_name(Some("app.audio-offset-reset"));

        let button: gtk::Button = builder.object("subtitle-offset-reset-button").unwrap();
        button
            .upcast::<gtk::Actionable>()
            .set_action_name(Some("app.subtitle-offset-reset"));

        let button: gtk::Button = builder.object("video-frame-step-button").unwrap();
        button
            .upcast::<gtk::Actionable>()
            .set_action_name(Some("app.video-frame-step"));

        let audio_offset_entry: gtk::SpinButton = builder.object("audio-video-offset").unwrap();
        let subtitle_offset_entry: gtk::SpinButton = builder.object("subtitle-video-offset").unwrap();

        let subtitle_track_menu: gio::Menu = builder.object("subtitle-track-menu").unwrap();
        let audio_track_menu: gio::Menu = builder.object("audio-track-menu").unwrap();
        let video_track_menu: gio::Menu = builder.object("video-track-menu").unwrap();
        let audio_visualization_menu: gio::Menu = builder.object("audio-visualization-menu").unwrap();

        let shortcuts_window: gtk::ShortcutsWindow = builder.object("shortcuts-window").unwrap();

        #[cfg(not(target_os = "linux"))]
        {
            let menu: gio::Menu = builder.object("main-menu").unwrap();
            menu.append(Some("Quit"), Some("app.quit"));
            menu.append(Some("About"), Some("app.about"));
        }

        let window_weak = SendWeakRef::from(window.downgrade());
        gtk_app.connect_startup(move |app| {
            let accels_per_action = [
                ("open-media", ["<Primary>o"]),
                ("quit", ["<Primary>q"]),
                ("fullscreen", ["<Primary>f"]),
                ("restore", ["Escape"]),
                ("pause", ["space"]),
                ("seek-forward", ["<Primary>Right"]),
                ("seek-backward", ["<Primary>Left"]),
                ("audio-volume-increase", ["<Primary>Up"]),
                ("audio-volume-decrease", ["<Primary>Down"]),
                ("audio-mute", ["<Primary>m"]),
                ("open-subtitle-file", ["<Primary>s"]),
                ("dump-pipeline", ["<Ctrl>d"]),
                ("show-shortcuts", ["<Primary>question"]),
                ("video-frame-step", ["<Primary>n"]),
            ];
            for (action, accels) in accels_per_action.iter() {
                app.set_accels_for_action(&format!("app.{action}"), accels);
            }

            if let Some(window) = window_weak.upgrade() {
                window.set_application(Some(app));
            }
        });

        let motion_controller = gtk::EventControllerMotion::builder().build();
        window.add_controller(motion_controller.clone());

        Self {
            window,
            header_bar,
            motion_controller,
            video_renderer,
            pause_button,
            progress_bar,
            volume_button,
            toolbar_revealer,
            track_synchronization_window,
            shortcuts_window,
            audio_offset_entry,
            subtitle_offset_entry,
            subtitle_track_menu,
            audio_track_menu,
            video_track_menu,
            audio_visualization_menu,
            volume_signal_handler_id: None,
            position_signal_handler_id: None,
            audio_offset_entry_signal_handler_id: None,
            subtitle_offset_entry_signal_handler_id: None,
            app: gtk_app,
        }
    }

    pub fn open_track_synchronization_window(&self) {
        let window = &self.track_synchronization_window;
        window.set_transient_for(Some(&self.window));
        window.show();
    }

    pub fn show_shortcuts(&self) {
        let window = &self.shortcuts_window;
        window.set_transient_for(Some(&self.window));
        window.show();
    }

    pub fn start_autohide_toolbar(&self) {
        let toolbar_weak = self.toolbar_revealer.downgrade();
        let window_weak = self.window.downgrade();
        let coords_cache: std::sync::Arc<Mutex<(f64, f64)>> = std::sync::Arc::new(Mutex::new((0.0, 0.0)));
        self.motion_controller.connect_motion(move |_controller, x, y| {
            let coords = (x, y);
            let mut cached_coords = coords_cache.lock().unwrap();
            if coords == *cached_coords {
                return;
            }
            *cached_coords = coords;

            if let Some(source) = AUTOHIDE_SOURCE.lock().unwrap().take() {
                source.remove();
            }

            let window = match window_weak.upgrade() {
                Some(t) => t,
                None => return,
            };

            let cursor = gtk::gdk::Cursor::from_name("default", None);
            window.set_cursor(cursor.as_ref());

            let toolbar = match toolbar_weak.upgrade() {
                Some(t) => t,
                None => return,
            };
            toolbar.set_reveal_child(true);

            let window_weak2 = SendWeakRef::from(window_weak.clone());
            let toolbar_weak2 = SendWeakRef::from(toolbar_weak.clone());
            *AUTOHIDE_SOURCE.lock().unwrap() = Some(glib::timeout_add_seconds(5, move || {
                if let Some(toolbar) = toolbar_weak2.upgrade() {
                    toolbar.set_reveal_child(false);
                }
                if let Ok(cookie) = INHIBIT_COOKIE.lock() {
                    if cookie.is_some() {
                        let window = match window_weak2.upgrade() {
                            Some(t) => t,
                            None => {
                                *AUTOHIDE_SOURCE.lock().unwrap() = None;
                                return glib::ControlFlow::Break;
                            }
                        };

                        let cursor = gtk::gdk::Cursor::from_name("none", None);
                        window.set_cursor(cursor.as_ref());
                    }
                }
                *AUTOHIDE_SOURCE.lock().unwrap() = None;
                glib::ControlFlow::Break
            }));
        });
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
            *INHIBIT_COOKIE.lock().unwrap() = Some(self.app.inhibit(Some(window), flags, Some("Glide full-screen")));
        }
        *INITIAL_SIZE.lock().unwrap() = Some(window.default_size());
        //*INITIAL_POSITION.lock().unwrap() = Some(window.position());
        self.header_bar.hide();
        window.fullscreen();
        let cursor = gtk::gdk::Cursor::from_name("none", None);
        window.set_cursor(cursor.as_ref());
    }

    pub fn leave_fullscreen(&self) {
        let window = &self.window;
        if let Ok(mut cookie) = INHIBIT_COOKIE.lock() {
            #[cfg(target_os = "macos")]
            iokit_sleep_disabler::release_sleep_assertion(cookie.unwrap());
            #[cfg(not(target_os = "macos"))]
            self.app.uninhibit(cookie.unwrap());
            *cookie = None;
        }
        window.unfullscreen();
        self.header_bar.show();
        let cursor = gtk::gdk::Cursor::from_name("default", None);
        window.set_cursor(cursor.as_ref());
    }

    pub fn open_dialog<F>(&self, relative_uri: Option<glib::GString>, f: F)
    where
        F: Fn(glib::GString) + Send + Sync + 'static,
    {
        let dialog = gtk::FileChooserDialog::new(
            Some("Choose a file"),
            Some(&self.window),
            gtk::FileChooserAction::Open,
            &[("Open", gtk::ResponseType::Ok), ("Cancel", gtk::ResponseType::Cancel)],
        );

        dialog.set_select_multiple(true);
        if let Some(uri) = relative_uri {
            if let Ok((filename, _)) = glib::filename_from_uri(&uri) {
                if let Some(folder) = filename.parent() {
                    dialog.set_current_folder(Some(&gio::File::for_path(folder))).unwrap();
                }
            }
        }

        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Ok {
                let file = dialog.file().unwrap();
                let filename = file.uri();
                f(filename);
            }
            dialog.close();
        });
        dialog.show();
    }

    pub fn start<F: Fn() + Send + Sync + 'static>(&self, f: F) {
        self.window.show();
        self.window.connect_close_request(move |_| {
            f();
            glib::Propagation::Proceed
        });

        self.start_autohide_toolbar();
    }

    pub fn stop(&self) {
        self.app.quit();
    }

    pub fn set_progress_bar_format_callback<F>(&self, f: F)
    where
        F: Fn(f64) -> string::String + Send + Sync + 'static,
    {
        self.progress_bar
            .set_format_value_func(move |_scale: &gtk::Scale, value: f64| f(value));
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
            f(range.value() as u64);
        }));
    }

    pub fn set_drop_data_callback<F: Fn(&str) + Send + Sync + 'static>(&mut self, f: F) {
        let dest = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY);
        dest.set_types(&[gio::File::static_type()]);
        dest.connect_drop(move |target, _value, _x, _y| -> bool {
            if let Some(value) = target.value_as::<gio::File>() {
                f(&value.uri());
            }
            true
        });
        self.window.add_controller(dest);
    }

    pub fn set_audio_offset_entry_updated_callback<F: Fn(i64) + Send + Sync + 'static>(&mut self, f: F) {
        let entry = self.audio_offset_entry.clone();
        self.audio_offset_entry_signal_handler_id = Some(entry.connect_value_changed(move |button| {
            f((button.value() * 1000000000_f64) as i64);
        }));
    }

    pub fn set_subtitle_offset_entry_updated_callback<F: Fn(i64) + Send + Sync + 'static>(&mut self, f: F) {
        let entry = self.subtitle_offset_entry.clone();
        self.subtitle_offset_entry_signal_handler_id = Some(entry.connect_value_changed(move |button| {
            f((button.value() * 1000000000_f64) as i64);
        }));
    }

    pub fn volume_changed(&self, volume: f64) {
        let button = &self.volume_button;
        let scale = button.clone().upcast::<gtk::ScaleButton>();
        if let Some(ref handler_id) = self.volume_signal_handler_id {
            glib::signal_handler_block(&scale, handler_id);
            scale.set_value(volume);
            glib::signal_handler_unblock(&scale, handler_id);
        }
    }

    pub fn audio_video_offset_changed(&self, offset: i64) {
        let entry = &self.audio_offset_entry;
        if let Some(ref handler_id) = self.audio_offset_entry_signal_handler_id {
            glib::signal_handler_block(entry, handler_id);
            entry.set_value(offset as f64 / 1000000000_f64);
            glib::signal_handler_unblock(entry, handler_id);
        }
    }

    pub fn subtitle_video_offset_changed(&self, offset: i64) {
        let entry = &self.subtitle_offset_entry;
        if let Some(ref handler_id) = self.subtitle_offset_entry_signal_handler_id {
            glib::signal_handler_block(entry, handler_id);
            entry.set_value(offset as f64 / 1000000000_f64);
            glib::signal_handler_unblock(entry, handler_id);
        }
    }

    pub fn set_position_range_value(&self, position: u64) {
        let range = self.progress_bar.clone().upcast::<gtk::Range>();
        if let Some(ref handler_id) = self.position_signal_handler_id {
            glib::signal_handler_block(&range, handler_id);
            range.set_value(position as f64);
            glib::signal_handler_unblock(&range, handler_id);
        }
    }

    pub fn set_video_paintable(&self, paintable: &gdk::Paintable) {
        self.video_renderer.set_paintable(Some(paintable));
    }

    pub fn resize_window(&self, width: u32, height: u32) {
        self.window.set_default_size(width as i32, height as i32);
    }

    pub fn set_window_title(&self, title: &str) {
        self.window.set_title(Some(title));
    }

    pub fn set_position_range_end(&self, end: f64) {
        let progress_bar = &self.progress_bar;
        let range = progress_bar.clone().upcast::<gtk::Range>();
        if let Some(ref handler_id) = self.position_signal_handler_id {
            glib::signal_handler_block(&range, handler_id);
            range.set_range(0.0, end);
            glib::signal_handler_unblock(&range, handler_id);
        }

        // Force the GtkScale to recompute its label widget size.
        progress_bar.set_draw_value(false);
        progress_bar.set_draw_value(true);
    }

    pub fn display_about_dialog(&self) {
        let s = format!(
            "Multimedia playback support provided by {}.\nUser interface running on GTK {}.{}.{}",
            gst::version_string(),
            gtk::major_version(),
            gtk::minor_version(),
            gtk::micro_version()
        );
        let dialog = adw::AboutWindow::builder()
            .application_name("Glide")
            .developer_name("Philippe Normand")
            .website("http://github.com/philn/glide")
            .issue_url("https://github.com/philn/glide/issues/new")
            .version(VERSION)
            .debug_info(s)
            .application(&self.app)
            .transient_for(&self.window)
            .build();
        dialog.show();
    }

    pub fn playback_state_changed(&self, playback_state: &PlaybackState) {
        match playback_state {
            PlaybackState::Paused => {
                self.pause_button.set_icon_name("media-playback-start-symbolic");
            }
            PlaybackState::Playing => {
                self.pause_button.set_icon_name("media-playback-pause-symbolic");
            }
            _ => {}
        };
    }

    pub fn update_subtitle_track_menu(&self, section: &gio::Menu) {
        // TODO: Would be nice to keep previous external subs in the menu.
        self.subtitle_track_menu.remove_all();
        self.subtitle_track_menu.append_section(None, section);
    }

    pub fn update_audio_track_menu(&self, section: &gio::Menu) {
        self.audio_track_menu.remove_all();
        self.audio_track_menu.append_section(None, section);
    }

    pub fn update_video_track_menu(&self, section: &gio::Menu) {
        self.video_track_menu.remove_all();
        self.video_track_menu.append_section(None, section);
    }

    pub fn clear_audio_visualization_menu(&self) {
        self.audio_visualization_menu.remove_all();
    }

    pub fn update_audio_visualization_menu(&self, section: &gio::Menu) {
        self.audio_visualization_menu.append_section(None, section);
        self.audio_visualization_menu.freeze();
    }

    pub fn mutable_audio_visualization_menu(&self) -> bool {
        self.audio_visualization_menu.is_mutable()
    }
}
