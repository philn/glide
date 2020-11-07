use glib::subclass;
use glib::subclass::prelude::*;
use gst::prelude::*;
use gst::subclass::prelude::*;

lazy_static! {
    static ref CAT: gst::DebugCategory =
        gst::DebugCategory::new("osd", gst::DebugColorFlags::empty(), Some("On-screen display"),);
}

struct Osd {
    overlay: gst::Element,
    src: gst::Element,
    textsrc: gst_app::AppSrc,
    srcpad: gst::GhostPad,
    sinkpad: gst::GhostPad,
}

impl Osd {}

impl ObjectSubclass for Osd {
    const NAME: &'static str = "Osd";
    type ParentType = gst::Bin;
    type Instance = gst::subclass::ElementInstanceStruct<Self>;
    type Class = subclass::simple::ClassStruct<Self>;

    glib_object_subclass!();

    fn with_class(klass: &subclass::simple::ClassStruct<Self>) -> Self {
        let template = klass.get_pad_template("sink").unwrap();
        let sinkpad = gst::GhostPad::from_template(&template, Some("sink"));
        let template = klass.get_pad_template("src").unwrap();
        let srcpad = gst::GhostPad::from_template(&template, Some("src"));
        let overlay = gst::ElementFactory::make("textoverlay", Some("overlay")).unwrap();
        overlay.set_property("wait-text", &false).unwrap();
        overlay.set_property_from_str("halignment", "right");
        overlay.set_property_from_str("valignment", "top");
        // overlay.set_property_from_str("valignment", "absolute");
        // overlay.set_property("y-absolute", &0.0.to_value()).unwrap();
        let src = gst::ElementFactory::make("appsrc", None).unwrap();
        let textsrc = src
            .clone()
            .dynamic_cast::<gst_app::AppSrc>()
            .expect("textsrc should be an appsrc");

        Self {
            overlay,
            src,
            textsrc,
            srcpad,
            sinkpad,
        }
    }

    fn class_init(klass: &mut subclass::simple::ClassStruct<Self>) {
        klass.set_metadata(
            "On-screen display",
            "Filter/Effect/Video",
            "Displays messages on-screen",
            "Philippe Normand <phil@base-art.net>",
        );

        let caps = gst::Caps::new_simple("video/x-raw", &[]);
        let src_pad_template =
            gst::PadTemplate::new("src", gst::PadDirection::Src, gst::PadPresence::Always, &caps).unwrap();
        klass.add_pad_template(src_pad_template);

        let sink_pad_template =
            gst::PadTemplate::new("sink", gst::PadDirection::Sink, gst::PadPresence::Always, &caps).unwrap();
        klass.add_pad_template(sink_pad_template);
    }
}

impl ObjectImpl for Osd {
    glib_object_impl!();

    fn constructed(&self, obj: &glib::Object) {
        self.parent_constructed(obj);

        let bin = obj.downcast_ref::<gst::Bin>().unwrap();
        bin.add(&self.overlay).unwrap();
        bin.add(&self.src).unwrap();

        let caps = gst::Caps::new_simple("text/x-raw", &[("format", &"pango-markup")]);
        self.textsrc.set_caps(Some(&caps));
        self.textsrc.set_property_format(gst::Format::Time);

        let srcpad = self.textsrc.get_static_pad("src").unwrap();
        let sinkpad = self.overlay.get_static_pad("text_sink").unwrap();
        srcpad.link(&sinkpad).unwrap();

        self.sinkpad
            .set_target(Some(&self.overlay.get_static_pad("video_sink").unwrap()))
            .unwrap();
        self.srcpad
            .set_target(Some(&self.overlay.get_static_pad("src").unwrap()))
            .unwrap();

        bin.add_pad(&self.sinkpad).unwrap();
        bin.add_pad(&self.srcpad).unwrap();
    }
}

impl ElementImpl for Osd {
    fn post_message(&self, element: &gst::Element, msg: gst::Message) -> bool {
        use gst::MessageView;

        match msg.view() {
            MessageView::Application(ref msg)
                if msg
                    .get_structure()
                    .map(|s| s.get_name() == "osd-request")
                    .unwrap_or(false) =>
            {
                let s = msg.get_structure().unwrap();
                // if let Ok(text) = s.get::<&str>("text") {
                //     self.overlay.set_property("text", &text).unwrap();
                // }
                if let Ok(buffer) = s.get::<&gst::BufferRef>("buffer") {
                    let mut foo = buffer.unwrap().copy_deep().unwrap();
                    {
                        let buf = foo.get_mut().unwrap();
                        let now = element.query_position::<gst::ClockTime>().unwrap_or_else(|| 0.into());
                        buf.set_pts(now);
                    }
                    self.textsrc.push_buffer(foo).unwrap();
                }
                true
            }
            _ => self.parent_post_message(element, msg),
        }
    }
}

impl BinImpl for Osd {}

pub fn register_osd() -> Result<(), glib::BoolError> {
    gst::Element::register(None, "osd", gst::Rank::None, Osd::get_type())
}
