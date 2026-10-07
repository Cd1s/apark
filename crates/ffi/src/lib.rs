//! C ABI over [`apark_core::rpc`]; see `include/apark.h`.

use std::ffi::{c_char, CStr, CString};
use std::sync::{Arc, Mutex, OnceLock};

use apark_core::rpc::Rpc;
use apark_core::Engine;
use serde_json::json;

type EventCb = extern "C" fn(*const c_char);

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static RPC: OnceLock<Result<Arc<Rpc>, String>> = OnceLock::new();
static CALLBACK: Mutex<Option<EventCb>> = Mutex::new(None);

fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

fn emit(event: serde_json::Value) {
    let cb = *CALLBACK.lock().unwrap_or_else(|e| e.into_inner());
    if let (Some(cb), Ok(s)) = (cb, CString::new(event.to_string())) {
        cb(s.as_ptr());
    }
}

fn rpc() -> Result<&'static Arc<Rpc>, String> {
    RPC.get_or_init(|| {
        // Engine::open builds a reqwest client, which needs to run inside the runtime.
        let _guard = runtime().enter();
        Engine::open().map(|eng| Rpc::new(eng, Arc::new(emit))).map_err(|e| format!("{e:#}"))
    })
    .as_ref()
    .map_err(Clone::clone)
}

fn into_c(s: String) -> *mut c_char {
    CString::new(s.replace('\0', "")).unwrap_or_default().into_raw()
}

/// # Safety
/// `request` must be a valid NUL-terminated UTF-8 string. Free the result with `apark_free`.
#[no_mangle]
pub unsafe extern "C" fn apark_call(request: *const c_char) -> *mut c_char {
    if request.is_null() {
        return into_c(json!({ "ok": false, "error": "null request" }).to_string());
    }
    let req = CStr::from_ptr(request).to_string_lossy().into_owned();
    let reply = match rpc() {
        Ok(rpc) => runtime().block_on(rpc.handle(&req)),
        Err(e) => json!({ "ok": false, "error": e }).to_string(),
    };
    into_c(reply)
}

/// # Safety
/// `s` must come from `apark_call` (or be null) and be freed only once.
#[no_mangle]
pub unsafe extern "C" fn apark_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

#[no_mangle]
pub extern "C" fn apark_set_event_callback(cb: Option<EventCb>) {
    *CALLBACK.lock().unwrap_or_else(|e| e.into_inner()) = cb;
}
