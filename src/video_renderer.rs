extern crate gstreamer_player as gst_player;
extern crate gstreamer_video as gst_video;

pub trait VideoRenderer {
    fn gst_video_renderer(&self) -> Option<&gst_player::PlayerVideoRenderer> {
        None
    }

    fn set_player(&self, player: &gst_player::Player) {}

    fn as_gtk_widget(&self) -> Option<&gtk::Widget> {
        None
    }
}
