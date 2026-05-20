/*
 * axvm.h — embed the Axon VM in C/C++/any-FFI host application.
 *
 * Pair this header with `libaxon_vm.{a,so,dylib}` produced by
 * `cargo build -p axon-vm-cabi --release`. The library exposes the
 * minimum surface to compile an Axon program from a UTF-8 source
 * string, call its `main()`, and read back the JSON-encoded return
 * value.
 *
 * Threading: each thread that calls `axvm_*` owns its own
 * `axvm_last_error` slot. Handles (AxvmHandle*) are *not* thread-safe;
 * if you need concurrent execution, compile a handle per thread.
 *
 * Memory rule: every char* this library *returns* must be freed with
 * `axvm_free_string`. Handles must be freed with `axvm_drop`. Calling
 * either free on NULL is a no-op.
 */

#ifndef AXVM_H
#define AXVM_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---------- ABI version ---------- */
uint32_t axvm_abi_version(void);

/* ---------- Opaque handle ---------- */
typedef struct AxvmHandle AxvmHandle;

/* ---------- Compile, run, drop ---------- */

/*
 * Compile a UTF-8 Axon source string into an executable AxvmHandle.
 * Returns NULL on failure; call axvm_last_error() to inspect the
 * reason. The returned handle starts with the default capability set
 * (Console, Net, Fs.Read, Fs.Write, Time, Random, LLM, Memory,
 * Spawn). Use axvm_set_caps to narrow before calling axvm_call_main.
 */
AxvmHandle* axvm_compile(const char* source_utf8);

/*
 * Replace the handle's capability set. Pass a comma-separated list
 * ("Console,Net") or NULL for the empty (isolated) set. Returns 0 on
 * success, non-zero on error (see axvm_last_error).
 */
int axvm_set_caps(AxvmHandle* handle, const char* caps_csv);

/*
 * Call main() on the handle. On success, returns 0 and writes a freshly
 * allocated, null-terminated JSON-encoded result into *out_json (the
 * caller must axvm_free_string it). On failure, returns non-zero and
 * leaves *out_json unchanged.
 *
 * Unit (the implicit return of a body with no value) maps to "null".
 */
int axvm_call_main(AxvmHandle* handle, char** out_json);

/*
 * Free a handle. Safe to call with NULL.
 */
void axvm_drop(AxvmHandle* handle);

/*
 * Free a string returned by the library (axvm_call_main /
 * axvm_last_error). Safe to call with NULL.
 */
void axvm_free_string(char* s);

/*
 * Return the last error message set on the calling thread, or NULL
 * if no error is recorded. The caller must free with axvm_free_string.
 */
char* axvm_last_error(void);

#ifdef __cplusplus
}
#endif

#endif /* AXVM_H */
