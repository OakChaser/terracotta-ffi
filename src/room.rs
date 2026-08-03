// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors
//
// Ported from burningtnt/Terracotta (https://github.com/burningtnt/Terracotta).
// Original copyright (c) burningtnt.
// Licensed under AGPL-3.0-or-later. See THIRD_PARTY_LICENSE.

use rand_core::{OsRng, TryRngCore};

const FORMAT: &str = "U/XXXX-XXXX-XXXX-XXXX";
const DIGITS: &str = "XXXX-XXXX-XXXX-XXXX";
const CHARS: &[u8] = b"0123456789ABCDEFGHJKLMNPQRSTUVWXYZ";

fn lookup_char(c: char) -> Option<u8> {
    let c = match c {
        'I' => '1',
        'O' => '0',
        _ => c,
    };
    CHARS.iter().position(|&x| x as char == c).map(|i| i as u8)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Room {
    pub code: String,
    pub network_name: String,
    pub network_secret: String,
}

impl Room {
    pub fn create() -> Room {
        let mut bytes = [0u8; 16];
        OsRng.try_fill_bytes(&mut bytes).expect("cannot fill os rng");
        let value = u128::from_be_bytes(bytes) % 34u128.pow(16);
        let value = value - value % 7;
        Self::from_value(value)
    }

    pub fn parse(code: &str) -> Option<Room> {
        let code: Vec<char> = code.to_ascii_uppercase().chars().collect();
        if code.len() < FORMAT.len() {
            return None;
        }

        let value = code.windows(FORMAT.len()).find_map(|window| {
            if window[0] != 'U' || window[1] != '/' {
                return None;
            }

            let mut value: u128 = 0;
            for i in (0..DIGITS.len()).rev() {
                if i == 4 || i == 9 || i == 14 {
                    if window[i + 2] != '-' {
                        return None;
                    }
                } else {
                    let v = lookup_char(window[i + 2])?;
                    value = value * 34 + v as u128;
                }
            }

            value.is_multiple_of(7).then_some(value)
        })?;

        Some(Self::from_value(value))
    }

    pub fn verify(code: &str) -> bool {
        Self::parse(code).is_some()
    }

    fn from_value(mut value: u128) -> Room {
        let mut code = String::with_capacity(FORMAT.len());
        code.push_str("U/");
        let mut network_name = String::with_capacity("scaffolding-mc-XXXX-XXXX".len());
        network_name.push_str("scaffolding-mc-");
        let mut network_secret = String::with_capacity("XXXX-XXXX".len());

        for i in 0..16 {
            let v = CHARS[(value % 34) as usize] as char;
            value /= 34;

            if i == 4 || i == 8 || i == 12 {
                code.push('-');
            }
            code.push(v);

            if i < 8 {
                if i == 4 {
                    network_name.push('-');
                }
                network_name.push(v);
            } else {
                if i == 12 {
                    network_secret.push('-');
                }
                network_secret.push(v);
            }
        }

        debug_assert_eq!(value, 0);
        debug_assert_eq!(code.len(), FORMAT.len());
        debug_assert_eq!(network_name.len(), "scaffolding-mc-XXXX-XXXX".len());
        debug_assert_eq!(network_secret.len(), "XXXX-XXXX".len());

        Room {
            code,
            network_name,
            network_secret,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_parse_round_trip() {
        for _ in 0..256 {
            let room = Room::create();
            assert_eq!(room.code.len(), FORMAT.len());
            assert!(room.code.starts_with("U/"));

            let parsed = Room::parse(&room.code).expect("round trip");
            assert_eq!(parsed, room);
            assert_eq!(parsed.network_name, format!("scaffolding-mc-{}", &room.code[2..11]));
            assert_eq!(
                parsed.network_secret,
                format!("{}-{}", &room.code[12..16], &room.code[17..21])
            );
        }
    }

    #[test]
    fn parse_tolerates_prefix_and_case() {
        let room = Room::create();
        let lower = room.code.to_ascii_lowercase();
        let with_prefix = format!("[SCANNING] {}", lower);
        assert_eq!(Room::parse(&with_prefix), Some(room));
    }

    #[test]
    fn parse_rejects_invalid() {
        assert!(Room::parse("").is_none());
        assert!(Room::parse("X/AAAA-AAAA-AAAA-AAAA").is_none());
        assert!(Room::parse("U/AAAA-AAAA-AAAA-AAA").is_none());
        assert!(Room::parse("U/AAAA AAAA AAAA AAAA").is_none());
        assert!(Room::parse("U/AAAA-AAAA+AAAA-AAAA").is_none());
    }
}
