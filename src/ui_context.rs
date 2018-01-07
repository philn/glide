extern crate gdk;
extern crate gio;
extern crate glib;
extern crate gtk;
extern crate send_cell;

use gdk::prelude::*;
#[allow(unused_imports)]
use gio::prelude::*;
use gtk::prelude::*;
#[allow(unused_imports)]
use send_cell::SendCell;

use common::{INHIBIT_COOKIE, INITIAL_POSITION, INITIAL_SIZE};

#[derive(Clone)]
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
}

impl UIContext {
    pub fn new(gtk_app: &gtk::Application) -> Self {
        let window = gtk::ApplicationWindow::new(gtk_app);
        window.set_default_size(320, 240);
        window.set_resizable(true);

        let main_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let toolbar_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);

        let pause_button = gtk::Button::new();
        let pause_actionable = pause_button.clone().upcast::<gtk::Actionable>();
        pause_actionable.set_action_name("app.pause");

        let seek_backward_button = gtk::Button::new();
        let seek_bw_actionable = seek_backward_button.clone().upcast::<gtk::Actionable>();
        seek_bw_actionable.set_action_name("app.seek-backward");
        let backward_image = gtk::Image::new_from_icon_name(
            "media-seek-backward-symbolic",
            gtk::IconSize::SmallToolbar.into(),
        );
        seek_backward_button.set_image(&backward_image);

        let seek_forward_button = gtk::Button::new();
        let seek_fw_actionable = seek_forward_button.clone().upcast::<gtk::Actionable>();
        seek_fw_actionable.set_action_name("app.seek-forward");
        let forward_image = gtk::Image::new_from_icon_name(
            "media-seek-forward-symbolic",
            gtk::IconSize::SmallToolbar.into(),
        );
        seek_forward_button.set_image(&forward_image);

        toolbar_box.pack_start(&seek_backward_button, false, false, 0);
        toolbar_box.pack_start(&pause_button, false, false, 0);
        toolbar_box.pack_start(&seek_forward_button, false, false, 0);

        let progress_bar = gtk::Scale::new(gtk::Orientation::Horizontal, None);
        progress_bar.set_draw_value(true);
        progress_bar.set_value_pos(gtk::PositionType::Right);

        toolbar_box.pack_start(&progress_bar, true, true, 10);

        let volume_button = gtk::VolumeButton::new();
        let volume_orientable = volume_button.clone().upcast::<gtk::Orientable>();
        volume_orientable.set_orientation(gtk::Orientation::Horizontal);
        toolbar_box.pack_start(&volume_button, false, false, 5);

        let fullscreen_button = gtk::Button::new();
        let fullscreen_image = gtk::Image::new_from_icon_name(
            "view-fullscreen-symbolic",
            gtk::IconSize::SmallToolbar.into(),
        );
        fullscreen_button.set_image(&fullscreen_image);
        let fs_actionable = fullscreen_button.clone().upcast::<gtk::Actionable>();
        fs_actionable.set_action_name("app.fullscreen");

        toolbar_box.pack_start(&fullscreen_button, false, false, 0);

        main_box.pack_start(&toolbar_box, false, false, 10);
        window.add(&main_box);

        UIContext {
            window,
            main_box,
            seek_backward_button,
            seek_forward_button,
            pause_button,
            fullscreen_button,
            progress_bar,
            volume_button,
            toolbar_box,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn start_autohide_toolbar(&self, fullscreen_action: &gio::SimpleAction) {
        let toolbar = SendCell::new(self.toolbar_box.clone());
        self.window
            .connect_motion_notify_event(clone_army!([toolbar, fullscreen_action] move |window, _| {
            if let Some(is_fullscreen) = fullscreen_action.get_state() {
                let fullscreen = is_fullscreen.get::<bool>().unwrap();
                if fullscreen {
                    let toolbar = toolbar.borrow();
                    toolbar.set_visible(true);
                    let gdk_window = window.get_window().unwrap();
                    gdk_window.set_cursor(None);

                    let inner_toolbar = SendCell::new(toolbar.clone());
                    let inner_window = SendCell::new(window.clone());
                    glib::timeout_add_seconds(5, clone_army!([inner_window, inner_toolbar] move || {
                        let cursor = gdk::Cursor::new(gdk::CursorType::BlankCursor);
                        let window = inner_window.borrow();
                        let gdk_window = window.get_window().unwrap();
                        let toolbar = inner_toolbar.borrow();
                        toolbar.set_visible(false);
                        gdk_window.set_cursor(Some(&cursor));
                        glib::Continue(false)
                    }));
                }
            }
            gtk::Inhibit(false)
        }));
    }

    pub fn enter_fullscreen(&self, app: &gtk::Application) {
        let window = &self.window;
        let flags = gtk::ApplicationInhibitFlags::SUSPEND | gtk::ApplicationInhibitFlags::IDLE;
        *INHIBIT_COOKIE.lock().unwrap() = Some(app.inhibit(window, flags, None));
        *INITIAL_SIZE.lock().unwrap() = Some(window.get_size());
        *INITIAL_POSITION.lock().unwrap() = Some(window.get_position());
        window.set_show_menubar(false);
        self.toolbar_box.set_visible(false);
        window.fullscreen();
        let cursor = gdk::Cursor::new(gdk::CursorType::BlankCursor);
        let gdk_window = window.get_window().unwrap();
        gdk_window.set_cursor(Some(&cursor));
    }

    pub fn leave_fullscreen(&self, app: &gtk::Application) {
        let window = &self.window;
        let gdk_window = window.get_window().unwrap();
        if let Ok(mut cookie) = INHIBIT_COOKIE.lock() {
            app.uninhibit(cookie.unwrap());
            *cookie = None;
        }
        window.unfullscreen();
        self.toolbar_box.set_visible(true);
        window.set_show_menubar(true);
        gdk_window.set_cursor(None);
    }
}
