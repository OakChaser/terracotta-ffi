use std::io;
use std::net::{Ipv4Addr, TcpListener};

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum PortRequest {
    EasyTierRPC = 0,
    Scaffolding = 1,
    Minecraft = 2,
}

impl PortRequest {
    pub fn request_specific(port: u16) -> io::Result<u16> {
        TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .and_then(|socket| socket.local_addr())
            .map(|address| address.port())
    }

    pub fn request(self) -> u16 {
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .and_then(|socket| socket.local_addr())
            .map(|address| address.port())
            .unwrap_or(self as u8 as u16 + 35780)
    }
}
