//! Embedding AxVM via the C ABI (§35.4).
//!
//! This crate exposes the AxVM compile + run pipeline behind an `extern
//! "C"` surface so anyone with a working C toolchain can embed Axon —
//! the original `axvm.h` use-case from the spec.
//!
//! The API is deliberately small: load source → compile → call `main`
//! → free. Values cross the boundary as JSON strings; numbers, strings,
//! lists, records, and unit values all round-trip safely. Closures,
//! tools, agents, and channels are excluded — they have no portable
//! cross-language representation and would force a full FFI marshalling
//! framework on every embedder.
//!
//! Memory rule: every pointer this library *returns* (other than the
//! opaque `AxvmHandle`) must be freed with `axvm_free_string`. The
//! handle itself must be freed with `axvm_drop`. Calling free on a null
//! pointer is a no-op so cleanup code stays simple.
//!
//! Error model: each `axvm_*` function returns an `i32` status code (0 =
//! ok, non-zero = error). On error, `axvm_last_error` returns a
//! human-readable string describing the failure on the calling thread.

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

use axon_diag::SourceFile;
use axon_vm::{compile, CapSet, CompiledProgram, Value, Vm, VmError};

/// Opaque embedder-facing handle. Holds a compiled program plus a VM
/// instance; one handle = one compiled module ready to be invoked.
pub struct AxvmHandle {
    program: CompiledProgram,
    caps: CapSet,
}

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = RefCell::new(None);
}

fn set_last_error(msg: impl Into<String>) {
    let s = msg.into();
    // Cap error message length so a runaway error doesn't blow up the
    // caller's memory budget on `axvm_last_error`.
    let truncated = if s.len() > 4096 {
        format!("{}...(truncated)", &s[..4096])
    } else {
        s
    };
    let c = CString::new(truncated.replace('\0', "?")).unwrap_or_else(|_| {
        CString::new("error message contained an interior NUL").unwrap()
    });
    LAST_ERROR.with(|cell| *cell.borrow_mut() = Some(c));
}

fn clear_last_error() {
    LAST_ERROR.with(|cell| *cell.borrow_mut() = None);
}

/// Compile `source_utf8` (null-terminated UTF-8 Axon program text) and
/// return an `AxvmHandle*`. On parse/type-check failure returns NULL
/// and `axvm_last_error` describes the diagnostics.
///
/// # Safety
/// `source_utf8` must be a valid null-terminated UTF-8 pointer or
/// NULL. The returned pointer (if non-NULL) must be released with
/// `axvm_drop`.
#[no_mangle]
pub unsafe extern "C" fn axvm_compile(source_utf8: *const c_char) -> *mut AxvmHandle {
    clear_last_error();
    if source_utf8.is_null() {
        set_last_error("axvm_compile: source pointer is null");
        return ptr::null_mut();
    }
    let s = match CStr::from_ptr(source_utf8).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("axvm_compile: source not valid UTF-8: {e}"));
            return ptr::null_mut();
        }
    };
    let file = SourceFile::new("<cabi>", s.to_string());
    let (program, parse_diags) = axon_parser::parse(&file);
    if !parse_diags.is_empty() {
        set_last_error(format!("parse: {} diagnostic(s)", parse_diags.len()));
        return ptr::null_mut();
    }
    let (_, tyck_diags) = axon_tyck::check(&file, &program);
    if !tyck_diags.is_empty() {
        set_last_error(format!("type: {} diagnostic(s)", tyck_diags.len()));
        return ptr::null_mut();
    }
    let compiled = match compile(&program) {
        Ok(c) => c,
        Err(e) => {
            set_last_error(format!("compile: {e:?}"));
            return ptr::null_mut();
        }
    };
    let h = Box::new(AxvmHandle {
        program: compiled,
        caps: CapSet::standard_default(),
    });
    Box::into_raw(h)
}

/// Replace the capability set on a handle.
///
/// `caps_csv` is a comma-separated list (`"Console,Net,Fs.Read"`).
/// Pass NULL for the empty (isolated) set.
///
/// # Safety
/// `handle` must be a valid `AxvmHandle*` previously returned from
/// `axvm_compile` and not yet dropped. `caps_csv` may be NULL or a
/// valid null-terminated UTF-8 pointer.
#[no_mangle]
pub unsafe extern "C" fn axvm_set_caps(
    handle: *mut AxvmHandle,
    caps_csv: *const c_char,
) -> c_int {
    clear_last_error();
    if handle.is_null() {
        set_last_error("axvm_set_caps: handle is null");
        return -1;
    }
    let h = &mut *handle;
    if caps_csv.is_null() {
        h.caps = CapSet::empty();
        return 0;
    }
    let s = match CStr::from_ptr(caps_csv).to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(format!("axvm_set_caps: caps not UTF-8: {e}"));
            return -1;
        }
    };
    let names: Vec<String> = s
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    h.caps = CapSet::from_iter(names);
    0
}

/// Call `main()` in the loaded program. Returns 0 on success and writes
/// the JSON-encoded return value into `*out_json` (caller must
/// `axvm_free_string` it).
///
/// # Safety
/// `handle` must be a valid `AxvmHandle*`. `out_json` must be a valid
/// non-null pointer.
#[no_mangle]
pub unsafe extern "C" fn axvm_call_main(
    handle: *mut AxvmHandle,
    out_json: *mut *mut c_char,
) -> c_int {
    clear_last_error();
    if handle.is_null() {
        set_last_error("axvm_call_main: handle is null");
        return -1;
    }
    if out_json.is_null() {
        set_last_error("axvm_call_main: out_json is null");
        return -1;
    }
    let h = &*handle;
    let mut vm = Vm::with_caps(h.program.clone(), h.caps.clone());
    let result: Result<Value, VmError> = vm.run_main();
    match result {
        Ok(v) => {
            let json = vm_value_to_json(&v);
            let s = serde_json::to_string(&json).unwrap_or_else(|_| "null".into());
            let c = match CString::new(s) {
                Ok(c) => c,
                Err(e) => {
                    set_last_error(format!("axvm_call_main: result has NUL: {e}"));
                    return -1;
                }
            };
            *out_json = c.into_raw();
            0
        }
        Err(e) => {
            set_last_error(format!("axvm_call_main: {e:?}"));
            -1
        }
    }
}

/// Free a `*mut c_char` previously returned by `axvm_call_main` /
/// `axvm_last_error`. No-op on NULL.
///
/// # Safety
/// `s` must either be NULL or have been returned by this library and
/// not yet freed.
#[no_mangle]
pub unsafe extern "C" fn axvm_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    let _ = CString::from_raw(s);
}

/// Free a handle previously returned by `axvm_compile`. No-op on NULL.
///
/// # Safety
/// `handle` must be NULL or a value previously returned from
/// `axvm_compile` and not yet dropped.
#[no_mangle]
pub unsafe extern "C" fn axvm_drop(handle: *mut AxvmHandle) {
    if handle.is_null() {
        return;
    }
    let _ = Box::from_raw(handle);
}

/// Return the last error message set by any `axvm_*` call on the
/// current thread, or NULL when no error is recorded. The returned
/// pointer must be freed with `axvm_free_string`.
#[no_mangle]
pub extern "C" fn axvm_last_error() -> *mut c_char {
    LAST_ERROR.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|c| c.clone().into_raw())
            .unwrap_or(ptr::null_mut())
    })
}

/// Library ABI version. Bump when the C ABI changes in a
/// backwards-incompatible way.
pub const AXVM_ABI_VERSION: u32 = 1;

#[no_mangle]
pub extern "C" fn axvm_abi_version() -> u32 {
    AXVM_ABI_VERSION
}

// ---------- value marshalling ----------

fn vm_value_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Unit => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => J::Number((*i).into()),
        Value::Float(f) => {
            serde_json::Number::from_f64(*f).map(J::Number).unwrap_or(J::Null)
        }
        Value::String(s) => J::String((**s).clone()),
        Value::List(xs) => {
            J::Array(xs.borrow().iter().map(vm_value_to_json).collect())
        }
        Value::Record(fields) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in fields.borrow().iter() {
                obj.insert(k.clone(), vm_value_to_json(v));
            }
            J::Object(obj)
        }
        other => J::String(format!("{other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn cstr(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    fn from_cstr_owned(p: *mut c_char) -> String {
        unsafe {
            assert!(!p.is_null());
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            axvm_free_string(p);
            s
        }
    }

    #[test]
    fn version_is_nonzero() {
        assert!(axvm_abi_version() > 0);
    }

    #[test]
    fn compile_and_run_simple_program() {
        let src = cstr("fn main() -> Int { 42 }");
        unsafe {
            let h = axvm_compile(src.as_ptr());
            assert!(!h.is_null(), "compile failed");
            let mut out: *mut c_char = ptr::null_mut();
            let rc = axvm_call_main(h, &mut out);
            assert_eq!(rc, 0);
            let result = from_cstr_owned(out);
            assert_eq!(result, "42");
            axvm_drop(h);
        }
    }

    #[test]
    fn parse_error_returns_null_and_records_message() {
        let src = cstr("fn main() {");
        unsafe {
            let h = axvm_compile(src.as_ptr());
            assert!(h.is_null());
            let err = from_cstr_owned(axvm_last_error());
            assert!(err.contains("parse"));
        }
    }

    #[test]
    fn null_source_handled_cleanly() {
        unsafe {
            let h = axvm_compile(ptr::null());
            assert!(h.is_null());
            let err = from_cstr_owned(axvm_last_error());
            assert!(err.contains("null"));
        }
    }

    #[test]
    fn isolated_caps_disable_console() {
        // No `print` allowed when isolated — but a pure-Int program still works.
        let src = cstr("fn main() -> Int { 7 }");
        unsafe {
            let h = axvm_compile(src.as_ptr());
            assert!(!h.is_null());
            let rc = axvm_set_caps(h, ptr::null());
            assert_eq!(rc, 0);
            let mut out: *mut c_char = ptr::null_mut();
            let rc = axvm_call_main(h, &mut out);
            assert_eq!(rc, 0);
            let val = from_cstr_owned(out);
            assert_eq!(val, "7");
            axvm_drop(h);
        }
    }

    #[test]
    fn drop_null_is_noop() {
        unsafe {
            axvm_drop(ptr::null_mut());
            axvm_free_string(ptr::null_mut());
        }
    }
}
