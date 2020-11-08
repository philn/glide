extern crate failure;

use crate::app::Application;
#[cfg(target_os = "macos")]
use crate::cocoa_app::GlideCocoaApp;
use crate::errors;
#[cfg(target_os = "linux")]
use crate::gtk_app::GlideGTKApp;

use std::boxed::Box;

pub fn app_make() -> Result<Box<Application>, errors::GlideError> {
    #[cfg(target_os = "linux")]
    return Ok(Box::new(GlideGTKApp::new()));

    #[cfg(target_os = "macos")]
    return Ok(Box::new(GlideCocoaApp::new()));

    #[cfg(not(unix))]
    Err(errors::GlideError::MissingUI)
}
