extern crate gio;
#[allow(unused_imports)]
use gio::prelude::*;

use crate::channel_player::PlaybackState;
use crate::video_renderer;

pub trait Application {
    fn add_action(&self, action: &gio::SimpleAction) {}

    fn display_about_dialog(&self) {}

    fn as_gtk_application(&self) -> Option<gtk::Application> {
        None
    }

    fn start(&self) {}
    fn stop(&self) {}

    fn enter_fullscreen(&self) {}

    fn leave_fullscreen(&self) {}

    fn dialog_result(&self, relative_uri: Option<glib::GString>) -> Option<glib::GString> {
        None
    }

    fn set_video_renderer(&self, renderer: &video_renderer::VideoRenderer) {}

    fn volume_changed(&self, volume: f64) {}

    fn set_position_range_value(&self, position: u64) {}
    fn set_position_range_end(&self, end: f64) {}

    fn resize_window(&self, width: i32, height: i32) {}

    fn set_window_title(&self, title: &str) {}

    fn playback_state_changed(&self, playback_state: &PlaybackState) {}
    fn update_subtitle_track_menu(&self, section: &gio::Menu) {}
    fn update_audio_track_menu(&self, section: &gio::Menu) {}
    fn update_video_track_menu(&self, section: &gio::Menu) {}

    fn clear_audio_visualization_menu(&self) {}
    fn update_audio_visualization_menu(&self, section: &gio::Menu) {}
    fn mutable_audio_visualization_menu(&self) -> bool {
        false
    }
}
