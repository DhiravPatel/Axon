//! Stage 36 scope invariants — mechanical guards against accidental
//! Stage 37/38 work creeping in.
//!
//! Stage 36 ships an async *boundary* (Interpreter::run_async) plus one
//! new surface primitive (`parallel { ask, ask }`). The interior of
//! eval.rs stays synchronous, and Interpreter stays `!Send`. If a future
//! contributor adds `async fn eval_*` or makes Interpreter Send-safe,
//! they've crossed into Stage 37 territory and these tests fire.

#[test]
fn no_async_fn_eval_methods_in_eval_rs() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/eval.rs"
    ))
    .expect("read eval.rs");
    let count = src.matches("async fn eval_").count();
    assert_eq!(
        count, 0,
        "Stage 36 invariant violated: found {count} occurrences of \
         `async fn eval_` in eval.rs. The async eval-interior migration \
         is Stage 37 work; Stage 36 keeps the entire interior synchronous \
         and only wraps run_main with a `block_on` shell."
    );
}

#[test]
fn no_async_fn_call_value_in_eval_rs() {
    // call_value_async exists as a wrapper that block_on's; the inner
    // call_value must stay synchronous so Interpreter doesn't accidentally
    // become Send.
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/eval.rs"
    ))
    .expect("read eval.rs");
    let inner = src.matches("    async fn call_value(").count();
    assert_eq!(
        inner, 0,
        "Stage 36 invariant violated: call_value must stay `fn`, not `async fn`. \
         The async sibling is `call_value_async` which block_on's; the inner is sync."
    );
}

#[test]
fn one_tokio_runtime_singleton_in_workspace() {
    // The async_rt module is the canonical home; nobody else may build a
    // `Builder::new_multi_thread` runtime in non-test, non-axon-async code,
    // or we re-introduce the "Cannot start a runtime from within a runtime"
    // hazard the singleton was designed to prevent.
    //
    // axon-async (the substrate crate) and axon-runtime::async_rt are the
    // only allowed sites. Everywhere else delegates to async_rt::runtime().
    let mut allowed_sites = 0usize;
    let mut other_sites: Vec<String> = Vec::new();
    let workspace_root = workspace_root_for_test();
    walk_rust_sources(&workspace_root, &mut |path, content| {
        // Skip target/, tests/, and #[cfg(test)] blocks (best-effort: we
        // only flag occurrences that aren't visibly under cfg(test)).
        let rel = path
            .strip_prefix(&workspace_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        if rel.contains("/target/") || rel.contains("/tests/") {
            return;
        }
        for (_, line) in content.lines().enumerate() {
            if !line.contains("Builder::new_multi_thread") {
                continue;
            }
            // Whitelist: axon-async crate (substrate) and axon-runtime::async_rt.
            if rel.contains("crates/axon-async/")
                || rel.contains("crates/axon-runtime/src/async_rt.rs")
            {
                allowed_sites += 1;
                continue;
            }
            other_sites.push(format!("{rel}: {}", line.trim()));
        }
    });
    assert!(
        other_sites.is_empty(),
        "Stage 36 invariant violated: \
         `Builder::new_multi_thread` must only appear in \
         crates/axon-runtime/src/async_rt.rs (the singleton) or in \
         crates/axon-async/ (the Stage 37/38 substrate). Extra sites: {other_sites:#?}"
    );
    assert!(
        allowed_sites >= 1,
        "Stage 36 invariant violated: the async_rt singleton site is missing"
    );
}

// ----------- tiny workspace walker for the invariant grep -----------

fn workspace_root_for_test() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR is crates/axon-runtime; the workspace is two parents up.
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn walk_rust_sources(
    root: &std::path::Path,
    visit: &mut dyn FnMut(&std::path::Path, &str),
) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for ent in entries.flatten() {
            let path = ent.path();
            let name = ent.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                // Skip hidden + build dirs.
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }
                stack.push(path);
            } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
                if let Ok(s) = std::fs::read_to_string(&path) {
                    visit(&path, &s);
                }
            }
        }
    }
}
