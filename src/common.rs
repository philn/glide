use std::sync::Mutex;

pub enum SeekDirection {
    Backward,
    Forward,
}

lazy_static! {
    pub static ref INHIBIT_COOKIE: Mutex<Option<u32>> = { Mutex::new(None) };
    pub static ref INITIAL_POSITION: Mutex<Option<(i32, i32)>> = { Mutex::new(None) };
    pub static ref INITIAL_SIZE: Mutex<Option<(i32, i32)>> = { Mutex::new(None) };
}
