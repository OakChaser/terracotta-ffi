// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors

use crate::easytier::argument::{Argument, PortForward};
use crate::session::ConnectionDifficulty;
use std::net::Ipv4Addr;
use std::path::Path;

pub mod argument;
pub mod process;
pub mod publics;

pub(crate) use process::initialize;

pub struct EasyTier(process::EasyTier);

#[derive(Debug)]
pub struct EasyTierMember {
    pub hostname: String,
    pub address: Option<Ipv4Addr>,
    pub is_local: bool,
    pub nat: NatType,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NatType {
    Unknown,
    OpenInternet,
    NoPAT,
    FullCone,
    Restricted,
    PortRestricted,
    Symmetric,
    SymmetricUdpWall,
    SymmetricEasyIncrease,
    SymmetricEasyDecrease,
}

pub fn calc_conn_difficulty(left: &NatType, right: &NatType) -> ConnectionDifficulty {
    let is = |types: &[NatType]| -> bool { types.contains(left) || types.contains(right) };

    if is(&[NatType::OpenInternet]) {
        ConnectionDifficulty::Easiest
    } else if is(&[NatType::NoPAT, NatType::FullCone]) {
        ConnectionDifficulty::Simple
    } else if is(&[NatType::Restricted, NatType::PortRestricted]) {
        ConnectionDifficulty::Medium
    } else {
        ConnectionDifficulty::Tough
    }
}

pub fn create(data_dir: &Path, args: Vec<Argument>) -> EasyTier {
    EasyTier(process::create(data_dir, args))
}

impl EasyTier {
    pub fn is_alive(&self) -> bool {
        self.0.is_alive()
    }

    pub fn get_players(&self) -> Option<Vec<EasyTierMember>> {
        self.0.get_players()
    }

    pub fn add_port_forward(&mut self, forwards: &[PortForward]) -> bool {
        self.0.add_port_forward(forwards)
    }
}
