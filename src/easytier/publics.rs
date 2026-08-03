// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors
//
// Ported from burningtnt/Terracotta (https://github.com/burningtnt/Terracotta).
// Original copyright (c) burningtnt.
// Licensed under AGPL-3.0-or-later. See THIRD_PARTY_LICENSE.

use crate::room::Room;

pub type PublicServers = Vec<String>;

pub fn fetch_public_nodes(_: &Room, mut external_nodes: PublicServers) -> PublicServers {
    external_nodes.extend_from_slice(&[
        "tcp://public.easytier.top:11010",
        "tcp://public2.easytier.cn:54321",
        "https://etnode.zkitefly.eu.org/node1",
        "https://etnode.zkitefly.eu.org/node2",
    ].map(|s| s.into()));

    external_nodes
}
