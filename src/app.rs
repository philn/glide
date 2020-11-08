extern crate failure;
extern crate gio;
extern crate glib;

#[allow(unused_imports)]
use gio::prelude::*;

use crate::channel_player::{ChannelPlayer, PlaybackState};
use crate::video_player;
use crate::video_renderer;

#[cfg(target_os = "macos")]
use fruitbasket::FruitApp;

pub enum ApplicationImpl {
    #[cfg(target_os = "linux")]
    GTK(gtk::Application),
    #[cfg(target_os = "macos")]
    Cocoa(FruitApp),
}

pub trait Application {
    fn set_args(&mut self, _args: &Vec<std::string::String>) {}
    fn post_init(&mut self, _player: &ChannelPlayer) {}

    fn add_action(&self, _action: &gio::SimpleAction) {}

    fn display_about_dialog(&self) {}

    fn implementation(&self) -> Option<ApplicationImpl> {
        None
    }

    fn glib_context(&self) -> Option<&glib::MainContext> {
        None
    }

    fn start(&self) {}
    fn stop(&self) {}

    fn refresh_video_renderer(&self) {}

    fn enter_fullscreen(&self) {}
    fn leave_fullscreen(&self) {}

    fn dialog_result(&self, _relative_uri: Option<glib::GString>) -> Option<glib::GString> {
        None
    }

    fn set_video_renderer(&self, _renderer: &video_renderer::VideoRenderer) {}

    fn volume_changed(&self, _volume: f64) {}

    fn set_position_range_value(&self, _position: u64) {}
    fn set_position_range_end(&self, _end: f64) {}

    fn resize_window(&self, _width: i32, _height: i32) {}

    fn set_window_title(&self, _title: &str) {}

    fn playback_state_changed(&self, _playback_state: &PlaybackState) {}
    fn update_subtitle_track_menu(&self, _section: &gio::Menu) {}
    fn update_audio_track_menu(&self, _section: &gio::Menu) {}
    fn update_video_track_menu(&self, _section: &gio::Menu) {}

    fn clear_audio_visualization_menu(&self) {}
    fn update_audio_visualization_menu(&self, _section: &gio::Menu) {}
    fn mutable_audio_visualization_menu(&self) -> bool {
        false
    }
}
