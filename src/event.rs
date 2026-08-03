// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors

use crate::profile::Profile;
use crate::session::{ConnectionDifficulty, SessionStateId};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    StateChanged {
        state: SessionStateId,
        version: u64,
    },
    PlayerJoined {
        profile: Profile,
    },
    PlayerLeft {
        machine_id: String,
    },
    ConnectionDifficulty {
        difficulty: ConnectionDifficulty,
    },
    HostReady {
        room_code: String,
        port: u16,
    },
    GuestReady {
        url: String,
    },
    Error {
        code: u32,
        message: String,
    },
}

impl Event {
    pub fn payload_json(&self) -> Value {
        match self {
            Event::StateChanged { state, version } => {
                serde_json::json!({ "state": *state as u8, "version": version })
            }
            Event::PlayerJoined { profile } => {
                let profile = serde_json::to_value(profile).unwrap_or_default();
                serde_json::json!({ "profile": profile })
            }
            Event::PlayerLeft { machine_id } => {
                serde_json::json!({ "machine_id": machine_id })
            }
            Event::ConnectionDifficulty { difficulty } => {
                serde_json::json!({ "difficulty": *difficulty as u8 })
            }
            Event::HostReady { room_code, port } => {
                serde_json::json!({ "room": room_code, "port": port })
            }
            Event::GuestReady { url } => {
                serde_json::json!({ "url": url })
            }
            Event::Error { code, message } => {
                serde_json::json!({ "code": code, "message": message })
            }
        }
    }

    pub fn type_id(&self) -> u8 {
        match self {
            Event::StateChanged { .. } => 1,
            Event::PlayerJoined { .. } => 2,
            Event::PlayerLeft { .. } => 3,
            Event::ConnectionDifficulty { .. } => 4,
            Event::HostReady { .. } => 5,
            Event::GuestReady { .. } => 6,
            Event::Error { .. } => 7,
        }
    }
}
