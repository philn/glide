extern crate gstreamer as gst;
extern crate gstreamer_player as gst_player;
extern crate gstreamer_video as gst_video;

use cocoa::appkit::NSView;
use cocoa::base::{nil, selector, NO, YES};
use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize};
use gst::prelude::*;
use objc::rc::StrongPtr;
use objc::runtime::Object;
use std::fmt;

use crate::glib::translate::ToGlibPtr;
use crate::glib::ObjectExt;
use crate::video_renderer;

pub struct CocoaVideoRenderer {
    gst_renderer: gst_player::PlayerVideoOverlayVideoRenderer,
    video_renderer: gst_player::PlayerVideoRenderer,
    video_window: StrongPtr,
}

impl CocoaVideoRenderer {
    pub fn new() -> Self {
        let sink = gst::ElementFactory::make("caopengllayersink", None).unwrap();
        let gst_renderer = gst_player::PlayerVideoOverlayVideoRenderer::new_with_sink(&sink);
        let video_renderer = gst_renderer.clone().upcast::<gst_player::PlayerVideoRenderer>();

        sink.set_state(gst::State::Ready).unwrap();

        let video_window = unsafe {
            let video_window = NSView::alloc(nil)
                .initWithFrame_(NSRect::new(NSPoint::new(0., 0.), NSSize::new(640., 480.)))
                .autorelease();
            let layer = sink.get_property("layer").unwrap();
            let layer_obj: *const Object = gobject_sys::g_value_get_pointer(layer.to_glib_none().0) as *const Object;

            let _: () = msg_send![video_window, setWantsLayer: YES];
            let _: () = msg_send![video_window, setLayer: layer_obj];
            StrongPtr::new(video_window)
        };

        Self {
            gst_renderer,
            video_renderer,
            video_window,
        }
    }
}

impl video_renderer::VideoRenderer for CocoaVideoRenderer {
    fn gst_video_renderer(&self) -> Option<&gst_player::PlayerVideoRenderer> {
        Some(&self.video_renderer)
    }

    fn set_player(&self, _player: &gst_player::Player) {}

    fn implementation(&self) -> Option<video_renderer::VideoWidgetImpl> {
        Some(video_renderer::VideoWidgetImpl::Cocoa(self.video_window.weak()))
    }

    fn refresh(&self, video_width: i32, video_height: i32) {
        eprintln!("refresh to {}x{}", video_width, video_height);
    }
}

impl fmt::Debug for CocoaVideoRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CocoaVideoRenderer")
    }
}
