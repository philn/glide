use crate::config;
use gettextrs::*;

pub fn init() {
    unsafe {
        setlocale(LocaleCategory::LcAll, "");
    }
    if let Some(localedir) = config::localedir() {
        bindtextdomain(config::gettext_package(), localedir).expect("Unable to bind the text domain");
        bind_textdomain_codeset(config::gettext_package(), "UTF-8").expect("Unable to set text domain encoding");
        textdomain(config::gettext_package()).expect("Unable to switch to the text domain");
    }
}
