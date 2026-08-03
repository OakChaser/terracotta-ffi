#![allow(dead_code)]

#[macro_export]
macro_rules! logging {
    ($prefix:expr, $($arg:tt)*) => {
        $crate::logging::log(&format!("[{}]: {}", $prefix, format_args!($($arg)*)))
    };
}

mod logging;

mod addresses;
mod command;
mod context;
mod easytier;
mod event;
mod ffi;
mod flow;
mod machine_id;
mod mc;
mod ports;
mod profile;
mod room;
mod scaffolding;
mod session;
mod snapshot;

pub const MOTD: &str = "§6§l双击进入陶瓦联机大厅（请保持陶瓦运行）";

pub static VENDOR: &str = concat!(
    "Conic Terracotta ",
    env!("TERRACOTTA_VERSION"),
    ", EasyTier ",
    env!("TERRACOTTA_ET_VERSION")
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    InvalidRoomCode = 1,
    Busy = 2,
    Internal = 3,
    NotImplemented = 4,
}
