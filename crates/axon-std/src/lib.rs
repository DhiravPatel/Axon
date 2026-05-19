//! Axon standard library — Stage 11.
//!
//! Eight core modules registered as flat-named native functions:
//!
//!   * `std.string` → `str_*`   (case, trim, split, join, parse, contains, ...)
//!   * `std.list`   → `list_*`  (push, pop, len, get, set, map, filter, fold, ...)
//!   * `std.map`    → `map_*`   (new, get, set, remove, contains, keys, values)
//!   * `std.set`    → `set_*`   (new, add, contains, union, intersection, diff)
//!   * `std.option` → `opt_*`   (some, none, is_some, is_none, unwrap_or, map)
//!   * `std.result` → `result_*` (ok, err, is_ok, is_err, unwrap_or, map)
//!   * `std.math`   → `math_*`  (pow, sqrt, floor, ceil, round, sin, cos, log,
//!                               exp, pi, e)
//!   * `std.time`   → `time_*`  (day_of_year, weekday, duration_seconds,
//!                               duration_millis, ...)  (the `Time` capability
//!                               is enforced for *clock-reading* primitives
//!                               which live in axon-runtime's built-ins.)
//!
//! The runtime is unmodified — registration uses the same `NativeFn` shape
//! and gets installed through [`Interpreter::register_native`].

use axon_runtime::{Interpreter, NativeFn};

mod list;
mod map;
mod math;
mod option;
mod result;
mod set;
mod string;
mod time;

/// Register every stdlib function on `interp`.
///
/// Idempotent — re-registering the same name overwrites the previous binding,
/// matching how the runtime's own built-ins behave.
pub fn register_all(interp: &Interpreter) {
    let mut reg = |name: &'static str, native: NativeFn| {
        interp.register_native(name, native);
    };
    string::register(&mut reg);
    list::register(&mut reg);
    map::register(&mut reg);
    set::register(&mut reg);
    option::register(&mut reg);
    result::register(&mut reg);
    math::register(&mut reg);
    time::register(&mut reg);
}

/// Total number of native functions registered by [`register_all`].
///
/// Tests and `axon doc std` use this to verify nothing slipped out.
pub const FUNCTION_COUNT: usize = string::COUNT
    + list::COUNT
    + map::COUNT
    + set::COUNT
    + option::COUNT
    + result::COUNT
    + math::COUNT
    + time::COUNT;
