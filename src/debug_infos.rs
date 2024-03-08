extern crate serde_json;

use crate::gio::glib::translate::IntoGlib;
use crate::gst::prelude::GstObjectExt;
use anyhow::Error;
use gstreamer::prelude::PluginFeatureExtManual;
use std::str::FromStr;

#[derive(Serialize, Deserialize, Clone)]
enum MediaType {
    Audio,
    Video,
}

#[derive(Serialize, Deserialize, Clone)]
struct Capability {
    name: String,
    gst_element_name: String,
    rank: i32,
    hardware: bool,
}

#[derive(Serialize, Deserialize, Clone)]
struct RuntimeDependency {
    name: String,
    version: String,
}

#[derive(Serialize, Deserialize)]
pub struct DebugInfos {
    dependencies: Vec<RuntimeDependency>,
    playbin3_enabled: bool,
    audio_decoders: Vec<Capability>,
    video_decoders: Vec<Capability>,
    features: Vec<String>,
}

// TODO: graph dumps

#[derive(Clone)]
pub struct Codec {
    name: &'static str,
    caps: &'static str,
}

lazy_static! {
    pub static ref AUDIO_CODECS: Vec<Codec> = vec![
        Codec {
            name: "MP3",
            caps: "mpeg, mpegversion=1, layer=3"
        },
        Codec {
            name: "AAC",
            caps: "mpeg, mpegversion=4"
        },
        Codec {
            name: "AC3",
            caps: "x-ac3"
        },
        Codec {
            name: "Flac",
            caps: "x-flac"
        },
        Codec {
            name: "Opus",
            caps: "x-opus"
        }
    ];
    pub static ref VIDEO_CODECS: Vec<Codec> = vec![
        Codec {
            name: "H.264",
            caps: "x-h264"
        },
        Codec {
            name: "H.265",
            caps: "x-h265"
        },
        Codec {
            name: "AV1",
            caps: "x-av1"
        },
        Codec {
            name: "VP8",
            caps: "x-vp8"
        },
        Codec {
            name: "VP9",
            caps: "x-vp9"
        }
    ];
}

fn fill_capabilities(media_type: &MediaType) -> Vec<Capability> {
    let (factory_type, codecs, prefix) = match media_type {
        MediaType::Audio => (gst::ElementFactoryType::MEDIA_AUDIO, AUDIO_CODECS.clone(), "audio"),
        MediaType::Video => (gst::ElementFactoryType::MEDIA_VIDEO, VIDEO_CODECS.clone(), "video"),
    };
    let decoder_factories =
        gst::ElementFactory::factories_with_type(gst::ElementFactoryType::DECODER | factory_type, gst::Rank::MARGINAL);
    let mut decoders: Vec<Capability> = [].to_vec();
    for codec in codecs {
        let name = format!("{prefix}/{0}", codec.caps);
        let caps = gst::caps::Caps::from_str(&name).unwrap();
        for factory in decoder_factories.iter() {
            if factory.can_sink_any_caps(&caps) {
                let f = factory.create().build().unwrap();
                let rank: i32 = factory.rank().into_glib();
                let hardware = factory
                    .metadata(gst::ELEMENT_METADATA_KLASS)
                    .expect("Missing klass")
                    .split('/')
                    .collect::<Vec<&str>>()
                    .contains(&"Hardware");
                let mut element_name = f.name().to_string();
                element_name.pop();
                if element_name.ends_with('-') {
                    element_name.pop();
                }
                decoders.push(Capability {
                    name: codec.name.to_string(),
                    gst_element_name: element_name,
                    rank,
                    hardware,
                });
            }
        }
    }

    decoders
}

impl DebugInfos {
    pub fn new() -> Self {
        let mut playbin3_enabled = gst::version() >= (1, 24, 0, 0);
        if let Ok(val) = std::env::var("GST_PLAY_USE_PLAYBIN3") {
            playbin3_enabled = val == "1";
        }

        let features: Vec<String> = match option_env!("VERGEN_CARGO_FEATURES") {
            Some(val) => val.split(',').map(|v| v.to_string()).collect::<Vec<_>>(),
            None => [].to_vec(),
        };

        let dependencies: Vec<RuntimeDependency> = vec![
            RuntimeDependency {
                name: "GTK".to_string(),
                version: format!(
                    "{}.{}.{}",
                    gtk::major_version(),
                    gtk::minor_version(),
                    gtk::micro_version()
                ),
            },
            RuntimeDependency {
                name: "GStreamer".to_string(),
                version: gst::version_string().to_string(),
            },
        ];

        Self {
            dependencies,
            playbin3_enabled,
            features,
            audio_decoders: fill_capabilities(&MediaType::Audio),
            video_decoders: fill_capabilities(&MediaType::Video),
        }
    }

    pub fn to_json(&self) -> anyhow::Result<String, Error> {
        Ok(serde_json::to_string_pretty(&self)?)
    }
}
