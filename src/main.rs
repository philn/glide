#[cfg(target_os = "macos")]
extern crate core_foundation;
extern crate crossbeam_channel as channel;
extern crate directories;
#[macro_use]
extern crate failure;
#[cfg(target_os = "linux")]
extern crate gdk;
extern crate gio;
extern crate glib;
extern crate gobject_sys;
extern crate gstreamer as gst;
extern crate gstreamer_player as gst_player;
extern crate gstreamer_video as gst_video;
#[cfg(target_os = "linux")]
extern crate gtk;
#[macro_use]
extern crate lazy_static;
#[cfg(feature = "self-updater")]
#[macro_use]
extern crate self_update;
#[macro_use]
extern crate serde_derive;
#[cfg(target_os = "macos")]
extern crate cocoa;
#[cfg(target_os = "macos")]
extern crate fruitbasket;
#[cfg(target_os = "macos")]
#[macro_use]
extern crate objc;

mod app;
mod app_factory;
mod channel_player;
#[cfg(target_os = "macos")]
mod cocoa_app;
#[cfg(target_os = "macos")]
mod cocoa_video_renderer;
mod errors;
#[cfg(target_os = "linux")]
mod gtk_app;
#[cfg(target_os = "linux")]
mod gtk_video_renderer;
#[cfg(target_os = "macos")]
mod iokit_sleep_disabler;
mod video_player;
mod video_renderer;
mod video_renderer_factory;

fn main() -> Result<(), failure::Error> {
    #[cfg(not(unix))]
    {
        return Err(errors::GlideError::UnsupportedPlatform("foo"));
    }

    gst::init()?;

    glib::set_application_name("Glide");

    let glide_app = app_factory::app_make()?;
    let video_renderer = video_renderer_factory::video_renderer_make()?;
    let mut player = video_player::VideoPlayer::new(glide_app, video_renderer);

    let args = std::env::args().collect::<Vec<_>>();
    video_player::register_player_and_run(player, &args);

    // unsafe {
    //     gst::deinit();
    // }
    Ok(())
}
