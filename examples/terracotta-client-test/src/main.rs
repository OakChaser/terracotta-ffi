// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors

//! terracotta-client-test — guest example for conic-terracotta via the C ABI.
//!
//! Usage: terracotta-client-test <room_code>
//!
//! This example shows how an external program (e.g. a Minecraft launcher)
//! joins a room hosted by another Terracotta instance. It loads
//! `libconic_terracotta` dynamically with `libloading` and drives it purely
//! through the exported C functions.
//!
//! Behavior:
//!   1. Load the dynamic library.
//!   2. Create a Terracotta context (terracotta_create).
//!   3. Configure it (terracotta_configure): data_dir, motd.
//!   4. Join the given room (terracotta_join_room).
//!   5. Print the connection process: state transitions and events.

mod bindings;

use bindings::*;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::Duration;

const USAGE: &str = "Usage: terracotta-client-test <room_code>\n\
                     e.g.   terracotta-client-test U/ABCD-EFGH-JKMN-PQRS";

fn lib_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("lib")
}

/// Path of the dynamic library, per platform.
fn library_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        lib_dir().join("libconic_terracotta.dylib")
    } else if cfg!(target_os = "windows") {
        lib_dir().join("conic_terracotta.dll")
    } else {
        lib_dir().join("libconic_terracotta.so")
    }
}

/// Human-readable name for an error result code.
fn result_code_name(code: TerraResult) -> String {
    match code {
        TERRA_OK => "TERRA_OK".into(),
        TERRA_ERR_INVALID_HANDLE => "TERRA_ERR_INVALID_HANDLE".into(),
        TERRA_ERR_INVALID_ARGUMENT => "TERRA_ERR_INVALID_ARGUMENT".into(),
        TERRA_ERR_BAD_STATE => "TERRA_ERR_BAD_STATE".into(),
        TERRA_ERR_INVALID_ROOM_CODE => "TERRA_ERR_INVALID_ROOM_CODE".into(),
        TERRA_ERR_ALREADY_ACTIVE => "TERRA_ERR_ALREADY_ACTIVE".into(),
        TERRA_ERR_INTERNAL => "TERRA_ERR_INTERNAL".into(),
        TERRA_ERR_OUT_OF_MEMORY => "TERRA_ERR_OUT_OF_MEMORY".into(),
        TERRA_ERR_NO_EVENT => "TERRA_ERR_NO_EVENT".into(),
        TERRA_ERR_SHUTTING_DOWN => "TERRA_ERR_SHUTTING_DOWN".into(),
        other => format!("UNKNOWN({other})"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        println!("{USAGE}");
        exit(1);
    }
    let room_code = &args[1];

    let lib_path = library_path();
    println!("Loading dynamic library: {}", lib_path.display());
    if !lib_path.exists() {
        eprintln!(
            "error: library not found. Build it with `cargo build --release` in \
             conic-terracotta and copy the artifact into \
             examples/terracotta-client-test/lib/"
        );
        exit(1);
    }

    let lib = match Terracotta::load(&lib_path) {
        Ok(lib) => lib,
        Err(err) => {
            eprintln!("error: cannot load library: {err}");
            exit(1);
        }
    };
    println!("terracotta library version: {}", unsafe { lib.version() });

    // Verify the room code before creating a context.
    if unsafe { lib.verify_room_code(room_code) } != 3 {
        eprintln!("error: invalid room code: {room_code}");
        println!("{USAGE}");
        exit(1);
    }

    let handle = unsafe { lib.create() };
    if handle.is_null() {
        eprintln!("error: terracotta_create returned NULL handle");
        exit(1);
    }

    // Configure before issuing any command. A dedicated data_dir keeps this
    // instance's EasyTier extraction and machine-id separate from other
    // instances on the same machine.
    let data_dir = std::env::temp_dir().join("conic-terracotta-example-client");
    let data_dir_c = CString::new(data_dir.to_string_lossy().as_bytes())
        .expect("data_dir contains a NUL byte");
    let motd_c = CString::new("Ciallo～(∠・ω< )⌒★!").expect("motd contains a NUL byte");
    let config = TerraConfig {
        public_nodes: std::ptr::null(),
        public_nodes_count: 0,
        data_dir: TerraString {
            data: data_dir_c.as_ptr(),
            len: data_dir_c.as_bytes().len() as u32,
        },
        motd: TerraString {
            data: motd_c.as_ptr(),
            len: motd_c.as_bytes().len() as u32,
        },
    };
    let config_result = unsafe { lib.configure(handle, Some(&config)) };
    if config_result != TERRA_OK {
        eprintln!(
            "warning: terracotta_configure returned {}",
            result_code_name(config_result)
        );
    }
    println!("Using data_dir: {}", data_dir.display());

    println!();
    println!("Joining room:");
    println!("{room_code}");
    println!();

    let result = unsafe { lib.join_room(handle, room_code, Some("conic-example-guest")) };
    if result != TERRA_OK {
        eprintln!(
            "error: terracotta_join_room returned {} ({})",
            result,
            result_code_name(result)
        );
        unsafe { lib.destroy(handle) };
        exit(1);
    }

    println!("Connecting...");
    println!("Running... (Ctrl+C to stop)");
    println!("Events will be printed below:");
    println!();

    // Report state transitions and events until stopped.
    let mut last_state = -1;
    loop {
        drain_events(&lib, handle);

        match unsafe { lib.get_state(handle) } {
            Ok(mut state) => {
                if state.state != last_state {
                    println!(
                        "[state] {} (version {})",
                        state_name(state.state),
                        state.version
                    );
                    let detail = state.detail.as_string();
                    if !detail.is_empty() && detail != "{}" {
                        println!("  detail: {detail}");
                    }
                    last_state = state.state;
                }
                unsafe { lib.free_state(&mut state) };
            }
            Err(code) => {
                eprintln!(
                    "warning: terracotta_get_state returned {code} ({})",
                    result_code_name(code)
                );
            }
        }

        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Pop the whole event queue, printing every event. `TERRA_ERR_NO_EVENT`
/// simply means the queue is empty.
fn drain_events(lib: &Terracotta, handle: TerraHandle) {
    loop {
        let mut event = match unsafe { lib.poll_event(handle) } {
            Ok(event) => event,
            Err(TERRA_ERR_NO_EVENT) | Err(TERRA_ERR_INVALID_HANDLE) => return,
            Err(code) => {
                eprintln!(
                    "warning: terracotta_poll_event returned {code} ({})",
                    result_code_name(code)
                );
                return;
            }
        };
        let payload = event.payload.as_string();
        println!(
            "[event #{}] {} payload={payload}",
            event.sequence,
            event_name(event.r#type)
        );
        unsafe { lib.free_event(&mut event) };
    }
}
