// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors

use crate::profile::Profile;
use crate::session::{Session, SessionError, SessionStateId};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::SystemTime;

struct Profiles<'a>(&'a [(SystemTime, Profile)]);

impl Serialize for Profiles<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for (_, profile) in self.0 {
            sequence.serialize_element(profile)?;
        }
        sequence.end()
    }
}

pub struct Snapshot {
    pub state: SessionStateId,
    pub version: u64,
    pub room_code: String,
    pub detail: Value,
}

impl Snapshot {
    pub fn from(session: &Session) -> Snapshot {
        let room_code = session
            .room
            .as_ref()
            .map(|room| room.code.clone())
            .unwrap_or_default();

        let detail = match session.state {
            SessionStateId::HostOk => json!({
                "port": session.port,
                "profiles": Profiles(&session.profiles),
            }),
            SessionStateId::GuestOk => json!({
                "url": guest_url(session.port),
                "profiles": Profiles(&session.profiles),
            }),
            SessionStateId::Exception => json!({
                "error": {
                    "code": session.error.map(|e| e as u32).unwrap_or(0),
                    "message": session.error.map(error_message).unwrap_or_default(),
                }
            }),
            _ => json!({}),
        };

        Snapshot {
            state: session.state,
            version: session.version(),
            room_code,
            detail,
        }
    }
}

fn guest_url(port: Option<u16>) -> String {
    match port {
        Some(25565) | None => "127.0.0.1".to_string(),
        Some(port) => format!("127.0.0.1:{}", port),
    }
}

fn error_message(error: SessionError) -> &'static str {
    match error {
        SessionError::PingHostFail => "cannot reach the host",
        SessionError::PingHostRst => "host reset the connection",
        SessionError::GuestEasytierCrash => "guest EasyTier process exited",
        SessionError::HostEasytierCrash => "host EasyTier process exited",
        SessionError::PingServerRst => "Minecraft server connection lost",
        SessionError::ScaffoldingInvalidResponse => "invalid scaffolding response",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_snapshot() {
        let session = Session::waiting();
        let snapshot = Snapshot::from(&session);
        assert_eq!(snapshot.state, SessionStateId::Waiting);
        assert!(snapshot.room_code.is_empty());
    }

    #[test]
    fn guest_ok_url() {
        assert_eq!(guest_url(Some(25565)), "127.0.0.1");
        assert_eq!(guest_url(Some(54321)), "127.0.0.1:54321");
    }
}
