extern crate gstreamer_player as gst_player;
extern crate gstreamer_video as gst_video;

#[cfg(target_os = "linux")]
use glib::SendWeakRef;
use std::fmt;

#[cfg(target_os = "macos")]
use objc::rc;

pub enum VideoWidgetImpl {
    #[cfg(target_os = "linux")]
    GTK(SendWeakRef<gtk::Widget>),
    #[cfg(target_os = "macos")]
    Cocoa(rc::WeakPtr),
}

pub trait VideoRenderer {
    fn gst_video_renderer(&self) -> Option<&gst_player::PlayerVideoRenderer> {
        None
    }

    fn set_player(&self, _player: &gst_player::Player) {}

    fn implementation(&self) -> Option<VideoWidgetImpl> {
        None
    }

    fn refresh(&self, _video_width: i32, _video_height: i32) {}
}
