use std::sync::Mutex;

lazy_static! {
    pub static ref INHIBIT_COOKIE: Mutex<Option<u32>> = { Mutex::new(None) };
    pub static ref INITIAL_POSITION: Mutex<Option<(i32, i32)>> = { Mutex::new(None) };
    pub static ref INITIAL_SIZE: Mutex<Option<(i32, i32)>> = { Mutex::new(None) };
    pub static ref MOUSE_NOTIFY_SIGNAL_ID: Mutex<Option<u64>> = { Mutex::new(None) };
}
