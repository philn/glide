// Only possible in nightly
// static SEEK_BACKWARD_OFFSET: gst::ClockTime = gst::ClockTime::from_mseconds(2000);
// static SEEK_FORWARD_OFFSET: gst::ClockTime = gst::ClockTime::from_mseconds(5000);

pub static SEEK_BACKWARD_OFFSET: gst::ClockTime = gst::ClockTime(Some(2_000_000_000));
pub static SEEK_FORWARD_OFFSET: gst::ClockTime = gst::ClockTime(Some(5_000_000_000));

pub static SUB_FILE_EXTENSIONS: [&str; 3] = ["srt", "sub", "ass"];
