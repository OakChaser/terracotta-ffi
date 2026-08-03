// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors

use rand_core::{OsRng, TryRngCore};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub fn get_or_create(path: &Path) -> Option<String> {
    if let Ok(mut file) = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
    {
        let mut bytes = [0u8; 17];
        match file.read(&mut bytes) {
            Ok(16) => {}
            Ok(length) => {
                logging!(
                    "MachineID",
                    "cannot restore machine id: expected 16 bytes, got {}",
                    length
                );
                if OsRng.try_fill_bytes(&mut bytes[0..16]).is_err() {
                    return None;
                }
                if file.seek(SeekFrom::Start(0)).is_ok() {
                    let _ = file.write(&bytes[0..16]);
                }
            }
            Err(e) => {
                logging!("MachineID", "cannot read machine id file: {:?}", e);
            }
        }
        return Some(hex::encode(&bytes[0..16]));
    }

    let mut bytes = [0u8; 16];
    OsRng.try_fill_bytes(&mut bytes).ok()?;
    Some(hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_across_calls() {
        let dir = std::env::temp_dir().join(format!("conic-terracotta-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("machine-id");

        let first = get_or_create(&path).expect("first");
        let second = get_or_create(&path).expect("second");
        assert_eq!(first, second);
        assert_eq!(first.len(), 32);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
