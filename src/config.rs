pub const GETTEXT_PACKAGE: Option<&str> = option_env!("GETTEXT_PACKAGE");
pub const LOCALEDIR: Option<&str> = option_env!("LOCALEDIR");

pub fn gettext_package() -> &'static str {
    GETTEXT_PACKAGE.unwrap_or("glide")
}

pub fn localedir() -> Option<&'static str> {
    LOCALEDIR
}

pub fn app_id() -> &'static str {
    if cfg!(feature = "devel") {
        "net.base_art.Glide.Devel"
    } else {
        "net.base_art.Glide"
    }
}
