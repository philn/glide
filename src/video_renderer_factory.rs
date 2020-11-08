extern crate failure;

#[cfg(target_os = "macos")]
use crate::cocoa_video_renderer;
use crate::errors;
#[cfg(target_os = "linux")]
use crate::gtk_video_renderer;
use crate::video_renderer::VideoRenderer;
use std::boxed::Box;

pub fn video_renderer_make() -> Result<Box<VideoRenderer>, errors::GlideError> {
    #[cfg(target_os = "linux")]
    return Ok(Box::new(gtk_video_renderer::GtkVideoRenderer::new()));

    #[cfg(target_os = "macos")]
    return Ok(Box::new(cocoa_video_renderer::CocoaVideoRenderer::new()));

    #[cfg(not(unix))]
    Err(errors::GlideError::MissingVideoRenderer)
}
