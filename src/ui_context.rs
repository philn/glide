extern crate gio;
extern crate glib;
extern crate gstreamer as gst;
extern crate gtk4 as gtk;

#[allow(unused_imports)]
use gio::prelude::*;
#[allow(unused_imports)]
use glib::SendWeakRef;
use gtk::gdk;
use gtk::prelude::*;
use std::cmp;
#[allow(unused_imports)]
use std::os::raw::c_void;
use std::string;
use std::sync::Mutex;

use crate::PlaybackState;

lazy_static! {
    pub static ref INHIBIT_COOKIE: Mutex<Option<u32>> = Mutex::new(None);
    pub static ref INITIAL_POSITION: Mutex<Option<(i32, i32)>> = Mutex::new(None);
    pub static ref INITIAL_SIZE: Mutex<Option<(i32, i32)>> = Mutex::new(None);
    pub static ref MOUSE_NOTIFY_SIGNAL_ID: Mutex<Option<glib::SignalHandlerId>> = Mutex::new(None);
    pub static ref AUTOHIDE_SOURCE: Mutex<Option<glib::SourceId>> = Mutex::new(None);
}

#[cfg(target_os = "macos")]
use crate::iokit_sleep_disabler;

pub fn create_app() -> gtk::Application {
    let gtk_app = gtk::Application::new(Some("net.baseart.Glide"), gio::ApplicationFlags::HANDLES_OPEN);

    if let Some(settings) = gtk::Settings::default() {
        settings.set_property("gtk-application-prefer-dark-theme", &true);
    }

    gtk_app
}

pub struct UIContext {
    window: gtk::ApplicationWindow,
    main_box: gtk::Box,
    video_renderer: gtk::Picture,
    pause_button: gtk::Button,
    progress_bar: gtk::Scale,
    volume_button: gtk::VolumeButton,
    toolbar_box: gtk::Box,
    track_synchronization_window: gtk::ApplicationWindow,
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
    app: gtk::Application,
}

const MINIMAL_WINDOW_SIZE: (i32, i32) = (640, 480);
const VERSION: &str = env!("CARGO_PKG_VERSION");

impl UIContext {
    pub fn new(gtk_app: gtk::Application) -> Self {
        let builder = gtk::Builder::from_string(include_str!("../data/net.baseart.Glide.ui"));

        let pause_button = {
            let button: gtk::Button = builder.object("pause-button").unwrap();
            button
                .clone()
                .upcast::<gtk::Actionable>()
                .set_action_name(Some("app.pause"));
            button
        };

        //image.set_icon_size(gtk::IconSize::SmallToolbar);
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

        let main_box: gtk::Box = builder.object("main-box").unwrap();
        let video_renderer: gtk::Picture = builder.object("video-renderer").unwrap();
        let toolbar_box: gtk::Box = builder.object("toolbar-box").unwrap();
        let progress_bar: gtk::Scale = builder.object("progress-bar").unwrap();
        let volume_button: gtk::VolumeButton = builder.object("volume-button").unwrap();

        let window: gtk::ApplicationWindow = builder.object("application-window").unwrap();
        // window.connect_map_event(move |widget, _| {
        //     if let Ok(size) = INITIAL_SIZE.lock() {
        //         if let Some((width, height)) = *size {
        //             widget.resize(width, height);
        //         }
        //     }
        //     if let Ok(position) = INITIAL_POSITION.lock() {
        //         if let Some((x, y)) = *position {
        //             widget.move_(x, y);
        //         }
        //     }
        //     gtk::Inhibit(false)
        // });

        let track_synchronization_window: gtk::ApplicationWindow = builder.object("synchronization-window").unwrap();

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

        let audio_offset_entry: gtk::SpinButton = builder.object("audio-video-offset").unwrap();
        let subtitle_offset_entry: gtk::SpinButton = builder.object("subtitle-video-offset").unwrap();

        let subtitle_track_menu: gio::Menu = builder.object("subtitle-track-menu").unwrap();
        let audio_track_menu: gio::Menu = builder.object("audio-track-menu").unwrap();
        let video_track_menu: gio::Menu = builder.object("video-track-menu").unwrap();
        let audio_visualization_menu: gio::Menu = builder.object("audio-visualization-menu").unwrap();

        let menu: gio::Menu = builder.object("main-menu").unwrap();

        #[cfg(not(target_os = "linux"))]
        {
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
            ];
            for (action, accels) in accels_per_action.iter() {
                app.set_accels_for_action(&format!("app.{}", action), accels);
            }

            if let Some(window) = window_weak.upgrade() {
                window.set_application(Some(app));
            }

            #[cfg(not(target_os = "linux"))]
            {
                app.set_menubar(Some(&menu));
            }
        });

        Self {
            window,
            main_box,
            video_renderer,
            pause_button,
            progress_bar,
            volume_button,
            toolbar_box,
            track_synchronization_window,
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
        window.set_modal(true);
        window.set_application(Some(&self.app));
        window.show();
    }

    #[cfg(target_os = "linux")]
    pub fn start_autohide_toolbar(&self) {
        let toolbar_weak = self.toolbar_box.downgrade();
        // let notify_signal_id = self.window.connect_motion_notify_event(move |window, _| {
        //     if let Some(source) = AUTOHIDE_SOURCE.lock().unwrap().take() {
        //         source.remove();
        //     }

        //     let gdk_window = window.window().unwrap();
        //     gdk_window.set_cursor(None);

        //     let toolbar = match toolbar_weak.upgrade() {
        //         Some(t) => t,
        //         None => return gtk::Inhibit(false),
        //     };
        //     toolbar.set_visible(true);

        //     let window_weak = SendWeakRef::from(window.downgrade());
        //     let toolbar_weak = SendWeakRef::from(toolbar.downgrade());
        //     *AUTOHIDE_SOURCE.lock().unwrap() = Some(glib::timeout_add_seconds(5, move || {
        //         if let Ok(cookie) = INHIBIT_COOKIE.lock() {
        //             if cookie.is_some() {
        //                 if let Some(toolbar) = toolbar_weak.upgrade() {
        //                     toolbar.set_visible(false);
        //                 }
        //                 if let Some(window) = window_weak.upgrade() {
        //                     let gdk_window = window.window().unwrap();
        //                     let cursor = gtk::gdk::Cursor::from_name(&gdk_window.display(), "blank");
        //                     gdk_window.set_cursor(cursor.as_ref());
        //                 }
        //             }
        //         }
        //         *AUTOHIDE_SOURCE.lock().unwrap() = None;
        //         glib::Continue(false)
        //     }));
        //     gtk::Inhibit(false)
        // });
        // *MOUSE_NOTIFY_SIGNAL_ID.lock().unwrap() = Some(notify_signal_id);
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
        window.set_show_menubar(false);
        self.toolbar_box.set_visible(false);
        window.fullscreen();
        let cursor = gtk::gdk::Cursor::from_name("none", None);
        window.set_cursor(cursor.as_ref());

        #[cfg(target_os = "linux")]
        self.start_autohide_toolbar();
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
        if let Ok(mut signal_handler_id) = MOUSE_NOTIFY_SIGNAL_ID.lock() {
            if let Some(handler) = signal_handler_id.take() {
                window.disconnect(handler);
            }
        }
        window.unfullscreen();
        self.toolbar_box.set_visible(true);
        window.set_show_menubar(true);
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
            gtk::Inhibit(false)
        });
    }

    pub fn stop(&self) {
        self.app.quit();
    }

    pub fn set_progress_bar_format_callback<F>(&self, f: F)
    where
        F: Fn(f64, f64) -> string::String + Send + Sync + 'static,
    {
        // //let range = self.progress_bar.clone().upcast::<gtk::Range>();
        // self.progress_bar.connect_value_changed(move |widget| {
        //     let scale = widget.clone().upcast::<gtk::Scale>();
        //     //f(value, range.adjustment().upper())
        //     widget.set_text(f(widget.value()));
        // });
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
        self.window.add_controller(&dest);
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

    pub fn resize_window(&self, width: i32, height: i32) {
        // let mut width = width;
        // let mut height = height;
        // let display = self.window.display();
        // let win = self.window.window().unwrap();
        // if let Some(monitor) = display.monitor_at_window(&win) {
        //     let geometry = monitor.geometry();
        //     width = cmp::min(width, geometry.width());
        //     height = cmp::min(height, geometry.height() - 100);
        // }

        // // FIXME: Somehow resize video_area to avoid black borders.
        // if width > MINIMAL_WINDOW_SIZE.0 && height > MINIMAL_WINDOW_SIZE.1 {
        //     self.window.resize(width, height);
        // }
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
        let dialog = gtk::AboutDialog::new();
        dialog.set_authors(&["Philippe Normand"]);
        dialog.set_website_label("base-art.net");
        dialog.set_website(Some("http://base-art.net"));
        dialog.set_title(Some("About"));
        dialog.set_version(Some(VERSION));
        let s = format!(
            "Multimedia playback support provided by {}.\nUser interface running on GTK {}.{}.{}",
            gst::version_string(),
            gtk::major_version(),
            gtk::minor_version(),
            gtk::micro_version()
        );
        dialog.set_comments(Some(s.as_str()));
        dialog.set_transient_for(Some(&self.window));
        dialog.show();
    }

    pub fn playback_state_changed(&self, playback_state: &PlaybackState) {
        match playback_state {
            PlaybackState::Paused => {
                //image.set_icon_size(gtk::IconSize::SmallToolbar);
                self.pause_button.set_icon_name("media-playback-start-symbolic");
            }
            PlaybackState::Playing => {
                //image.set_icon_size(gtk::IconSize::SmallToolbar);
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
