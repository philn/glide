#[cfg(target_os = "macos")]
extern crate cocoa;
#[cfg(target_os = "macos")]
extern crate core_foundation;
extern crate crossbeam_channel as channel;
extern crate directories;
extern crate failure;
extern crate gdk;
extern crate gio;
extern crate glib;
extern crate gstreamer as gst;
extern crate gstreamer_player as gst_player;
extern crate gstreamer_video as gst_video;
extern crate gtk;
#[cfg(target_os = "macos")]
extern crate objc;
#[macro_use]
extern crate lazy_static;
#[cfg(feature = "self-updater")]
#[macro_use]
extern crate self_update;
#[macro_use]
extern crate serde_derive;

mod app;
mod channel_player;
mod gtk_app;
mod gtk_video_renderer;
#[cfg(target_os = "macos")]
mod iokit_sleep_disabler;
mod video_player;
mod video_renderer;

fn main() {
    #[cfg(not(unix))]
    {
        println!("Add support for target platform");
        std::process::exit(-1);
    }

    gst::init().expect("Failed to initialize GStreamer.");

    glib::set_application_name("Glide");

    let player = video_player::VideoPlayer::new();
    let args = std::env::args().collect::<Vec<_>>();
    video_player::register_player_and_run(player, &args);

    // unsafe {
    //     gst::deinit();
    // }
}
