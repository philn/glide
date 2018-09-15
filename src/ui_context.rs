extern crate gdk;
extern crate gio;
extern crate glib;
extern crate gtk;

use gdk::prelude::*;
#[allow(unused_imports)]
use gio::prelude::*;
#[allow(unused_imports)]
use glib::SendWeakRef;
use gtk::prelude::*;
use std::cmp;
use std::string;
use std::sync::Mutex;

lazy_static! {
    pub static ref INHIBIT_COOKIE: Mutex<Option<u32>> = { Mutex::new(None) };
    pub static ref INITIAL_POSITION: Mutex<Option<(i32, i32)>> = { Mutex::new(None) };
    pub static ref INITIAL_SIZE: Mutex<Option<(i32, i32)>> = { Mutex::new(None) };
    pub static ref MOUSE_NOTIFY_SIGNAL_ID: Mutex<Option<glib::SignalHandlerId>> = { Mutex::new(None) };
    pub static ref AUTOHIDE_SOURCE: Mutex<Option<glib::SourceId>> = { Mutex::new(None) };
}

#[cfg(target_os = "macos")]
use iokit_sleep_disabler;

pub struct UIContext {
    pub window: gtk::ApplicationWindow,
    pub main_box: gtk::Box,
    pub pause_button: gtk::Button,
    pub seek_backward_button: gtk::Button,
    pub seek_forward_button: gtk::Button,
    pub fullscreen_button: gtk::Button,
    pub progress_bar: gtk::Scale,
    pub volume_button: gtk::VolumeButton,
    pub toolbar_box: gtk::Box,
    volume_signal_handler_id: Option<glib::SignalHandlerId>,
    position_signal_handler_id: Option<glib::SignalHandlerId>,
}

static MINIMAL_WINDOW_SIZE: (i32, i32) = (640, 480);

impl UIContext {
    pub fn new(gtk_app: &gtk::Application) -> Self {
        let window = gtk::ApplicationWindow::new(gtk_app);
        window.set_default_size(MINIMAL_WINDOW_SIZE.0, MINIMAL_WINDOW_SIZE.1);

        let main_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let toolbar_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);

        let pause_button = {
            let button = gtk::Button::new();
            button.clone().upcast::<gtk::Actionable>().set_action_name("app.pause");
            button
        };

        let seek_backward_button = {
            let button = gtk::Button::new();
            button.clone().upcast::<gtk::Actionable>().set_action_name("app.seek-backward");
            let image = 
                gtk::Image::new_from_icon_name("media-seek-backward-symbolic", gtk::IconSize::SmallToolbar.into());
            button.set_image(&image);
            button
        };

        let seek_forward_button = {
            let button = gtk::Button::new();
            button.clone().upcast::<gtk::Actionable>().set_action_name("app.seek-forward");
            let image = 
                gtk::Image::new_from_icon_name("media-seek-forward-symbolic", gtk::IconSize::SmallToolbar.into());
            button.set_image(&image);
            button
        };

        toolbar_box.pack_start(&seek_backward_button, false, false, 0);
        toolbar_box.pack_start(&pause_button, false, false, 0);
        toolbar_box.pack_start(&seek_forward_button, false, false, 0);

        let progress_bar = {
            let bar = gtk::Scale::new(gtk::Orientation::Horizontal, None);
            bar.set_draw_value(true);
            bar.set_value_pos(gtk::PositionType::Right);
            bar
        };

        toolbar_box.pack_start(&progress_bar, true, true, 10);

        let volume_button = {
            let button = gtk::VolumeButton::new();
            button.clone().upcast::<gtk::Orientable>().set_orientation(gtk::Orientation::Horizontal);
            button
        };

        toolbar_box.pack_start(&volume_button, false, false, 5);

        let fullscreen_button = {
            let button = gtk::Button::new();
            let fullscreen_image =
                gtk::Image::new_from_icon_name("view-fullscreen-symbolic", gtk::IconSize::SmallToolbar.into());
            button.set_image(&fullscreen_image);
            button.clone().upcast::<gtk::Actionable>().set_action_name("app.fullscreen");
            button
        };

        toolbar_box.pack_start(&fullscreen_button, false, false, 0);

        main_box.pack_start(&toolbar_box, false, false, 10);
        window.add(&main_box);

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

        Self {
            window,
            main_box,
            seek_backward_button,
            seek_forward_button,
            pause_button,
            fullscreen_button,
            progress_bar,
            volume_button,
            toolbar_box,
            volume_signal_handler_id: None,
            position_signal_handler_id: None,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn start_autohide_toolbar(&self) {
        let toolbar_weak = self.toolbar_box.downgrade();
        let notify_signal_id = self.window.connect_motion_notify_event(move |window, _| {
            let toolbar = match toolbar_weak.upgrade() {
                Some(t) => t,
                None => return gtk::Inhibit(false),
            };

            toolbar.set_visible(true);
            let gdk_window = window.get_window().unwrap();
            gdk_window.set_cursor(None);

            let window_weak = SendWeakRef::from(window.downgrade());
            let toolbar_weak = SendWeakRef::from(toolbar.downgrade());
            if let Some(source) = AUTOHIDE_SOURCE.lock().unwrap().take() {
                glib::source_remove(source);
            }
            *AUTOHIDE_SOURCE.lock().unwrap() = Some(glib::timeout_add_seconds(5, move || {
                let cursor = gdk::Cursor::new(gdk::CursorType::BlankCursor);
                let window = match window_weak.upgrade() {
                    Some(w) => w,
                    None => {
                        *AUTOHIDE_SOURCE.lock().unwrap() = None;
                        return glib::Continue(false);
                    }
                };
                if let Ok(cookie) = INHIBIT_COOKIE.lock() {
                    if cookie.is_some() {
                        let gdk_window = window.get_window().unwrap();
                        let toolbar = match toolbar_weak.upgrade() {
                            Some(t) => t,
                            None => {
                                *AUTOHIDE_SOURCE.lock().unwrap() = None;
                                return glib::Continue(false);
                            }
                        };
                        toolbar.set_visible(false);
                        gdk_window.set_cursor(Some(&cursor));
                    }
                }
                *AUTOHIDE_SOURCE.lock().unwrap() = None;
                glib::Continue(false)
            }));
            gtk::Inhibit(false)
        });
        *MOUSE_NOTIFY_SIGNAL_ID.lock().unwrap() = Some(notify_signal_id);
    }

    pub fn enter_fullscreen(&self, _app: &gtk::Application) {
        let window = &self.window;
        #[cfg(target_os = "macos")]
        {
            *INHIBIT_COOKIE.lock().unwrap() = Some(iokit_sleep_disabler::prevent_display_sleep("Glide full-screen"));
        }
        #[cfg(not(target_os = "macos"))]
        {
            let flags = gtk::ApplicationInhibitFlags::SUSPEND | gtk::ApplicationInhibitFlags::IDLE;
            *INHIBIT_COOKIE.lock().unwrap() = Some(_app.inhibit(window, flags, Some("Glide full-screen")));
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

    pub fn leave_fullscreen(&self, _app: &gtk::Application) {
        let window = &self.window;
        let gdk_window = window.get_window().unwrap();
        if let Ok(mut cookie) = INHIBIT_COOKIE.lock() {
            #[cfg(target_os = "macos")]
            iokit_sleep_disabler::release_sleep_assertion(cookie.unwrap());
            #[cfg(not(target_os = "macos"))]
            _app.uninhibit(cookie.unwrap());
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
}
