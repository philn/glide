#[macro_use]
use failure;

#[derive(Debug, Fail)]
pub enum GlideError {
    #[fail(display = "unable to create a video renderer")]
    MissingVideoRenderer,
    #[fail(display = "unable to create a UI window")]
    MissingUI,
    #[cfg(not(unix))]
    #[fail(display = "unsupported platform {}", name)]
    UnsupportedPlatform { name: std::string::String },
}
