// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors
//
// Ported from burningtnt/Terracotta (https://github.com/burningtnt/Terracotta).
// Original copyright (c) burningtnt.
// Licensed under AGPL-3.0-or-later. See THIRD_PARTY_LICENSE.

use crate::easytier::EasyTier;
use crate::mc::fakeserver::FakeServer;
use crate::profile::Profile;
use crate::room::Room;
use serde::Serialize;
use std::net::Ipv4Addr;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[repr(u8)]
pub enum SessionStateId {
    Waiting = 0,
    HostScanning = 1,
    HostStarting = 2,
    HostOk = 3,
    GuestConnecting = 4,
    GuestStarting = 5,
    GuestOk = 6,
    Exception = 7,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[repr(u8)]
pub enum ConnectionDifficulty {
    Unknown = 0,
    Easiest = 1,
    Simple = 2,
    Medium = 3,
    Tough = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[repr(u32)]
pub enum SessionError {
    PingHostFail = 0,
    PingHostRst = 1,
    GuestEasytierCrash = 2,
    HostEasytierCrash = 3,
    PingServerRst = 4,
    ScaffoldingInvalidResponse = 5,
}

pub struct Session {
    pub(crate) state: SessionStateId,
    index: u64,
    sharing: u64,
    pub(crate) room: Option<Room>,
    pub(crate) profiles: Vec<(SystemTime, Profile)>,
    pub(crate) easytier: Option<EasyTier>,
    pub(crate) server: Option<FakeServer>,
    pub(crate) port: Option<u16>,
    pub(crate) host_ip: Option<Ipv4Addr>,
    pub(crate) difficulty: Option<ConnectionDifficulty>,
    pub(crate) error: Option<SessionError>,
}

impl Session {
    pub fn waiting() -> Session {
        Session {
            state: SessionStateId::Waiting,
            index: 0,
            sharing: 0,
            room: None,
            profiles: Vec::new(),
            easytier: None,
            server: None,
            port: None,
            host_ip: None,
            difficulty: None,
            error: None,
        }
    }

    pub fn state(&self) -> SessionStateId {
        self.state
    }

    pub fn version(&self) -> u64 {
        self.index
    }

    pub fn token(&self) -> u64 {
        self.index
    }

    pub fn can_capture(&self, token: u64) -> bool {
        self.index - self.sharing <= token
    }

    pub fn commit(&mut self) -> u64 {
        self.index += 1;
        self.sharing = 0;
        self.index
    }

    pub fn commit_shared(&mut self) -> u64 {
        self.index += 1;
        self.sharing += 1;
        self.index
    }

    pub fn reset_to_waiting(&mut self) -> u64 {
        self.state = SessionStateId::Waiting;
        self.room = None;
        self.profiles.clear();
        self.easytier = None;
        self.server = None;
        self.port = None;
        self.host_ip = None;
        self.difficulty = None;
        self.error = None;
        self.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_token_semantics() {
        let mut s = Session::waiting();
        assert!(s.can_capture(s.token()));

        s.commit();
        assert!(!s.can_capture(s.token() - 1));

        let token = s.token();
        s.commit_shared();
        assert!(s.can_capture(token));

        s.reset_to_waiting();
        assert!(!s.can_capture(token));
    }

    #[test]
    fn reset_keeps_index_monotonic() {
        let mut s = Session::waiting();
        let before = s.version();
        s.reset_to_waiting();
        assert!(s.version() > before);
        assert_eq!(s.state(), SessionStateId::Waiting);
    }
}
