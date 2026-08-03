// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors

//! terracotta-test — host example for conic-terracotta via the C ABI.
//!
//! This example shows how an external program (e.g. a Minecraft launcher)
//! uses the shared library. It loads `libconic_terracotta` dynamically with
//! `libloading`, drives it purely through the exported C functions, and never
//! touches conic-terracotta's internal Rust API.
//!
//! Behavior:
//!   1. Load the dynamic library.
//!   2. Create a Terracotta context (terracotta_create).
//!   3. Configure it (terracotta_configure): data_dir, motd.
//!   4. Host a room (terracotta_create_room).
//!   5. Print the room code, then print state changes / events forever,
//!      so other players can join the room.

mod bindings;

use bindings::*;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::Duration;

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
    let lib_path = library_path();
    println!("Loading dynamic library: {}", lib_path.display());
    if !lib_path.exists() {
        eprintln!(
            "error: library not found. Build it with `cargo build --release` in \
             conic-terracotta and copy the artifact into examples/terracotta-test/lib/"
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

    // Create the context handle. This spawns the library's internal runtime.
    let handle = unsafe { lib.create() };
    if handle.is_null() {
        eprintln!("error: terracotta_create returned NULL handle");
        exit(1);
    }

    // Configure before issuing any command. A dedicated data_dir keeps this
    // instance's EasyTier extraction and machine-id separate from other
    // instances on the same machine.
    let data_dir = std::env::temp_dir().join("conic-terracotta-example-host");
    let data_dir_c =
        CString::new(data_dir.to_string_lossy().as_bytes()).expect("data_dir contains a NUL byte");
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

    // Host a room. room_code is NULL here so the library generates a new one.
    println!("Creating room...");
    let result = unsafe { lib.create_room(handle, Some("Conic_Connect")) };
    if result != TERRA_OK {
        eprintln!(
            "error: terracotta_create_room returned {} ({})",
            result,
            result_code_name(result)
        );
        unsafe { lib.destroy(handle) };
        exit(1);
    }

    // Wait until the runtime processes the command and the room code appears.
    let mut room_code = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while room_code.is_empty() {
        drain_events(&lib, handle);
        match unsafe { lib.get_state(handle) } {
            Ok(mut state) => {
                room_code = state.room_code.as_string();
                if !room_code.is_empty() {
                    println!(
                        "State: {} (version {})",
                        state_name(state.state),
                        state.version
                    );
                    let detail = state.detail.as_string();
                    if !detail.is_empty() && detail != "{}" {
                        println!("  detail: {detail}");
                    }
                }
                unsafe { lib.free_state(&mut state) };
            }
            Err(code) => {
                eprintln!(
                    "error: terracotta_get_state returned {code} ({})",
                    result_code_name(code)
                );
                unsafe { lib.destroy(handle) };
                exit(1);
            }
        }
        if std::time::Instant::now() > deadline {
            eprintln!("error: no room code obtained within 10s");
            unsafe { lib.destroy(handle) };
            exit(1);
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    println!();
    println!("Room code: {room_code}");
    println!(
        "Verify:    {}",
        if unsafe { lib.verify_room_code(&room_code) } == 3 {
            "valid"
        } else {
            "INVALID"
        }
    );
    println!();
    println!("Waiting for players to join... (Ctrl+C to stop)");

    // Keep running: report state transitions and events until stopped.
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
