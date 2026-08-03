// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors
//
// Ported from burningtnt/Terracotta (https://github.com/burningtnt/Terracotta).
// Original copyright (c) burningtnt.
// Licensed under AGPL-3.0-or-later. See THIRD_PARTY_LICENSE.

use crate::context::Emitter;
use crate::event::Event;
use crate::profile::{Profile, ProfileKind, ProfileSnapshot};
use crate::scaffolding::server::Handlers;
use crate::scaffolding::PacketResponse;
use crate::session::SessionStateId;
use serde_json::{json, Value};
use std::io;
use std::sync::Arc;
use std::time::SystemTime;

pub fn build_handlers(emitter: &Emitter) -> Handlers {
    let names: Arc<Vec<(&'static str, &'static str)>> = Arc::new(vec![
        ("c", "ping"),
        ("c", "protocols"),
        ("c", "server_port"),
        ("c", "player_ping"),
        ("c", "player_profiles_list"),
    ]);

    let mut handlers: Handlers = Vec::with_capacity(names.len());

    let emitter = emitter.clone();

    // c:ping
    {
        handlers.push((
            "c",
            "ping",
            Box::new(|request, mut response| {
                response.extend_from_slice(request);
                PacketResponse::ok(response)
            }),
        ));
    }

    // c:protocols
    {
        let names = Arc::clone(&names);
        handlers.push((
            "c",
            "protocols",
            Box::new(move |_, mut response| {
                for (i, (namespace, path)) in names.iter().enumerate() {
                    response.extend_from_slice(namespace.as_bytes());
                    response.push(b':');
                    response.extend_from_slice(path.as_bytes());

                    if i != names.len() - 1 {
                        response.push(b'\0');
                    }
                }
                PacketResponse::ok(response)
            }),
        ));
    }

    // c:server_port
    {
        let emitter = emitter.clone();
        handlers.push((
            "c",
            "server_port",
            Box::new(move |_, response| {
                let port = {
                    let session = emitter.session().lock();
                    if session.state == SessionStateId::HostOk {
                        session.port
                    } else {
                        None
                    }
                };
                match port {
                    Some(port) => {
                        let mut response = response;
                        response.extend_from_slice(&port.to_be_bytes());
                        PacketResponse::ok(response)
                    }
                    None => PacketResponse::fail(32, response),
                }
            }),
        ));
    }

    // c:player_ping
    {
        let emitter = emitter.clone();
        handlers.push((
            "c",
            "player_ping",
            Box::new(move |request, response| {
                let value: Value = serde_json::from_str(&String::from_utf8_lossy(request))?;

                let name = parse(|| value.as_object()?.get("name")?.as_str())?;
                let machine_id = parse(|| value.as_object()?.get("machine_id")?.as_str())?;
                let vendor = parse(|| value.as_object()?.get("vendor")?.as_str())?;

                enum Action {
                    None,
                    Renamed,
                    Joined(Profile),
                    Error(&'static str),
                }

                let action = {
                    let mut session = emitter.session().lock();
                    match session
                        .profiles
                        .iter()
                        .position(|(_, profile)| profile.get_machine_id() == machine_id)
                    {
                        Some(i) if i >= 1 => {
                            session.profiles[i].0 = SystemTime::now();

                            if session.profiles[i].1.get_name() != name {
                                session.profiles[i].1.set_name(name.to_string());
                                Action::Renamed
                            } else {
                                Action::None
                            }
                        }
                        Some(_) => Action::Error(
                            "IllegalStateException: Cannot modify host, machine_id may conflict.",
                        ),
                        None => {
                            let profile = ProfileSnapshot {
                                machine_id: machine_id.to_string(),
                                name: name.to_string(),
                                vendor: vendor.to_string(),
                                kind: ProfileKind::Guest,
                            }
                            .into_profile();
                            session
                                .profiles
                                .push((SystemTime::now(), profile.clone()));
                            Action::Joined(profile)
                        }
                    }
                };

                match action {
                    Action::None => PacketResponse::ok(response),
                    Action::Renamed => {
                        emitter.commit_shared(|_| {});
                        PacketResponse::ok(response)
                    }
                    Action::Joined(profile) => {
                        emitter.commit_shared(|_| {});
                        emitter.emit(Event::PlayerJoined { profile });
                        PacketResponse::ok(response)
                    }
                    Action::Error(message) => {
                        Err(io::Error::other(message))
                    }
                }
            }),
        ));
    }

    // c:player_profiles_list
    {
        let emitter = emitter.clone();
        handlers.push((
            "c",
            "player_profiles_list",
            Box::new(move |_, mut response| {
                let profiles: Vec<Value> = {
                    let session = emitter.session().lock();
                    if session.state != SessionStateId::HostOk {
                        return Err(io::Error::other(
                            "IllegalStateException: Expecting HostOk.",
                        ));
                    }

                    session
                        .profiles
                        .iter()
                        .map(|(_, profile)| {
                            json!({
                                "name": profile.get_name(),
                                "machine_id": profile.get_machine_id(),
                                "vendor": profile.get_vendor(),
                                "kind": match profile.get_kind() {
                                    ProfileKind::Host => "HOST",
                                    ProfileKind::Guest => "GUEST",
                                    ProfileKind::Local => unreachable!(),
                                }
                            })
                        })
                        .collect()
                };

                serde_json::to_writer(&mut response, &profiles)?;
                PacketResponse::ok(response)
            }),
        ));
    }

    handlers
}

fn parse<F, R>(f: F) -> io::Result<R>
where
    F: FnOnce() -> Option<R>,
{
    f().ok_or(io::Error::from(io::ErrorKind::InvalidInput))
}
