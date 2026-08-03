// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors
//
// Ported from burningtnt/Terracotta (https://github.com/burningtnt/Terracotta).
// Original copyright (c) burningtnt.
// Licensed under AGPL-3.0-or-later. See THIRD_PARTY_LICENSE.

use std::borrow::Cow;
use std::net::{Ipv4Addr, SocketAddr};

type CowString = Cow<'static, str>;

#[derive(Clone, Debug)]
pub struct PortForward {
    pub(crate) local: SocketAddr,
    pub(crate) remote: SocketAddr,
    pub(crate) proto: Proto,
}

#[derive(Clone, Debug)]
pub enum Proto {
    Tcp,
    Udp,
}

impl Proto {
    pub fn name(&self) -> &'static str {
        match self {
            Proto::Tcp => "tcp",
            Proto::Udp => "udp",
        }
    }
}

#[derive(Clone)]
pub enum Argument {
    NoTun,
    Compression(CowString),
    MultiThread,
    LatencyFirst,
    EnableKcpProxy,
    NetworkName(CowString),
    NetworkSecret(CowString),
    PublicServer(CowString),
    Listener { address: SocketAddr, proto: Proto },
    PortForward(PortForward),
    Dhcp,
    HostName(CowString),
    IPv4(Ipv4Addr),
    TcpWhitelist(u16),
    UdpWhitelist(u16),
    P2POnly,
}
