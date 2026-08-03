//! Complete C ABI bindings for `libconic_terracotta` (see include/terracotta.h).
//!
//! This mirrors the header exactly. No conic-terracotta Rust types are used;
//! every interaction happens through the dynamic library's exported symbols.
//!
//! This file is the reference binding for external integrations: it declares
//! the full ABI, so some symbols may be unused by a particular example.

#![allow(dead_code)]

use libloading::{Library, Symbol};
use std::ffi::{c_char, c_void, CStr, CString};
use std::os::raw::c_int;
use std::path::Path;

pub type TerraHandle = *mut c_void;
pub type TerraResult = c_int;

pub const TERRA_OK: TerraResult = 0;
pub const TERRA_ERR_INVALID_HANDLE: TerraResult = -1;
pub const TERRA_ERR_INVALID_ARGUMENT: TerraResult = -2;
pub const TERRA_ERR_BAD_STATE: TerraResult = -3;
pub const TERRA_ERR_INVALID_ROOM_CODE: TerraResult = -4;
pub const TERRA_ERR_ALREADY_ACTIVE: TerraResult = -5;
pub const TERRA_ERR_INTERNAL: TerraResult = -6;
pub const TERRA_ERR_OUT_OF_MEMORY: TerraResult = -7;
pub const TERRA_ERR_NO_EVENT: TerraResult = -8;
pub const TERRA_ERR_SHUTTING_DOWN: TerraResult = -9;

pub const TERRA_STATE_WAITING: i32 = 0;
pub const TERRA_STATE_HOST_SCANNING: i32 = 1;
pub const TERRA_STATE_HOST_STARTING: i32 = 2;
pub const TERRA_STATE_HOST_OK: i32 = 3;
pub const TERRA_STATE_GUEST_CONNECTING: i32 = 4;
pub const TERRA_STATE_GUEST_STARTING: i32 = 5;
pub const TERRA_STATE_GUEST_OK: i32 = 6;
pub const TERRA_STATE_EXCEPTION: i32 = 7;

pub const TERRA_EVENT_STATE_CHANGED: i32 = 1;
pub const TERRA_EVENT_PLAYER_JOINED: i32 = 2;
pub const TERRA_EVENT_PLAYER_LEFT: i32 = 3;
pub const TERRA_EVENT_CONNECTION_DIFFICULTY: i32 = 4;
pub const TERRA_EVENT_HOST_READY: i32 = 5;
pub const TERRA_EVENT_GUEST_READY: i32 = 6;
pub const TERRA_EVENT_ERROR: i32 = 7;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TerraString {
    pub data: *const c_char,
    pub len: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TerraEvent {
    pub sequence: u64,
    pub r#type: i32,
    pub payload: TerraString,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TerraState {
    pub version: u64,
    pub state: i32,
    pub room_code: TerraString,
    pub detail: TerraString,
}

#[repr(C)]
pub struct TerraConfig {
    pub public_nodes: *const TerraString,
    pub public_nodes_count: u32,
    pub data_dir: TerraString,
    pub motd: TerraString,
}

pub struct Terracotta {
    lib: Library,
    create: Symbol<'static, unsafe extern "C" fn() -> TerraHandle>,
    destroy: Symbol<'static, unsafe extern "C" fn(TerraHandle)>,
    configure: Symbol<'static, unsafe extern "C" fn(TerraHandle, *const TerraConfig) -> TerraResult>,
    create_room:
        Symbol<'static, unsafe extern "C" fn(TerraHandle, *const c_char, *const c_char) -> TerraResult>,
    join_room:
        Symbol<'static, unsafe extern "C" fn(TerraHandle, *const c_char, *const c_char) -> TerraResult>,
    set_waiting: Symbol<'static, unsafe extern "C" fn(TerraHandle) -> TerraResult>,
    get_state: Symbol<'static, unsafe extern "C" fn(TerraHandle, *mut TerraState) -> TerraResult>,
    poll_event: Symbol<'static, unsafe extern "C" fn(TerraHandle, *mut TerraEvent) -> TerraResult>,
    verify_room_code: Symbol<'static, unsafe extern "C" fn(*const c_char) -> c_int>,
    version: Symbol<'static, unsafe extern "C" fn() -> *const c_char>,
    free_string: Symbol<'static, unsafe extern "C" fn(*mut TerraString)>,
    free_state: Symbol<'static, unsafe extern "C" fn(*mut TerraState)>,
    free_event: Symbol<'static, unsafe extern "C" fn(*mut TerraEvent)>,
}

fn symbol<'lib, T>(lib: &'lib Library, name: &[u8]) -> Result<Symbol<'static, T>, libloading::Error> {
    let symbol = unsafe { lib.get::<T>(name)? };
    // The Library is stored in `Terracotta` and dropped after all symbols
    // (declaration order in the struct), so the 'static borrow is sound.
    Ok(unsafe { std::mem::transmute::<Symbol<'lib, T>, Symbol<'static, T>>(symbol) })
}

impl Terracotta {
    pub fn load(lib_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        unsafe {
            let lib = Library::new(lib_path)?;
            let create = symbol::<unsafe extern "C" fn() -> TerraHandle>(&lib, b"terracotta_create")?;
            let destroy = symbol::<unsafe extern "C" fn(TerraHandle)>(&lib, b"terracotta_destroy")?;
            let configure = symbol::<unsafe extern "C" fn(TerraHandle, *const TerraConfig) -> TerraResult>(
                &lib, b"terracotta_configure",
            )?;
            let create_room =
                symbol::<unsafe extern "C" fn(TerraHandle, *const c_char, *const c_char) -> TerraResult>(
                    &lib, b"terracotta_create_room",
                )?;
            let join_room =
                symbol::<unsafe extern "C" fn(TerraHandle, *const c_char, *const c_char) -> TerraResult>(
                    &lib, b"terracotta_join_room",
                )?;
            let set_waiting = symbol::<unsafe extern "C" fn(TerraHandle) -> TerraResult>(
                &lib, b"terracotta_set_waiting",
            )?;
            let get_state = symbol::<unsafe extern "C" fn(TerraHandle, *mut TerraState) -> TerraResult>(
                &lib, b"terracotta_get_state",
            )?;
            let poll_event = symbol::<unsafe extern "C" fn(TerraHandle, *mut TerraEvent) -> TerraResult>(
                &lib, b"terracotta_poll_event",
            )?;
            let verify_room_code = symbol::<unsafe extern "C" fn(*const c_char) -> c_int>(
                &lib, b"terracotta_verify_room_code",
            )?;
            let version = symbol::<unsafe extern "C" fn() -> *const c_char>(&lib, b"terracotta_version")?;
            let free_string =
                symbol::<unsafe extern "C" fn(*mut TerraString)>(&lib, b"terracotta_free_string")?;
            let free_state = symbol::<unsafe extern "C" fn(*mut TerraState)>(&lib, b"terracotta_free_state")?;
            let free_event = symbol::<unsafe extern "C" fn(*mut TerraEvent)>(&lib, b"terracotta_free_event")?;

            Ok(Terracotta {
                lib,
                create,
                destroy,
                configure,
                create_room,
                join_room,
                set_waiting,
                get_state,
                poll_event,
                verify_room_code,
                version,
                free_string,
                free_state,
                free_event,
            })
        }
    }

    pub unsafe fn create(&self) -> TerraHandle {
        (self.create)()
    }

    pub unsafe fn destroy(&self, handle: TerraHandle) {
        (self.destroy)(handle)
    }

    pub unsafe fn configure(&self, handle: TerraHandle, config: Option<&TerraConfig>) -> TerraResult {
        let ptr = match config {
            Some(c) => c as *const TerraConfig,
            None => std::ptr::null(),
        };
        (self.configure)(handle, ptr)
    }

    pub unsafe fn create_room(&self, handle: TerraHandle, player_name: Option<&str>) -> TerraResult {
        let name = cstr_or_null(player_name);
        let name_ptr = name.as_deref().map(|s| s.as_ptr()).unwrap_or(std::ptr::null());
        (self.create_room)(handle, name_ptr, std::ptr::null())
    }

    pub unsafe fn join_room(
        &self,
        handle: TerraHandle,
        room_code: &str,
        player_name: Option<&str>,
    ) -> TerraResult {
        let code = CString::new(room_code).unwrap();
        let name = cstr_or_null(player_name);
        let name_ptr = name.as_deref().map(|s| s.as_ptr()).unwrap_or(std::ptr::null());
        (self.join_room)(handle, code.as_ptr(), name_ptr)
    }

    pub unsafe fn set_waiting(&self, handle: TerraHandle) -> TerraResult {
        (self.set_waiting)(handle)
    }

    pub unsafe fn get_state(&self, handle: TerraHandle) -> Result<TerraState, TerraResult> {
        let mut state = TerraState {
            version: 0,
            state: 0,
            room_code: TerraString { data: std::ptr::null(), len: 0 },
            detail: TerraString { data: std::ptr::null(), len: 0 },
        };
        let result = (self.get_state)(handle, &mut state);
        if result == TERRA_OK {
            Ok(state)
        } else {
            Err(result)
        }
    }

    pub unsafe fn poll_event(&self, handle: TerraHandle) -> Result<TerraEvent, TerraResult> {
        let mut event = TerraEvent {
            sequence: 0,
            r#type: 0,
            payload: TerraString { data: std::ptr::null(), len: 0 },
        };
        let result = (self.poll_event)(handle, &mut event);
        if result == TERRA_OK {
            Ok(event)
        } else {
            Err(result)
        }
    }

    pub unsafe fn verify_room_code(&self, code: &str) -> c_int {
        let c = CString::new(code).unwrap();
        (self.verify_room_code)(c.as_ptr())
    }

    pub unsafe fn version(&self) -> String {
        let ptr = (self.version)();
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }

    pub unsafe fn free_state(&self, state: &mut TerraState) {
        (self.free_state)(state);
    }

    pub unsafe fn free_event(&self, event: &mut TerraEvent) {
        (self.free_event)(event);
    }
}

fn cstr_or_null(value: Option<&str>) -> Option<CString> {
    value.map(|v| CString::new(v).unwrap())
}

impl TerraString {
    pub fn as_string(&self) -> String {
        if self.data.is_null() || self.len == 0 {
            return String::new();
        }
        let bytes = unsafe { std::slice::from_raw_parts(self.data as *const u8, self.len as usize) };
        String::from_utf8_lossy(bytes).into_owned()
    }
}

pub fn state_name(state: i32) -> &'static str {
    match state {
        TERRA_STATE_WAITING => "WAITING",
        TERRA_STATE_HOST_SCANNING => "HOST_SCANNING",
        TERRA_STATE_HOST_STARTING => "HOST_STARTING",
        TERRA_STATE_HOST_OK => "HOST_OK",
        TERRA_STATE_GUEST_CONNECTING => "GUEST_CONNECTING",
        TERRA_STATE_GUEST_STARTING => "GUEST_STARTING",
        TERRA_STATE_GUEST_OK => "GUEST_OK",
        TERRA_STATE_EXCEPTION => "EXCEPTION",
        other => {
            println!("UNKNOWN_STATE({other})");
            "UNKNOWN"
        }
    }
}

pub fn event_name(r#type: i32) -> &'static str {
    match r#type {
        TERRA_EVENT_STATE_CHANGED => "STATE_CHANGED",
        TERRA_EVENT_PLAYER_JOINED => "PLAYER_JOINED",
        TERRA_EVENT_PLAYER_LEFT => "PLAYER_LEFT",
        TERRA_EVENT_CONNECTION_DIFFICULTY => "CONNECTION_DIFFICULTY",
        TERRA_EVENT_HOST_READY => "HOST_READY",
        TERRA_EVENT_GUEST_READY => "GUEST_READY",
        TERRA_EVENT_ERROR => "ERROR",
        other => {
            println!("UNKNOWN_EVENT({other})");
            "UNKNOWN"
        }
    }
}
