// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors
//
// Ported from burningtnt/Terracotta (https://github.com/burningtnt/Terracotta).
// Original copyright (c) burningtnt.
// Licensed under AGPL-3.0-or-later. See THIRD_PARTY_LICENSE.

use std::io;
use std::time::Duration;

pub mod client;
pub mod protocols;
pub mod server;

pub(crate) static TIMEOUT: Duration = Duration::from_secs(64);

pub enum PacketResponse {
    Ok { data: Vec<u8> },
    Fail { status: u8, data: Vec<u8> },
}

impl PacketResponse {
    pub fn ok(data: Vec<u8>) -> io::Result<PacketResponse> {
        Ok(PacketResponse::Ok { data })
    }

    pub fn fail(status: u8, data: Vec<u8>) -> io::Result<PacketResponse> {
        Ok(PacketResponse::Fail { status, data })
    }
}
