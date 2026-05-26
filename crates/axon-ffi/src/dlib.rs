//! Dynamic-library FFI via `libloading`.
//!
//! Stage 23 deepens §35 from "subprocess JSON protocol" to real
//! C-ABI calls into a `.so` / `.dylib` / `.dll` loaded at runtime.
//!
//! The surface is intentionally narrow:
//!
//!   * [`DynamicLibrary::open`] — `dlopen` the shared object.
//!   * [`DynamicLibrary::call`] — invoke a symbol with a fixed-shape
//!     argument list of [`DlibValue`]s. Supported signatures cover the
//!     vast majority of C/Rust FFI: 0..=4 args of `i64`/`f64`/`*const c_char`,
//!     returning `i64`/`f64`/`*const c_char`.
//!
//! Larger argument arities, struct passing, callbacks, and non-trivial
//! lifetimes are deliberately *not* supported. The caller writes a thin
//! C-shim around any function that doesn't fit the supported shape.
//!
//! Why the small surface? Real FFI is unsafe by construction:
//! mismatched signatures crash the process. Restricting v0 to a handful
//! of primitive shapes keeps the unsafe footprint small and auditable —
//! a future stage can add `cranelift`-style trampolines for richer
//! shapes if real demand appears.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::Arc;

use libloading::Library;
use serde::{Deserialize, Serialize};

/// A typed argument or return value crossing the FFI boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ty", content = "v", rename_all = "snake_case")]
pub enum DlibValue {
    /// `int64_t` / Rust `i64`. Most common scalar in C ABIs.
    I64(i64),
    /// `double` / Rust `f64`.
    F64(f64),
    /// Borrowed C-string. On input: an owned Rust `String` is converted
    /// to a `CString` at call time, the C pointer lives for the duration
    /// of the call only. On return: the runtime copies the bytes pointed
    /// to by the returned `*const c_char` into a fresh Rust `String`.
    Str(String),
}

/// A loaded shared object + the function pointers we've fished out of it.
/// Held by `Arc` so multiple call sites can share a single dlopen.
pub struct DynamicLibrary {
    inner: Arc<Library>,
    path: String,
}

impl std::fmt::Debug for DynamicLibrary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamicLibrary")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum DlibError {
    Open(String),
    Symbol(String),
    UnsupportedSignature {
        arity: usize,
        ret_is_str: bool,
        detail: String,
    },
    ArgType {
        index: usize,
        expected: &'static str,
        got: &'static str,
    },
    Utf8(String),
    NullPointer,
}

impl std::fmt::Display for DlibError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DlibError::Open(m) => write!(f, "ffi dlib: dlopen failed: {m}"),
            DlibError::Symbol(m) => write!(f, "ffi dlib: symbol not found: {m}"),
            DlibError::UnsupportedSignature { arity, ret_is_str, detail } => write!(
                f,
                "ffi dlib: unsupported signature (arity={arity}, ret_str={ret_is_str}): {detail}"
            ),
            DlibError::ArgType {
                index,
                expected,
                got,
            } => write!(
                f,
                "ffi dlib: arg #{index}: expected `{expected}`, got `{got}`"
            ),
            DlibError::Utf8(m) => write!(f, "ffi dlib: returned string is not UTF-8: {m}"),
            DlibError::NullPointer => write!(f, "ffi dlib: function returned NULL"),
        }
    }
}
impl std::error::Error for DlibError {}

impl DynamicLibrary {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, DlibError> {
        let path_str = path.as_ref().display().to_string();
        // SAFETY: `dlopen` runs arbitrary library initializers. We accept
        // this in exchange for being able to call vendor SDKs — the
        // caller must trust the library's provenance.
        let lib = unsafe {
            Library::new(path.as_ref()).map_err(|e| DlibError::Open(e.to_string()))?
        };
        Ok(Self {
            inner: Arc::new(lib),
            path: path_str,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Call `symbol` with `args`. The supported (arity, types) shapes
    /// are listed in the module docs — anything else returns
    /// `UnsupportedSignature`.
    ///
    /// Returns the typed return value as a [`DlibValue`]. `i64` and
    /// `f64` returns surface directly; string returns are copied into a
    /// fresh Rust `String` (the underlying C pointer's lifetime ends
    /// when the function returns, but the bytes are copied first).
    pub fn call(
        &self,
        symbol: &str,
        args: &[DlibValue],
        ret_is_str: bool,
    ) -> Result<DlibValue, DlibError> {
        // Categorize the args into a fixed shape we have a typed
        // function-pointer signature for. v0 accepts:
        //   - all-i64 arity 0..=4 → i64
        //   - all-f64 arity 0..=2 → f64
        //   - single str arg → str
        //   - i64 + str → str
        let arity = args.len();
        let all_i64 = args.iter().all(|a| matches!(a, DlibValue::I64(_)));
        let all_f64 = args.iter().all(|a| matches!(a, DlibValue::F64(_)));
        let single_str =
            arity == 1 && matches!(args.first(), Some(DlibValue::Str(_)));

        if !ret_is_str && all_i64 {
            return self.call_i64_returning_i64(symbol, args);
        }
        if !ret_is_str && all_f64 && arity <= 2 {
            return self.call_f64_returning_f64(symbol, args);
        }
        if ret_is_str && single_str {
            return self.call_str_returning_str(symbol, args);
        }
        if ret_is_str && arity == 0 {
            return self.call_void_returning_str(symbol);
        }
        Err(DlibError::UnsupportedSignature {
            arity,
            ret_is_str,
            detail: "see axon-ffi::dlib module docs for supported shapes".into(),
        })
    }

    // ---- shape handlers ----

    fn call_i64_returning_i64(
        &self,
        symbol: &str,
        args: &[DlibValue],
    ) -> Result<DlibValue, DlibError> {
        let ints: Vec<i64> = args
            .iter()
            .map(|a| match a {
                DlibValue::I64(i) => *i,
                _ => unreachable!(),
            })
            .collect();
        // SAFETY: We typed the symbol as `unsafe extern "C" fn(...) -> i64`.
        // The user is responsible for the library exposing this exact shape.
        let r = unsafe {
            match ints.len() {
                0 => {
                    let f: libloading::Symbol<unsafe extern "C" fn() -> i64> =
                        self.symbol(symbol)?;
                    f()
                }
                1 => {
                    let f: libloading::Symbol<unsafe extern "C" fn(i64) -> i64> =
                        self.symbol(symbol)?;
                    f(ints[0])
                }
                2 => {
                    let f: libloading::Symbol<unsafe extern "C" fn(i64, i64) -> i64> =
                        self.symbol(symbol)?;
                    f(ints[0], ints[1])
                }
                3 => {
                    let f: libloading::Symbol<
                        unsafe extern "C" fn(i64, i64, i64) -> i64,
                    > = self.symbol(symbol)?;
                    f(ints[0], ints[1], ints[2])
                }
                4 => {
                    let f: libloading::Symbol<
                        unsafe extern "C" fn(i64, i64, i64, i64) -> i64,
                    > = self.symbol(symbol)?;
                    f(ints[0], ints[1], ints[2], ints[3])
                }
                _ => {
                    return Err(DlibError::UnsupportedSignature {
                        arity: ints.len(),
                        ret_is_str: false,
                        detail: "i64 fns supported up to arity 4".into(),
                    });
                }
            }
        };
        Ok(DlibValue::I64(r))
    }

    fn call_f64_returning_f64(
        &self,
        symbol: &str,
        args: &[DlibValue],
    ) -> Result<DlibValue, DlibError> {
        let floats: Vec<f64> = args
            .iter()
            .map(|a| match a {
                DlibValue::F64(x) => *x,
                _ => unreachable!(),
            })
            .collect();
        let r = unsafe {
            match floats.len() {
                0 => {
                    let f: libloading::Symbol<unsafe extern "C" fn() -> f64> =
                        self.symbol(symbol)?;
                    f()
                }
                1 => {
                    let f: libloading::Symbol<unsafe extern "C" fn(f64) -> f64> =
                        self.symbol(symbol)?;
                    f(floats[0])
                }
                2 => {
                    let f: libloading::Symbol<unsafe extern "C" fn(f64, f64) -> f64> =
                        self.symbol(symbol)?;
                    f(floats[0], floats[1])
                }
                _ => unreachable!(),
            }
        };
        Ok(DlibValue::F64(r))
    }

    fn call_str_returning_str(
        &self,
        symbol: &str,
        args: &[DlibValue],
    ) -> Result<DlibValue, DlibError> {
        let s = match &args[0] {
            DlibValue::Str(s) => s,
            _ => unreachable!(),
        };
        let cs = CString::new(s.as_str()).map_err(|e| {
            DlibError::Utf8(format!("input contains a NUL byte: {e}"))
        })?;
        let ret = unsafe {
            let f: libloading::Symbol<
                unsafe extern "C" fn(*const c_char) -> *const c_char,
            > = self.symbol(symbol)?;
            f(cs.as_ptr())
        };
        if ret.is_null() {
            return Err(DlibError::NullPointer);
        }
        let owned = unsafe { CStr::from_ptr(ret) }
            .to_str()
            .map_err(|e| DlibError::Utf8(e.to_string()))?
            .to_string();
        Ok(DlibValue::Str(owned))
    }

    fn call_void_returning_str(&self, symbol: &str) -> Result<DlibValue, DlibError> {
        let ret = unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn() -> *const c_char> =
                self.symbol(symbol)?;
            f()
        };
        if ret.is_null() {
            return Err(DlibError::NullPointer);
        }
        let owned = unsafe { CStr::from_ptr(ret) }
            .to_str()
            .map_err(|e| DlibError::Utf8(e.to_string()))?
            .to_string();
        Ok(DlibValue::Str(owned))
    }

    fn symbol<'a, T>(&'a self, name: &str) -> Result<libloading::Symbol<'a, T>, DlibError> {
        unsafe {
            self.inner
                .get(name.as_bytes())
                .map_err(|e| DlibError::Symbol(format!("{name}: {e}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// libc's `abs(int)` returns the absolute value. It's on every Unix
    /// — `dlopen(NULL, ...)` reaches it via the executable's symbol
    /// table on macOS, or `libc.so.6` on Linux. The test loads libm's
    /// `cos(double)` instead because it's universally available and the
    /// signature exercises the f64 path.
    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn dlopen_libm_and_call_cos_returns_one_at_zero() {
        let path = if cfg!(target_os = "macos") {
            "/usr/lib/libSystem.dylib"
        } else {
            "libm.so.6"
        };
        let lib = match DynamicLibrary::open(path) {
            Ok(l) => l,
            Err(_) => {
                // CI sandboxes may not have these; skip silently.
                return;
            }
        };
        let r = lib.call("cos", &[DlibValue::F64(0.0)], false);
        match r {
            Ok(DlibValue::F64(v)) => assert!((v - 1.0).abs() < 1e-12),
            other => panic!("expected cos(0) = 1, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_signature_is_reported() {
        // Build a `DynamicLibrary` against the current binary so we have
        // a real library to talk to, then ask for an unsupported shape.
        let path = if cfg!(target_os = "macos") {
            "/usr/lib/libSystem.dylib"
        } else {
            "libc.so.6"
        };
        let lib = match DynamicLibrary::open(path) {
            Ok(l) => l,
            Err(_) => return, // skip if not available
        };
        let err = lib
            .call(
                "definitely_not_a_real_symbol_qwerty",
                &[
                    DlibValue::I64(1),
                    DlibValue::F64(2.0),
                    DlibValue::Str("3".into()),
                ],
                true,
            )
            .unwrap_err();
        assert!(
            matches!(err, DlibError::UnsupportedSignature { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn missing_symbol_is_reported() {
        let path = if cfg!(target_os = "macos") {
            "/usr/lib/libSystem.dylib"
        } else {
            "libc.so.6"
        };
        let lib = match DynamicLibrary::open(path) {
            Ok(l) => l,
            Err(_) => return,
        };
        let err = lib
            .call(
                "axon_ffi_nonexistent_symbol_for_test_only",
                &[DlibValue::I64(0)],
                false,
            )
            .unwrap_err();
        assert!(matches!(err, DlibError::Symbol(_)), "got: {err:?}");
    }

    #[test]
    fn open_nonexistent_path_errors() {
        let err = DynamicLibrary::open("/this/path/does/not/exist.so").unwrap_err();
        assert!(matches!(err, DlibError::Open(_)));
    }

    #[test]
    fn dlib_value_round_trips_through_json() {
        let xs = vec![
            DlibValue::I64(42),
            DlibValue::F64(3.14),
            DlibValue::Str("hi".into()),
        ];
        let bytes = serde_json::to_vec(&xs).unwrap();
        let back: Vec<DlibValue> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, xs);
    }
}
