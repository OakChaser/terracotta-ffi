// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ConicMC contributors

//! C ABI for Conic Launcher. See include/terracotta.h for the contract.
//!
//! The handle is a stable pointer to a `HandleRecord` which owns an
//! `Arc<TerracottaContext>`. The record is intentionally leaked after
//! destruction so that calling any function with a stale handle yields
//! `TERRA_ERR_INVALID_HANDLE` instead of dereferencing freed memory.

use crate::context::{ContextConfig, TerracottaContext};
use crate::room::Room;
use crate::session::SessionStateId;
use std::cell::UnsafeCell;
use std::ffi::{c_char, c_void, CString};
use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const TERRA_OK: i32 = 0;
const TERRA_ERR_INVALID_HANDLE: i32 = -1;
const TERRA_ERR_INVALID_ARGUMENT: i32 = -2;
const TERRA_ERR_BAD_STATE: i32 = -3;
const TERRA_ERR_INVALID_ROOM_CODE: i32 = -4;
const TERRA_ERR_ALREADY_ACTIVE: i32 = -5;
const TERRA_ERR_INTERNAL: i32 = -6;
const TERRA_ERR_OUT_OF_MEMORY: i32 = -7;
const TERRA_ERR_NO_EVENT: i32 = -8;
const TERRA_ERR_SHUTTING_DOWN: i32 = -9;

#[allow(non_camel_case_types)]
pub type terracotta_handle = *mut c_void;

#[repr(C)]
pub struct TerracottaString {
    data: *const c_char,
    len: u32,
}

#[repr(C)]
pub struct TerracottaEvent {
    sequence: u64,
    r#type: i32,
    payload: TerracottaString,
}

#[repr(C)]
pub struct TerracottaState {
    version: u64,
    state: i32,
    room_code: TerracottaString,
    detail: TerracottaString,
}

#[repr(C)]
pub struct TerracottaConfig {
    public_nodes: *const TerracottaString,
    public_nodes_count: u32,
    data_dir: TerracottaString,
    motd: TerracottaString,
}

struct HandleRecord {
    destroyed: AtomicBool,
    arc: UnsafeCell<Option<Arc<TerracottaContext>>>,
}

impl HandleRecord {
    fn create_handle(arc: Arc<TerracottaContext>) -> terracotta_handle {
        Box::into_raw(Box::new(HandleRecord {
            destroyed: AtomicBool::new(false),
            arc: UnsafeCell::new(Some(arc)),
        })) as terracotta_handle
    }

    fn get(handle: terracotta_handle) -> Result<&'static HandleRecord, i32> {
        if handle.is_null() {
            return Err(TERRA_ERR_INVALID_HANDLE);
        }
        let record = unsafe { &*(handle as *const HandleRecord) };
        if record.destroyed.load(Ordering::Acquire) {
            return Err(TERRA_ERR_INVALID_HANDLE);
        }
        Ok(record)
    }

    fn with<T>(&self, f: impl FnOnce(&TerracottaContext) -> Result<T, i32>) -> Result<T, i32> {
        if self.destroyed.load(Ordering::Acquire) {
            return Err(TERRA_ERR_INVALID_HANDLE);
        }
        let arc = unsafe { &*self.arc.get() };
        match arc.as_ref() {
            Some(ctx) => f(ctx),
            None => Err(TERRA_ERR_INVALID_HANDLE),
        }
    }
}

fn catch<T>(f: impl FnOnce() -> Result<T, i32>) -> Result<T, i32> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => Err(TERRA_ERR_INTERNAL),
    }
}

fn make_string(value: String) -> TerracottaString {
    let c = CString::new(value).unwrap_or_else(|_| CString::new("\0").expect("nul byte"));
    let len = c.as_bytes().len() as u32;
    TerracottaString {
        data: c.into_raw(),
        len,
    }
}

fn make_empty() -> TerracottaString {
    let c = CString::new("").expect("nul byte");
    TerracottaString {
        data: c.into_raw(),
        len: 0,
    }
}

fn read_string(s: &TerracottaString) -> Result<String, i32> {
    if s.data.is_null() || s.len == 0 {
        return Ok(String::new());
    }
    let bytes = unsafe { std::slice::from_raw_parts(s.data as *const u8, s.len as usize) };
    String::from_utf8(bytes.to_vec()).map_err(|_| TERRA_ERR_INVALID_ARGUMENT)
}

fn free_string(s: &mut TerracottaString) {
    if !s.data.is_null() {
        unsafe { drop(CString::from_raw(s.data as *mut c_char)) };
        s.data = null_mut();
        s.len = 0;
    }
}

fn room_code_from_c(room_code: *const c_char) -> Result<Option<String>, i32> {
    if room_code.is_null() {
        return Ok(None);
    }
    let value = unsafe { std::ffi::CStr::from_ptr(room_code) };
    let value = value.to_str().map_err(|_| TERRA_ERR_INVALID_ARGUMENT)?;
    if value.is_empty() {
        return Ok(None);
    }
    match Room::parse(value) {
        Some(_) => Ok(Some(value.to_string())),
        None => Err(TERRA_ERR_INVALID_ROOM_CODE),
    }
}

fn player_name_from_c(player_name: *const c_char) -> Result<Option<String>, i32> {
    if player_name.is_null() {
        return Ok(None);
    }
    let value = unsafe { std::ffi::CStr::from_ptr(player_name) };
    let value = value.to_str().map_err(|_| TERRA_ERR_INVALID_ARGUMENT)?;
    let value = if value.is_empty() { None } else { Some(value.to_string()) };
    Ok(value)
}

fn require_waiting(ctx: &TerracottaContext) -> Result<(), i32> {
    let state = ctx.emitter().session().lock().state();
    if state == SessionStateId::Waiting {
        Ok(())
    } else {
        Err(TERRA_ERR_ALREADY_ACTIVE)
    }
}

static VERSION_STRING: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terracotta_create() -> terracotta_handle {
    match catch(|| {
        let ctx = TerracottaContext::create(ContextConfig::default())
            .map_err(|_| TERRA_ERR_INTERNAL)?;
        Ok(HandleRecord::create_handle(ctx))
    }) {
        Ok(handle) => handle,
        Err(_) => null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terracotta_destroy(handle: terracotta_handle) {
    let Ok(record) = HandleRecord::get(handle) else {
        return;
    };
    if record.destroyed.swap(true, Ordering::AcqRel) {
        return;
    }
    let arc = unsafe { (*record.arc.get()).take() };
    if let Some(ctx) = arc {
        ctx.destroy();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terracotta_configure(
    handle: terracotta_handle,
    config: *const TerracottaConfig,
) -> i32 {
    let result = catch(|| {
        let record = HandleRecord::get(handle)?;
        record.with(|ctx| {
            let mut public_nodes = Vec::new();
            let mut data_dir = std::env::temp_dir().join("conic-terracotta");
            let mut motd = None;

            if !config.is_null() {
                let config = unsafe { &*config };
                if !config.public_nodes.is_null() {
                    for i in 0..config.public_nodes_count as usize {
                        let node = unsafe { &*config.public_nodes.add(i) };
                        public_nodes.push(read_string(node)?);
                    }
                }
                let dir = read_string(&config.data_dir)?;
                if !dir.is_empty() {
                    data_dir = PathBuf::from(dir);
                }
                let motd_value = read_string(&config.motd)?;
                if !motd_value.is_empty() {
                    motd = Some(motd_value);
                }
            }

            let state = ctx.emitter().session().lock().state();
            if state != SessionStateId::Waiting {
                return Err(TERRA_ERR_BAD_STATE);
            }
            ctx.set_config(ContextConfig {
                public_nodes,
                data_dir,
                motd,
            });
            Ok(())
        })
    });
    match result {
        Ok(()) => TERRA_OK,
        Err(code) => code,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terracotta_create_room(
    handle: terracotta_handle,
    player_name: *const c_char,
    room_code: *const c_char,
) -> i32 {
    let result = catch(|| {
        let record = HandleRecord::get(handle)?;
        record.with(|ctx| {
            let room_code = room_code_from_c(room_code)?;
            let player_name = player_name_from_c(player_name)?;
            require_waiting(ctx)?;
            ctx.submit_create_room(player_name, room_code, Vec::new());
            Ok(())
        })
    });
    match result {
        Ok(()) => TERRA_OK,
        Err(code) => code,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terracotta_join_room(
    handle: terracotta_handle,
    room_code: *const c_char,
    player_name: *const c_char,
) -> i32 {
    let result = catch(|| {
        let record = HandleRecord::get(handle)?;
        record.with(|ctx| {
            if room_code.is_null() {
                return Err(TERRA_ERR_INVALID_ARGUMENT);
            }
            let code = unsafe { std::ffi::CStr::from_ptr(room_code) }
                .to_str()
                .map_err(|_| TERRA_ERR_INVALID_ARGUMENT)?;
            if Room::parse(code).is_none() {
                return Err(TERRA_ERR_INVALID_ROOM_CODE);
            }
            let player_name = player_name_from_c(player_name)?;
            require_waiting(ctx)?;
            ctx.submit_join_room(code.to_string(), player_name, Vec::new());
            Ok(())
        })
    });
    match result {
        Ok(()) => TERRA_OK,
        Err(code) => code,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terracotta_set_waiting(handle: terracotta_handle) -> i32 {
    let result = catch(|| {
        let record = HandleRecord::get(handle)?;
        record.with(|ctx| {
            ctx.submit_set_waiting();
            Ok(())
        })
    });
    match result {
        Ok(()) => TERRA_OK,
        Err(code) => code,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terracotta_get_state(
    handle: terracotta_handle,
    out: *mut TerracottaState,
) -> i32 {
    let result = catch(|| {
        let record = HandleRecord::get(handle)?;
        record.with(|ctx| {
            if out.is_null() {
                return Err(TERRA_ERR_INVALID_ARGUMENT);
            }
            let snapshot = ctx.session_snapshot();
            let state = TerracottaState {
                version: snapshot.version,
                state: snapshot.state as i32,
                room_code: make_string(snapshot.room_code),
                detail: make_string(snapshot.detail.to_string()),
            };
            unsafe { *out = state };
            Ok(())
        })
    });
    match result {
        Ok(()) => TERRA_OK,
        Err(code) => code,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terracotta_poll_event(
    handle: terracotta_handle,
    out: *mut TerracottaEvent,
) -> i32 {
    let result = catch(|| {
        let record = HandleRecord::get(handle)?;
        record.with(|ctx| {
            if out.is_null() {
                return Err(TERRA_ERR_INVALID_ARGUMENT);
            }
            let Some(event) = ctx.poll_event() else {
                return Err(TERRA_ERR_NO_EVENT);
            };
            let sequence = ctx.emitter().next_sequence();
            let event = TerracottaEvent {
                sequence,
                r#type: event.type_id() as i32,
                payload: make_string(event.payload_json().to_string()),
            };
            unsafe { *out = event };
            Ok(())
        })
    });
    match result {
        Ok(()) => TERRA_OK,
        Err(code) => code,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terracotta_verify_room_code(room_code: *const c_char) -> i32 {
    catch(|| -> Result<i32, i32> {
        if room_code.is_null() {
            return Ok(-1);
        }
        let value = unsafe { std::ffi::CStr::from_ptr(room_code) }
            .to_str()
            .map_err(|_| TERRA_ERR_INVALID_ARGUMENT)?;
        if Room::verify(value) {
            Ok(3)
        } else {
            Ok(-1)
        }
    })
    .unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terracotta_version() -> *const c_char {
    VERSION_STRING.as_ptr() as *const c_char
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terracotta_free_string(value: *mut TerracottaString) {
    if value.is_null() {
        return;
    }
    let _ = catch(|| {
        free_string(unsafe { &mut *value });
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terracotta_free_state(value: *mut TerracottaState) {
    if value.is_null() {
        return;
    }
    let _ = catch(|| {
        let state = unsafe { &mut *value };
        free_string(&mut state.room_code);
        free_string(&mut state.detail);
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn terracotta_free_event(value: *mut TerracottaEvent) {
    if value.is_null() {
        return;
    }
    let _ = catch(|| {
        let event = unsafe { &mut *value };
        free_string(&mut event.payload);
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_handle() -> terracotta_handle {
        unsafe { terracotta_create() }
    }

    #[test]
    fn version_and_verify() {
        let version = unsafe { terracotta_version() };
        let version = unsafe { std::ffi::CStr::from_ptr(version) };
        assert_eq!(version.to_str().unwrap(), env!("CARGO_PKG_VERSION"));

        let room = Room::create();
        let c = std::ffi::CString::new(room.code.clone()).unwrap();
        assert_eq!(unsafe { terracotta_verify_room_code(c.as_ptr()) }, 3);

        let bad = std::ffi::CString::new("not-a-code").unwrap();
        assert_eq!(unsafe { terracotta_verify_room_code(bad.as_ptr()) }, -1);
        assert_eq!(unsafe { terracotta_verify_room_code(std::ptr::null()) }, -1);
    }

    #[test]
    fn create_room_poll_events_and_destroy() {
        let handle = make_handle();
        assert!(!handle.is_null());

        let name = std::ffi::CString::new("test-player").unwrap();
        let mut event = TerracottaEvent {
            sequence: 0,
            r#type: 0,
            payload: TerracottaString { data: null_mut(), len: 0 },
        };

        assert_eq!(
            unsafe { terracotta_create_room(handle, name.as_ptr(), std::ptr::null()) },
            TERRA_OK
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let code = unsafe { terracotta_poll_event(handle, &mut event) };
            if code == TERRA_OK {
                if event.r#type == 1 {
                    break;
                }
                unsafe { terracotta_free_event(&mut event) };
            }
            if std::time::Instant::now() > deadline {
                panic!("no StateChanged event within 5s");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        unsafe { terracotta_free_event(&mut event) };

        let mut state = TerracottaState {
            version: 0,
            state: 0,
            room_code: TerracottaString { data: null_mut(), len: 0 },
            detail: TerracottaString { data: null_mut(), len: 0 },
        };
        assert_eq!(unsafe { terracotta_get_state(handle, &mut state) }, TERRA_OK);
        assert_eq!(state.state, SessionStateId::HostScanning as i32);
        assert!(!state.room_code.data.is_null());
        assert_eq!(state.room_code.len, 21);
        unsafe { terracotta_free_state(&mut state) };

        assert_eq!(unsafe { terracotta_set_waiting(handle) }, TERRA_OK);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(unsafe { terracotta_get_state(handle, &mut state) }, TERRA_OK);
        assert_eq!(state.state, SessionStateId::Waiting as i32);
        unsafe { terracotta_free_state(&mut state) };

        unsafe { terracotta_destroy(handle) };
        unsafe { terracotta_destroy(handle) };
        assert_eq!(
            unsafe { terracotta_set_waiting(handle) },
            TERRA_ERR_INVALID_HANDLE
        );
        assert_eq!(
            unsafe { terracotta_get_state(handle, &mut state) },
            TERRA_ERR_INVALID_HANDLE
        );
    }

    #[test]
    fn invalid_handle_and_bad_room_code() {
        let handle = make_handle();

        let bad = std::ffi::CString::new("U/INVALID").unwrap();
        assert_eq!(
            unsafe { terracotta_create_room(handle, std::ptr::null(), bad.as_ptr()) },
            TERRA_ERR_INVALID_ROOM_CODE
        );

        let c = std::ffi::CString::new("U/AAAA-AAAA-AAAA-AAA").unwrap();
        assert_eq!(
            unsafe { terracotta_join_room(handle, c.as_ptr(), std::ptr::null()) },
            TERRA_ERR_INVALID_ROOM_CODE
        );

        let mut state = TerracottaState {
            version: 0,
            state: 0,
            room_code: TerracottaString { data: null_mut(), len: 0 },
            detail: TerracottaString { data: null_mut(), len: 0 },
        };
        assert_eq!(unsafe { terracotta_get_state(std::ptr::null_mut(), &mut state) }, TERRA_ERR_INVALID_HANDLE);

        let mut event = TerracottaEvent {
            sequence: 0,
            r#type: 0,
            payload: TerracottaString { data: null_mut(), len: 0 },
        };
        assert_eq!(unsafe { terracotta_poll_event(handle, &mut event) }, TERRA_ERR_NO_EVENT);
        unsafe { terracotta_free_event(&mut event) };

        unsafe { terracotta_destroy(handle) };
    }

    #[test]
    fn configure_motd_roundtrip() {
        let handle = make_handle();

        let motd = std::ffi::CString::new("custom-motd").unwrap();
        let data_dir = std::ffi::CString::new("/tmp/conic-ffi-motd-test").unwrap();
        let config = TerracottaConfig {
            public_nodes: null_mut(),
            public_nodes_count: 0,
            data_dir: TerracottaString {
                data: data_dir.as_ptr(),
                len: data_dir.as_bytes().len() as u32,
            },
            motd: TerracottaString {
                data: motd.as_ptr(),
                len: motd.as_bytes().len() as u32,
            },
        };
        assert_eq!(unsafe { terracotta_configure(handle, &config) }, TERRA_OK);

        let record = HandleRecord::get(handle).unwrap();
        record
            .with(|ctx| {
                let cfg = ctx.emitter().config();
                assert_eq!(cfg.motd.as_deref(), Some("custom-motd"));
                assert_eq!(cfg.data_dir, std::path::PathBuf::from("/tmp/conic-ffi-motd-test"));
                Ok(())
            })
            .unwrap();

        let empty = TerracottaConfig {
            public_nodes: null_mut(),
            public_nodes_count: 0,
            data_dir: TerracottaString { data: null_mut(), len: 0 },
            motd: TerracottaString { data: null_mut(), len: 0 },
        };
        assert_eq!(unsafe { terracotta_configure(handle, &empty) }, TERRA_OK);
        record
            .with(|ctx| {
                assert_eq!(ctx.emitter().config().motd, None);
                Ok(())
            })
            .unwrap();

        unsafe { terracotta_destroy(handle) };
    }
}
