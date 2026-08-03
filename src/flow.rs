// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors
//
// Ported from burningtnt/Terracotta (https://github.com/burningtnt/Terracotta).
// Original copyright (c) burningtnt.
// Licensed under AGPL-3.0-or-later. See THIRD_PARTY_LICENSE.

use crate::context::{ContextConfig, Emitter};
use crate::easytier::argument::{Argument, PortForward, Proto};
use crate::easytier::publics::fetch_public_nodes;
use crate::easytier::{self, EasyTierMember};
use crate::event::Event;
use crate::mc::fakeserver::FakeServer;
use crate::mc::scanner::MinecraftScanner;
use crate::ports::PortRequest;
use crate::profile::{ProfileKind, ProfileSnapshot};
use crate::room::Room;
use crate::scaffolding::client::ClientSession;
use crate::scaffolding::PacketResponse;
use crate::session::{ConnectionDifficulty, SessionError, SessionStateId};
use crate::{MOTD, VENDOR};
use serde_json::{json, Value};
use socket2::{Domain, SockAddr, Socket, Type};
use std::borrow::Cow;
use std::mem::{transmute, MaybeUninit};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::str::FromStr;
use std::thread;
use std::time::{Duration, SystemTime};

pub fn run_host(
    emitter: Emitter,
    token: u64,
    room: Room,
    player_name: Option<String>,
    requested_nodes: Vec<String>,
    config: ContextConfig,
) {
    let public_nodes = fetch_public_nodes(&room, {
        let mut nodes = requested_nodes;
        nodes.extend(config.public_nodes);
        nodes
    });

    let host_motd = config.motd.unwrap_or_else(|| MOTD.to_string());
    let scanner = MinecraftScanner::create(move |motd| motd != host_motd);

    let port = 'scan: loop {
        thread::sleep(Duration::from_millis(200));

        if let Some(port) = scanner.get_ports().first().copied() {
            break 'scan port;
        }

        if emitter.is_shutdown() || !emitter.can_capture(token) {
            return;
        }
    };

    let token = match emitter.transition(token, |s| {
        s.port = Some(port);
        SessionStateId::HostStarting
    }) {
        Some(token) => token,
        None => return,
    };

    let scaffolding = emitter.scaffolding_port();

    let mut args = compute_arguments(&room, &public_nodes);
    args.push(Argument::HostName(Cow::Owned(format!(
        "scaffolding-mc-server-{}",
        scaffolding
    ))));
    args.push(Argument::IPv4(Ipv4Addr::new(10, 144, 144, 1)));
    args.push(Argument::TcpWhitelist(scaffolding));
    args.push(Argument::TcpWhitelist(port));
    args.push(Argument::UdpWhitelist(port));

    let easytier = easytier::create(&config.data_dir, args);

    let machine_id = crate::machine_id::get_or_create(&config.data_dir.join("machine-id"))
        .unwrap_or_else(|| "00000000000000000000000000000000".to_string());
    let host_profile = ProfileSnapshot {
        machine_id,
        name: player_name.unwrap_or_else(|| "Terracotta Anonymous Host".to_string()),
        vendor: VENDOR.to_string(),
        kind: ProfileKind::Host,
    }
    .into_profile();

    let token = match emitter.transition(token, |s| {
        s.easytier = Some(easytier);
        s.port = Some(port);
        s.profiles = vec![(SystemTime::now(), host_profile)];
        SessionStateId::HostOk
    }) {
        Some(token) => token,
        None => return,
    };

    emitter.emit(Event::HostReady {
        room_code: room.code.clone(),
        port,
    });

    let monitor = emitter.clone();
    thread::spawn(move || {
        let mut counter = 0;
        loop {
            thread::sleep(Duration::from_secs(5));

            if check_mc_conn(port) {
                counter = 0;
            } else {
                counter += 1;
                if counter >= 3 {
                    let _ = monitor.set_exception(token, SessionError::PingServerRst);
                    return;
                }
            }

            let (alive, changed, left) = {
                let mut s = monitor.session().lock();
                if !s.can_capture(token) {
                    return;
                }

                let alive = s.easytier.as_ref().map(|e| e.is_alive()).unwrap_or(false);
                if !alive {
                    s.state = SessionStateId::Exception;
                    s.error = Some(SessionError::HostEasytierCrash);
                    s.commit();
                    monitor.emit(Event::StateChanged {
                        state: s.state,
                        version: s.version(),
                    });
                    return;
                }

                let mut changed = false;
                let mut left = Vec::new();
                let now = SystemTime::now();
                for i in (1..s.profiles.len()).rev() {
                    let (time, profile) = &s.profiles[i];
                    if now
                        .duration_since(*time)
                        .is_ok_and(|d| d >= Duration::from_secs(10))
                    {
                        left.push(profile.get_machine_id().to_string());
                        s.profiles.remove(i);
                        changed = true;
                    }
                }
                (alive, changed, left)
            };
            let _ = alive;

            if changed {
                monitor.commit_shared(|_| {});                for machine_id in left {
                    monitor.emit(Event::PlayerLeft { machine_id });
                }
            }
        }
    });
}

pub fn run_guest(
    emitter: Emitter,
    token: u64,
    room: Room,
    player_name: Option<String>,
    requested_nodes: Vec<String>,
    config: ContextConfig,
) {
    let public_nodes = fetch_public_nodes(&room, {
        let mut nodes = requested_nodes;
        nodes.extend(config.public_nodes);
        nodes
    });

    let mut args = compute_arguments(&room, &public_nodes);
    args.push(Argument::Dhcp);
    args.push(Argument::TcpWhitelist(0));
    args.push(Argument::UdpWhitelist(0));

    let easytier = easytier::create(&config.data_dir, args);

    let token = match emitter.transition(token, |s| {
        s.easytier = Some(easytier);
        s.difficulty = Some(ConnectionDifficulty::Unknown);
        SessionStateId::GuestStarting
    }) {
        Some(token) => token,
        None => return,
    };

    let (scaffolding_port, host_ip) = 'local_port: {
        for _ in 0..5 {
            thread::sleep(Duration::from_secs(3));

            let players = {
                let mut s = emitter.session().lock();
                if !s.can_capture(token) {
                    return;
                }
                let Some(easytier) = s.easytier.as_mut() else {
                    return;
                };
                if !easytier.is_alive() {
                    let _ = emitter.set_exception(token, SessionError::GuestEasytierCrash);
                    return;
                }
                easytier.get_players()
            };

            let Some(players) = players else {
                continue;
            };

            let Some(local_nat) = players.iter().find_map(
                |EasyTierMember { is_local, nat, .. }| {
                    if *is_local {
                        Some(nat)
                    } else {
                        None
                    }
                },
            ) else {
                continue;
            };

            let Some((server_address, server_port, server_nat)) = players.iter().find_map(
                |EasyTierMember {
                     hostname,
                     address,
                     is_local,
                     nat,
                     ..
                 }| {
                    static PREFIX: &str = "scaffolding-mc-server-";

                    if let Some(address) = address
                        && !is_local
                        && hostname.starts_with(PREFIX)
                        && let Ok(port) = u16::from_str(&hostname[PREFIX.len()..])
                    {
                        Some((address, port, nat))
                    } else {
                        None
                    }
                },
            ) else {
                continue;
            };

            logging!(
                "RoomExperiment",
                "Scaffolding Server is at {}:{}",
                server_address,
                server_port
            );
            let local_port = PortRequest::Scaffolding.request();
            let forwarded = {
                let mut s = emitter.session().lock();
                let Some(easytier) = s.easytier.as_mut() else {
                    return;
                };
                easytier.add_port_forward(&[PortForward {
                    local: SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, local_port).into(),
                    remote: SocketAddrV4::new(*server_address, server_port).into(),
                    proto: Proto::Tcp,
                }])
            };
            if !forwarded {
                logging!(
                    "RoomExperiment",
                    "Cannot create a port-forward {} -> {} for Scaffolding Connection.",
                    local_port,
                    server_port
                );
                let _ = emitter.set_exception(token, SessionError::GuestEasytierCrash);
                return;
            }

            let difficulty = easytier::calc_conn_difficulty(local_nat, server_nat);
            let _ = emitter.mutate_shared(token, |s| {
                s.difficulty = Some(difficulty);
            });
            logging!(
                "RoomExperiment",
                "Current NAT status: {:?} -> {:?}, difficulty = {:?}",
                local_nat,
                server_nat,
                difficulty
            );
            emitter.emit(Event::ConnectionDifficulty { difficulty });

            break 'local_port (local_port, *server_address);
        }

        logging!("RoomExperiment", "Cannot find scaffolding server.");
        let _ = emitter.set_exception(token, SessionError::PingHostFail);
        return;
    };

    fn fail(emitter: &Emitter, token: u64) {
        let _ = emitter.set_exception(token, SessionError::PingHostFail);
    }

    let mut session = 'session: {
        for _ in 0..60 {
            thread::sleep(Duration::from_secs(4));

            const FINGERPRINT: [u8; 16] = [
                0x41, 0x57, 0x48, 0x44, 0x86, 0x37, 0x40, 0x59, 0x57, 0x44, 0x92, 0x43, 0x96,
                0x99, 0x85, 0x01,
            ];
            if let Ok(mut session) =
                ClientSession::open(IpAddr::V4(Ipv4Addr::LOCALHOST), scaffolding_port)
                && let Some(response) = session.send_sync(("c", "ping"), |body| {
                    body.extend_from_slice(&FINGERPRINT);
                })
            {
                let PacketResponse::Ok { data } = response else {
                    unreachable!();
                };

                if data.len() == 16 && data == FINGERPRINT {
                    logging!("RoomExperiment", "Scaffolding Server has been verified.");
                    break 'session session;
                }
            }

            let alive = {
                let s = emitter.session().lock();
                s.easytier.as_ref().map(|e| e.is_alive()).unwrap_or(false)
            };
            if !alive {
                let _ = emitter.set_exception(token, SessionError::GuestEasytierCrash);
                return;
            }
        }

        logging!("RoomExperiment", "Cannot connect to scaffolding server.");
        fail(&emitter, token);
        return;
    };

    let Some(response) = session.send_sync(("c", "server_port"), |_| {}) else {
        fail(&emitter, token);
        return;
    };

    let port = if let PacketResponse::Ok { data } = response
        && data.len() == 2
    {
        let mut p = [0u8; 2];
        p.copy_from_slice(data.as_slice());
        u16::from_be_bytes(p)
    } else {
        fail(&emitter, token);
        return;
    };
    logging!("RoomExperiment", "MC server is at {}", port);

    let local_port = {
        let mut s = emitter.session().lock();
        let Some(easytier) = s.easytier.as_mut() else {
            return;
        };

        // To maximum compatibility, try to request the identical port.
        // If failed, use a dynamic free port instead.
        let local_port = PortRequest::request_specific(port).unwrap_or_else(|e| {
            logging!("RoomExperiment", "Unable to request shadow port {} on client: {:?}. Mods requiring UDP socket like SimpleVoiceChat may go wrong.", port, e);
            PortRequest::Minecraft.request()
        });

        let locals = [
            SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, local_port).into(),
            SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, local_port, 0, 0).into(),
        ];
        let protos = [Proto::Tcp, Proto::Udp];

        const SIZE: usize = 4;
        assert_eq!(locals.len() * protos.len(), SIZE);
        let mut forwards: [MaybeUninit<PortForward>; SIZE] =
            [const { MaybeUninit::uninit() }; _];
        for (i, local) in locals.into_iter().enumerate() {
            for (j, proto) in protos.iter().enumerate() {
                forwards[i * 2 + j].write(PortForward {
                    remote: SocketAddrV4::new(host_ip, port).into(),
                    local,
                    proto: proto.clone(),
                });
            }
        }
        // SAFETY: These two types are of the same size and all elements have been properly initialized.
        let forwards = unsafe {
            transmute::<[MaybeUninit<PortForward>; SIZE], [PortForward; SIZE]>(forwards)
        };

        if !easytier.add_port_forward(&forwards) {
            logging!(
                "RoomExperiment",
                "Cannot create a port-forward {} -> {} for MC Connection.",
                local_port,
                port
            );
            let _ = emitter.set_exception(token, SessionError::GuestEasytierCrash);
            return;
        }

        local_port
    };

    for _ in 0..8 {
        if check_mc_conn(local_port) {
            break;
        }
    }
    logging!("RoomExperiment", "MC connection is OK.");

    let machine_id = crate::machine_id::get_or_create(&config.data_dir.join("machine-id"))
        .unwrap_or_else(|| "00000000000000000000000000000000".to_string());
    let local_profile = ProfileSnapshot {
        machine_id,
        name: player_name.unwrap_or_else(|| "Terracotta Anonymous Guest".to_string()),
        vendor: VENDOR.to_string(),
        kind: ProfileKind::Local,
    }
    .into_profile();

    let guest_motd = config.motd.unwrap_or_else(|| MOTD.to_string());
    let token = match emitter.transition(token, |s| {
        s.server = Some(FakeServer::create(local_port, guest_motd.clone()));
        s.port = Some(local_port);
        s.profiles = vec![(SystemTime::now(), local_profile.clone())];
        SessionStateId::GuestOk
    }) {
        Some(token) => token,
        None => return,
    };

    emitter.emit(Event::GuestReady {
        url: if local_port == 25565 {
            "127.0.0.1".into()
        } else {
            format!("127.0.0.1:{}", local_port)
        },
    });

    let monitor = emitter.clone();
    let local_machine_id = local_profile.get_machine_id().to_string();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(5));

        {
            let Some(_) = session.send_sync(("c", "player_ping"), |body| {
                serde_json::to_writer(
                    body,
                    &json!({
                        "machine_id": local_profile.get_machine_id(),
                        "name": local_profile.get_name(),
                        "vendor": local_profile.get_vendor()
                    }),
                )
                .unwrap();
            }) else {
                fail(&monitor, token);
                return;
            };
        }

        {
            let Some(server_profiles) = session
                .send_sync(("c", "player_profiles_list"), |_| {})
                .map(|response| {
                    let PacketResponse::Ok { data } = response else {
                        unreachable!();
                    };
                    data
                })
                .and_then(|data| {
                    let mut host = false;
                    let mut local = false;

                    let mut server_players: Vec<crate::profile::Profile> = vec![];
                    for item in
                        serde_json::from_slice::<Value>(data.as_slice()).ok()?.as_array()?
                    {
                        let name = item.as_object()?.get("name")?.as_str()?;
                        let machine_id = item.as_object()?.get("machine_id")?.as_str()?;
                        let vendor = item.as_object()?.get("vendor")?.as_str()?;

                        let kind = if machine_id == local_machine_id {
                            if local {
                                logging!("RoomExperiment", "API c:player_profiles_list invocation failed: Multiple local player, machine_id may have conflicted.");
                                return None;
                            }
                            local = true;

                            ProfileKind::Local
                        } else {
                            match item.as_object()?.get("kind")?.as_str()? {
                                "HOST" if !host => {
                                    host = true;
                                    ProfileKind::Host
                                }
                                "GUEST" => ProfileKind::Guest,
                                _ => return None,
                            }
                        };

                        server_players.push(
                            ProfileSnapshot {
                                machine_id: machine_id.to_string(),
                                name: name.to_string(),
                                vendor: vendor.to_string(),
                                kind,
                            }
                            .into_profile(),
                        )
                    }
                    if !host {
                        logging!("RoomExperiment", "API c:player_profiles_list invocation failed: No host detected.");
                        return None;
                    }
                    if !local {
                        server_players.push(local_profile.clone());
                    }

                    server_players
                        .sort_by_cached_key(|profile| profile.get_machine_id().to_string());
                    for profile in server_players.windows(2) {
                        if profile[0].get_machine_id() == profile[1].get_machine_id() {
                            logging!("RoomExperiment", "API c:player_profiles_list invocation failed: machine_id conflict.");
                            return None;
                        }
                    }
                    Some(server_players)
                })
            else {
                fail(&monitor, token);
                return;
            };

            enum Action {
                None,
                Shared {
                    joined: Vec<crate::profile::Profile>,
                    left: Vec<String>,
                },
                Exception(SessionError),
            }

            let action = 'action: {
                let mut s = monitor.session().lock();
                if !s.can_capture(token) {
                    return;
                }
                let alive = s.easytier.as_ref().map(|e| e.is_alive()).unwrap_or(false);
                if !alive {
                    break 'action Action::Exception(SessionError::GuestEasytierCrash);
                }
                let mut used = vec![false; server_profiles.len()];
                let mut changed = false;
                let mut joined = Vec::new();
                let mut left = Vec::new();
                for i in (0..s.profiles.len()).rev() {
                    let (_, profile) = &mut s.profiles[i];
                    match profile.get_kind() {
                        ProfileKind::Host => {
                            match server_profiles
                                .binary_search_by_key(&profile.get_machine_id(), |p| p.get_machine_id())
                            {
                                Ok(index)
                                    if !used[index]
                                        && server_profiles[index].get_kind()
                                            == ProfileKind::Host =>
                                {
                                    used[index] = true;
                                    if profile.get_name() != server_profiles[index].get_name() {
                                        profile.set_name(
                                            server_profiles[index].get_name().to_string(),
                                        );
                                        changed = true;
                                    }
                                }
                                _ => {
                                    break 'action Action::Exception(
                                        SessionError::ScaffoldingInvalidResponse,
                                    );
                                }
                            }
                        }
                        ProfileKind::Local => {}
                        ProfileKind::Guest => {
                            match server_profiles
                                .binary_search_by_key(&profile.get_machine_id(), |p| p.get_machine_id())
                            {
                                Ok(index)
                                    if used[index]
                                        && server_profiles[index].get_kind()
                                            == ProfileKind::Guest =>
                                {
                                    left.push(profile.get_machine_id().to_string());
                                    s.profiles.remove(i);
                                    changed = true;
                                }
                                Ok(index)
                                    if server_profiles[index].get_kind()
                                        == ProfileKind::Guest =>
                                {
                                    used[index] = true;
                                    if profile.get_name() != server_profiles[index].get_name() {
                                        profile.set_name(
                                            server_profiles[index].get_name().to_string(),
                                        );
                                        changed = true;
                                    }
                                }
                                Ok(_) => {
                                    break 'action Action::Exception(
                                        SessionError::ScaffoldingInvalidResponse,
                                    );
                                }
                                Err(_) => {
                                    left.push(profile.get_machine_id().to_string());
                                    s.profiles.remove(i);
                                    changed = true;
                                }
                            }
                        }
                    }
                }

                let mut server_profiles = server_profiles;
                for i in (0..server_profiles.len()).rev() {
                    let profile = server_profiles.pop().unwrap();
                    if !used[i] && profile.get_kind() != ProfileKind::Local {
                        s.profiles.push((SystemTime::now(), profile.clone()));
                        joined.push(profile);
                        changed = true;
                    }
                }

                if changed {
                    Action::Shared { joined, left }
                } else {
                    Action::None
                }
            };

            match action {
                Action::None => {}
                Action::Shared { joined, left } => {
                    monitor.commit_shared(|_| {});
                    for profile in joined {
                        monitor.emit(Event::PlayerJoined { profile });
                    }
                    for machine_id in left {
                        monitor.emit(Event::PlayerLeft { machine_id });
                    }
                }
                Action::Exception(error) => {
                    let _ = monitor.set_exception(token, error);
                    return;
                }
            }
        }
    });
}

fn compute_arguments(room: &Room, public_servers: &[String]) -> Vec<Argument> {
    static DEFAULT_ARGUMENTS: [Argument; 8] = [
        Argument::NoTun,
        Argument::Compression(Cow::Borrowed("zstd")),
        Argument::MultiThread,
        Argument::LatencyFirst,
        Argument::EnableKcpProxy,
        Argument::Listener {
            address: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
            proto: Proto::Udp,
        },
        Argument::Listener {
            address: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
            proto: Proto::Tcp,
        },
        Argument::P2POnly,
    ];

    let mut args: Vec<Argument> = Vec::with_capacity(32);
    args.extend_from_slice(&[
        Argument::NetworkName(Cow::Owned(room.network_name.clone())),
        Argument::NetworkSecret(Cow::Owned(room.network_secret.clone())),
    ]);

    for server in public_servers {
        args.push(Argument::PublicServer(Cow::Owned(server.clone())));
    }

    args.extend_from_slice(&DEFAULT_ARGUMENTS);
    args
}

fn check_mc_conn(port: u16) -> bool {
    let start = SystemTime::now();

    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(64))).unwrap();
    socket.set_write_timeout(Some(Duration::from_secs(64))).unwrap();
    if let Ok(_) = socket.connect_timeout(
        &SockAddr::from(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), port)),
        Duration::from_secs(64),
    ) && let Ok(_) = socket.send(&[0xFE])
    {
        let mut buf: [MaybeUninit<u8>; _] = [MaybeUninit::uninit(); 1];

        if let Ok(size) = socket.recv(&mut buf)
            && size >= 1
            // SAFETY: The first byte has been initialized by recv, as size >= 1
            && unsafe { buf[0].assume_init() } == 0xFF
        {
            return true;
        }
    }

    thread::sleep(
        (start + Duration::from_secs(5))
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO),
    );
    false
}
