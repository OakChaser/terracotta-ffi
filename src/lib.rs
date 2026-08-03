// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors
//
// Ported from burningtnt/Terracotta (https://github.com/burningtnt/Terracotta).
// Original copyright (c) burningtnt.
// Licensed under AGPL-3.0-or-later. See THIRD_PARTY_LICENSE.

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

pub const MOTD: &str = "§w§lConic Connect";

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
