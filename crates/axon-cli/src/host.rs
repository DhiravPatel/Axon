//! Host integration — installs `axon-std` + `axon-memory` on an interpreter.
//!
//! The runtime stays oblivious to which crates extend it; the CLI is where
//! "Axon the language" meets "Axon the standard distribution". Programs run
//! through `axon test` or `axon run` automatically get the same set of
//! native bindings.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use axon_memory::{EphemeralStore, Entry, FileStore, Store};
use axon_runtime::{Interpreter, NativeExtFn, NativeFn, Value};

/// Install every host-provided native binding on `interp`.
pub fn install(interp: &Interpreter) {
    axon_std::register_all(interp);
    install_extern_bridge(interp);
    install_memory(interp);
    install_rag(interp);
    install_media(interp);
    install_flow(interp);
    install_trigger(interp);
    install_skill(interp);
    install_a2a(interp);
    install_guard(interp);
    install_secret(interp);
    install_sandbox(interp);
    install_eval(interp);
    install_cost(interp);
    install_ffi(interp);
    install_env(interp);
    install_deploy(interp);
    install_supervisor(interp);
    install_migrate(interp);
    install_otlp(interp);
    install_stage24(interp);
    install_stage25(interp);
    install_stage26(interp);
    install_stage27(interp);
    install_stage28(interp);
    install_stage29(interp);
    install_stage31(interp);
    install_stage32(interp);
}

// ---------------------------------------------------------------------------
// `mem_*` bindings
//
// A single mutable slot ("the default memory") lives in the host. `mem_open`
// re-points it at either an in-memory or file-backed store; all subsequent
// `mem_*` calls hit whichever is current. This gives programs a single
// stable identifier surface; richer per-handle APIs land when the
// `memory <name> = ...` declaration ships.
// ---------------------------------------------------------------------------

thread_local! {
    static MEM: RefCell<Arc<dyn Store>> =
        RefCell::new(Arc::new(EphemeralStore::new()) as Arc<dyn Store>);
}

fn current_store() -> Arc<dyn Store> {
    MEM.with(|cell| cell.borrow().clone())
}

fn set_store(new: Arc<dyn Store>) {
    MEM.with(|cell| *cell.borrow_mut() = new);
}

fn install_memory(interp: &Interpreter) {
    interp.register_native("mem_open_file", n("mem_open_file", 1, Some(1), mem_open_file));
    interp.register_native(
        "mem_open_ephemeral",
        n("mem_open_ephemeral", 0, Some(0), mem_open_ephemeral),
    );
    interp.register_native("mem_set", n("mem_set", 2, Some(2), mem_set));
    interp.register_native("mem_get", n("mem_get", 1, Some(1), mem_get));
    interp.register_native("mem_remove", n("mem_remove", 1, Some(1), mem_remove));
    interp.register_native("mem_keys", n("mem_keys", 0, Some(0), mem_keys));
    interp.register_native("mem_len", n("mem_len", 0, Some(0), mem_len));
    interp.register_native(
        "mem_contains",
        n("mem_contains", 1, Some(1), mem_contains),
    );
}

fn n(
    name: &'static str,
    min_arity: usize,
    max_arity: Option<usize>,
    call: fn(&[Value]) -> Result<Value, String>,
) -> NativeFn {
    NativeFn {
        name,
        min_arity,
        max_arity,
        required_caps: &[],
        call,
    }
}

fn s_arg(args: &[Value], idx: usize, fn_name: &str) -> Result<Rc<String>, String> {
    match &args[idx] {
        Value::String(s) => Ok(s.clone()),
        other => Err(format!(
            "`{fn_name}` expected a String at position {idx}, got `{}`",
            other.type_name()
        )),
    }
}

fn value_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Unit | Value::Nil => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::Number((*i).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Char(c) => serde_json::Value::String(c.to_string()),
        Value::String(s) => serde_json::Value::String(s.as_str().to_string()),
        Value::Bytes(b) => serde_json::Value::Array(
            b.iter()
                .map(|byte| serde_json::Value::Number((*byte as i64).into()))
                .collect(),
        ),
        Value::List(l) => {
            serde_json::Value::Array(l.borrow().iter().map(value_to_json).collect())
        }
        Value::Set(s) => {
            serde_json::Value::Array(s.borrow().iter().map(value_to_json).collect())
        }
        Value::Tuple(t) => serde_json::Value::Array(t.iter().map(value_to_json).collect()),
        Value::Map(m) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in m.borrow().iter() {
                let key = match k {
                    Value::String(s) => s.as_str().to_string(),
                    other => format!("{other}"),
                };
                obj.insert(key, value_to_json(v));
            }
            serde_json::Value::Object(obj)
        }
        Value::Record(r) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in r.borrow().iter() {
                obj.insert(k.clone(), value_to_json(v));
            }
            serde_json::Value::Object(obj)
        }
        other => serde_json::Value::String(format!("{other}")),
    }
}

fn json_to_value(j: &serde_json::Value) -> Value {
    match j {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Nil
            }
        }
        serde_json::Value::String(s) => Value::String(Rc::new(s.clone())),
        serde_json::Value::Array(a) => {
            let xs: Vec<Value> = a.iter().map(json_to_value).collect();
            Value::List(Rc::new(std::cell::RefCell::new(xs)))
        }
        serde_json::Value::Object(o) => {
            let entries: Vec<(String, Value)> = o
                .iter()
                .map(|(k, v)| (k.clone(), json_to_value(v)))
                .collect();
            Value::Record(Rc::new(std::cell::RefCell::new(entries)))
        }
    }
}

fn mem_open_file(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "mem_open_file")?;
    let store = FileStore::open(path.as_str()).map_err(|e| e.to_string())?;
    set_store(Arc::new(store));
    Ok(Value::Unit)
}

fn mem_open_ephemeral(_args: &[Value]) -> Result<Value, String> {
    set_store(Arc::new(EphemeralStore::new()));
    Ok(Value::Unit)
}

fn mem_set(args: &[Value]) -> Result<Value, String> {
    let key = s_arg(args, 0, "mem_set")?;
    let value_json = value_to_json(&args[1]);
    let store = current_store();
    store
        .set(key.as_str(), Entry::new(value_json))
        .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn mem_get(args: &[Value]) -> Result<Value, String> {
    let key = s_arg(args, 0, "mem_get")?;
    let store = current_store();
    let entry = store.get(key.as_str()).map_err(|e| e.to_string())?;
    Ok(match entry {
        Some(e) => json_to_value(&e.value),
        None => Value::Nil,
    })
}

fn mem_remove(args: &[Value]) -> Result<Value, String> {
    let key = s_arg(args, 0, "mem_remove")?;
    let store = current_store();
    let existed = store.remove(key.as_str()).map_err(|e| e.to_string())?;
    Ok(Value::Bool(existed))
}

fn mem_keys(_args: &[Value]) -> Result<Value, String> {
    let store = current_store();
    let ks = store.keys().map_err(|e| e.to_string())?;
    let items: Vec<Value> = ks.into_iter().map(|k| Value::String(Rc::new(k))).collect();
    Ok(Value::List(Rc::new(std::cell::RefCell::new(items))))
}

fn mem_len(_args: &[Value]) -> Result<Value, String> {
    let store = current_store();
    let n = store.len().map_err(|e| e.to_string())?;
    Ok(Value::Int(n as i64))
}

fn mem_contains(args: &[Value]) -> Result<Value, String> {
    let key = s_arg(args, 0, "mem_contains")?;
    let store = current_store();
    Ok(Value::Bool(
        store
            .get(key.as_str())
            .map_err(|e| e.to_string())?
            .is_some(),
    ))
}

// Suppress an unused warning when the CLI doesn't reference these directly
// from main.rs in a future build configuration.
#[allow(dead_code)]
pub fn reset_memory_for_tests() {
    set_store(Arc::new(EphemeralStore::new()));
}

// Mutex-guard re-export: integration tests in axon-cli use this to run
// stdlib calls under a coherent memory snapshot.
#[allow(dead_code)]
pub fn with_memory_guard<R>(f: impl FnOnce() -> R) -> R {
    static GUARD: Mutex<()> = Mutex::new(());
    let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

// ---------------------------------------------------------------------------
// `rag_*` bindings  (Stage 12 — §50 retrieval-augmented generation)
//
// Programs work with a single default `Index` held in thread-local storage,
// mirroring the memory model. `rag_index_new(dims)` resets it; `rag_ingest`
// adds chunks; `rag_retrieve` runs hybrid search.
// ---------------------------------------------------------------------------

use axon_rag::{Chunker, HashEmbedder, Index, RecursiveChunker, Retriever};

thread_local! {
    static RAG_INDEX: RefCell<Index> = RefCell::new(Index::new(256));
    static RAG_EMBEDDER: RefCell<HashEmbedder> = RefCell::new(HashEmbedder::new(256));
}

fn install_rag(interp: &Interpreter) {
    interp.register_native("rag_index_new", n("rag_index_new", 1, Some(1), rag_index_new));
    interp.register_native("rag_index_len", n("rag_index_len", 0, Some(0), rag_index_len));
    interp.register_native("rag_chunk", n("rag_chunk", 4, Some(4), rag_chunk));
    interp.register_native("rag_ingest", n("rag_ingest", 2, Some(2), rag_ingest));
    interp.register_native("rag_retrieve", n("rag_retrieve", 2, Some(3), rag_retrieve));
    interp.register_native("rag_save", n("rag_save", 1, Some(1), rag_save));
    interp.register_native("rag_load", n("rag_load", 1, Some(1), rag_load));
}

fn i_arg(args: &[Value], idx: usize, fn_name: &str) -> Result<i64, String> {
    match &args[idx] {
        Value::Int(i) => Ok(*i),
        other => Err(format!(
            "`{fn_name}` expected an Int at position {idx}, got `{}`",
            other.type_name()
        )),
    }
}

fn rag_index_new(args: &[Value]) -> Result<Value, String> {
    let dims = i_arg(args, 0, "rag_index_new")?;
    if !(1..=4096).contains(&dims) {
        return Err(format!("rag_index_new: dims must be in 1..=4096, got {dims}"));
    }
    let d = dims as usize;
    RAG_INDEX.with(|c| *c.borrow_mut() = Index::new(d));
    RAG_EMBEDDER.with(|c| *c.borrow_mut() = HashEmbedder::new(d));
    Ok(Value::Unit)
}

fn rag_index_len(_args: &[Value]) -> Result<Value, String> {
    let n = RAG_INDEX.with(|c| c.borrow().len()) as i64;
    Ok(Value::Int(n))
}

fn rag_chunk(args: &[Value]) -> Result<Value, String> {
    let source = s_arg(args, 0, "rag_chunk")?;
    let text = s_arg(args, 1, "rag_chunk")?;
    let size = i_arg(args, 2, "rag_chunk")?;
    let overlap = i_arg(args, 3, "rag_chunk")?;
    if size <= 0 || overlap < 0 || overlap >= size {
        return Err("rag_chunk: require size > 0 and 0 <= overlap < size".into());
    }
    let chunker = RecursiveChunker::new(size as usize, overlap as usize);
    let chunks = chunker.chunks(source.as_str(), text.as_str());
    let list: Vec<Value> = chunks
        .into_iter()
        .map(|c| {
            let mut fields = Vec::new();
            fields.push(("source".to_string(), Value::String(Rc::new(c.source))));
            fields.push(("ordinal".to_string(), Value::Int(c.ordinal as i64)));
            fields.push(("text".to_string(), Value::String(Rc::new(c.text))));
            Value::Record(Rc::new(std::cell::RefCell::new(fields)))
        })
        .collect();
    Ok(Value::List(Rc::new(std::cell::RefCell::new(list))))
}

fn rag_ingest(args: &[Value]) -> Result<Value, String> {
    let source = s_arg(args, 0, "rag_ingest")?;
    let text = s_arg(args, 1, "rag_ingest")?;
    let added = RAG_INDEX.with(|idx_cell| {
        RAG_EMBEDDER.with(|emb_cell| {
            let mut idx = idx_cell.borrow_mut();
            let emb = emb_cell.borrow();
            let chunker = RecursiveChunker::default();
            let mut count: i64 = 0;
            for chunk in chunker.chunks(source.as_str(), text.as_str()) {
                if idx.add(&*emb, chunk, Default::default()) {
                    count += 1;
                }
            }
            count
        })
    });
    Ok(Value::Int(added))
}

fn rag_retrieve(args: &[Value]) -> Result<Value, String> {
    let query = s_arg(args, 0, "rag_retrieve")?;
    let k = i_arg(args, 1, "rag_retrieve")?;
    let alpha = if args.len() > 2 {
        match &args[2] {
            Value::Float(f) => *f as f32,
            Value::Int(i) => *i as f32,
            other => {
                return Err(format!(
                    "rag_retrieve: alpha must be a number, got `{}`",
                    other.type_name()
                ));
            }
        }
    } else {
        0.7f32
    };
    if !(0.0..=1.0).contains(&alpha) {
        return Err(format!("rag_retrieve: alpha must be in [0, 1], got {alpha}"));
    }
    let hits = RAG_INDEX.with(|idx_cell| {
        RAG_EMBEDDER.with(|emb_cell| {
            let idx = idx_cell.borrow();
            let emb = emb_cell.borrow();
            let r = Retriever::new(&*idx, &*emb).with_alpha(alpha);
            r.retrieve(query.as_str(), k.max(0) as usize)
        })
    });
    let list: Vec<Value> = hits
        .into_iter()
        .map(|h| {
            let mut fields = Vec::new();
            fields.push(("id".to_string(), Value::String(Rc::new(h.passage.id))));
            fields.push((
                "source".to_string(),
                Value::String(Rc::new(h.passage.chunk.source)),
            ));
            fields.push((
                "ordinal".to_string(),
                Value::Int(h.passage.chunk.ordinal as i64),
            ));
            fields.push((
                "text".to_string(),
                Value::String(Rc::new(h.passage.chunk.text)),
            ));
            fields.push(("score".to_string(), Value::Float(h.score as f64)));
            fields.push((
                "vector_score".to_string(),
                Value::Float(h.vector_score as f64),
            ));
            fields.push((
                "lexical_score".to_string(),
                Value::Float(h.lexical_score as f64),
            ));
            Value::Record(Rc::new(std::cell::RefCell::new(fields)))
        })
        .collect();
    Ok(Value::List(Rc::new(std::cell::RefCell::new(list))))
}

fn rag_save(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "rag_save")?;
    let bytes = RAG_INDEX.with(|c| c.borrow().to_json_bytes().map_err(|e| e.to_string()))?;
    std::fs::write(path.as_str(), &bytes).map_err(|e| format!("rag_save: {e}"))?;
    Ok(Value::Unit)
}

fn rag_load(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "rag_load")?;
    let bytes = std::fs::read(path.as_str()).map_err(|e| format!("rag_load: {e}"))?;
    let idx = Index::from_json_bytes(&bytes).map_err(|e| e.to_string())?;
    let dims = idx.dims;
    RAG_INDEX.with(|c| *c.borrow_mut() = idx);
    RAG_EMBEDDER.with(|c| *c.borrow_mut() = HashEmbedder::new(dims));
    Ok(Value::Unit)
}

// ---------------------------------------------------------------------------
// `media_*` bindings  (Stage 12 — §51 multimodal)
// ---------------------------------------------------------------------------

use axon_media::{Audio, Document, Image, MediaKind};

fn install_media(interp: &Interpreter) {
    interp.register_native(
        "media_image_load",
        n("media_image_load", 1, Some(1), media_image_load),
    );
    interp.register_native(
        "media_audio_load",
        n("media_audio_load", 1, Some(1), media_audio_load),
    );
    interp.register_native(
        "media_document_load",
        n("media_document_load", 1, Some(1), media_document_load),
    );
    interp.register_native("media_sniff", n("media_sniff", 1, Some(1), media_sniff));
}

fn media_image_load(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "media_image_load")?;
    let img = Image::from_path(path.as_str()).map_err(|e| e.to_string())?;
    let mut fields = Vec::new();
    fields.push((
        "kind".to_string(),
        Value::String(Rc::new("Image".to_string())),
    ));
    fields.push((
        "mime".to_string(),
        Value::String(Rc::new(img.format.mime().to_string())),
    ));
    fields.push(("width".to_string(), Value::Int(img.width as i64)));
    fields.push(("height".to_string(), Value::Int(img.height as i64)));
    fields.push(("byte_len".to_string(), Value::Int(img.byte_len as i64)));
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(fields))))
}

fn media_audio_load(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "media_audio_load")?;
    let a = Audio::from_path(path.as_str()).map_err(|e| e.to_string())?;
    let mut fields = Vec::new();
    fields.push((
        "kind".to_string(),
        Value::String(Rc::new("Audio".to_string())),
    ));
    fields.push((
        "mime".to_string(),
        Value::String(Rc::new(a.format.mime().to_string())),
    ));
    fields.push((
        "sample_rate_hz".to_string(),
        Value::Int(a.sample_rate_hz as i64),
    ));
    fields.push(("channels".to_string(), Value::Int(a.channels as i64)));
    fields.push((
        "bits_per_sample".to_string(),
        Value::Int(a.bits_per_sample as i64),
    ));
    fields.push((
        "duration_ms".to_string(),
        Value::Int(a.duration_ms as i64),
    ));
    fields.push(("byte_len".to_string(), Value::Int(a.byte_len as i64)));
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(fields))))
}

fn media_document_load(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "media_document_load")?;
    let d = Document::from_path(path.as_str()).map_err(|e| e.to_string())?;
    let mut fields = Vec::new();
    fields.push((
        "kind".to_string(),
        Value::String(Rc::new("Document".to_string())),
    ));
    fields.push(("mime".to_string(), Value::String(Rc::new(d.mime.clone()))));
    fields.push((
        "page_count".to_string(),
        Value::Int(d.page_count() as i64),
    ));
    fields.push((
        "byte_len".to_string(),
        Value::Int(d.byte_len as i64),
    ));
    let pages: Vec<Value> = d
        .pages
        .into_iter()
        .map(|p| Value::String(Rc::new(p)))
        .collect();
    fields.push((
        "pages".to_string(),
        Value::List(Rc::new(std::cell::RefCell::new(pages))),
    ));
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(fields))))
}

fn media_sniff(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "media_sniff")?;
    let bytes =
        std::fs::read(path.as_str()).map_err(|e| format!("media_sniff: {e}"))?;
    let r = axon_media::sniff(&bytes);
    let kind = match r.kind {
        MediaKind::Image => "Image",
        MediaKind::Audio => "Audio",
        MediaKind::Document => "Document",
        MediaKind::Unknown => "Unknown",
    };
    let mut fields = Vec::new();
    fields.push((
        "kind".to_string(),
        Value::String(Rc::new(kind.to_string())),
    ));
    fields.push((
        "mime".to_string(),
        Value::String(Rc::new(r.mime.to_string())),
    ));
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(fields))))
}

// ---------------------------------------------------------------------------
// `flow_*` bindings  (Stage 13 — §29 orchestration + §49 reasoning)
//
// Each binding is a `NativeExt` so it can re-enter the interpreter and
// invoke user-supplied callables (top-level fns or closures).
// ---------------------------------------------------------------------------

fn install_flow(interp: &Interpreter) {
    interp.register_native_ext(
        "flow_seq",
        ext("flow_seq", 2, Some(2), flow_seq_impl),
    );
    interp.register_native_ext(
        "flow_parallel",
        ext("flow_parallel", 2, Some(2), flow_parallel_impl),
    );
    interp.register_native_ext(
        "flow_refine",
        ext("flow_refine", 5, Some(5), flow_refine_impl),
    );
}

fn ext(
    name: &'static str,
    min_arity: usize,
    max_arity: Option<usize>,
    call: axon_runtime::NativeExtCall,
) -> NativeExtFn {
    NativeExtFn {
        name,
        min_arity,
        max_arity,
        required_caps: &[],
        call,
    }
}

/// Pull a list of callables from a `List`-valued argument.
fn callables_arg(args: &[Value], idx: usize, fn_name: &str) -> Result<Vec<Value>, String> {
    match &args[idx] {
        Value::List(l) => Ok(l.borrow().clone()),
        other => Err(format!(
            "`{fn_name}` expected a List of callables at position {idx}, got `{}`",
            other.type_name()
        )),
    }
}

fn is_callable(v: &Value) -> bool {
    matches!(
        v,
        Value::Fn(_) | Value::Native(_) | Value::NativeExt(_) | Value::Tool(_)
    )
}

fn flow_seq_impl(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let steps = callables_arg(args, 0, "flow_seq")?;
    let mut current = args[1].clone();
    for (i, step) in steps.iter().enumerate() {
        if !is_callable(step) {
            return Err(format!(
                "flow_seq: step at index {i} is not callable (got `{}`)",
                step.type_name()
            ));
        }
        match interp.call_value(step, &[current.clone()], span) {
            Ok(v) => current = v,
            Err(sig) => {
                return Err(format!("flow_seq[{i}]: {}", eval_signal_msg(&sig)));
            }
        }
    }
    Ok(current)
}

fn flow_parallel_impl(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let steps = callables_arg(args, 0, "flow_parallel")?;
    let input = args[1].clone();
    let mut out: Vec<Value> = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        if !is_callable(step) {
            return Err(format!(
                "flow_parallel: step at index {i} is not callable (got `{}`)",
                step.type_name()
            ));
        }
        match interp.call_value(step, &[input.clone()], span) {
            Ok(v) => out.push(v),
            Err(sig) => {
                return Err(format!(
                    "flow_parallel[branch={i}]: {}",
                    eval_signal_msg(&sig)
                ));
            }
        }
    }
    Ok(Value::List(Rc::new(std::cell::RefCell::new(out))))
}

/// `flow_refine(generate, critique, revise, max_rounds, accept_score)`
///
/// * `generate(prompt) -> draft`
/// * `critique(draft)  -> score` (Int or Float; higher is better)
/// * `revise(draft, score) -> draft`
/// * `accept_score`: numeric threshold — first draft with `score >= threshold` is returned
///
/// Returns a record `{ draft, score, rounds, outcome }` where outcome is
/// `"accepted"` or `"max_rounds"`.
fn flow_refine_impl(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let generate = &args[0];
    let critique = &args[1];
    let revise = &args[2];
    let max_rounds = match &args[3] {
        Value::Int(n) if *n >= 0 => *n as usize,
        other => {
            return Err(format!(
                "flow_refine: max_rounds must be a non-negative Int, got `{}`",
                other.type_name()
            ));
        }
    };
    let threshold = numeric_value(&args[4]).ok_or_else(|| {
        format!(
            "flow_refine: accept_score must be Int or Float, got `{}`",
            args[4].type_name()
        )
    })?;
    for (name, v) in [
        ("generate", generate),
        ("critique", critique),
        ("revise", revise),
    ] {
        if !is_callable(v) {
            return Err(format!(
                "flow_refine: `{name}` is not callable (got `{}`)",
                v.type_name()
            ));
        }
    }

    let mut draft = interp
        .call_value(generate, &[], span)
        .map_err(|e| format!("flow_refine[generate]: {}", eval_signal_msg(&e)))?;
    let mut score_value = interp
        .call_value(critique, &[draft.clone()], span)
        .map_err(|e| format!("flow_refine[critique:0]: {}", eval_signal_msg(&e)))?;
    let mut score = numeric_value(&score_value)
        .ok_or_else(|| "flow_refine: critique must return a number".to_string())?;
    let mut best = (draft.clone(), score, score_value.clone());

    let mut round: usize = 0;
    let mut outcome = if score >= threshold {
        "accepted".to_string()
    } else {
        String::new()
    };
    while outcome.is_empty() && round < max_rounds {
        round += 1;
        draft = interp
            .call_value(revise, &[draft.clone(), score_value.clone()], span)
            .map_err(|e| format!("flow_refine[revise:{round}]: {}", eval_signal_msg(&e)))?;
        score_value = interp
            .call_value(critique, &[draft.clone()], span)
            .map_err(|e| format!("flow_refine[critique:{round}]: {}", eval_signal_msg(&e)))?;
        score = numeric_value(&score_value)
            .ok_or_else(|| "flow_refine: critique must return a number".to_string())?;
        if score > best.1 {
            best = (draft.clone(), score, score_value.clone());
        }
        if score >= threshold {
            outcome = "accepted".to_string();
        }
    }
    if outcome.is_empty() {
        outcome = "max_rounds".to_string();
        draft = best.0;
        score_value = best.2;
    }
    let mut fields = Vec::new();
    fields.push(("draft".to_string(), draft));
    fields.push(("score".to_string(), score_value));
    fields.push(("rounds".to_string(), Value::Int(round as i64)));
    fields.push((
        "outcome".to_string(),
        Value::String(Rc::new(outcome)),
    ));
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(fields))))
}

fn numeric_value(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

fn eval_signal_msg(sig: &axon_runtime::EvalSignal) -> String {
    match sig {
        axon_runtime::EvalSignal::Error(e) => e.message.clone(),
        other => format!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// `trigger_*` bindings  (Stage 14 — §52 triggers & durable scheduling)
//
// A single thread-local Scheduler holds the trigger set for the process.
// Programs can opt into persistence by calling `trigger_attach_memory(path)`,
// which routes saves/loads through the memory store.
// ---------------------------------------------------------------------------

use axon_trigger::{Schedule, Scheduler, Trigger};

thread_local! {
    static SCHEDULER: RefCell<Scheduler> = RefCell::new(Scheduler::new());
}

fn install_trigger(interp: &Interpreter) {
    interp.register_native(
        "trigger_every",
        n("trigger_every", 3, Some(3), trigger_every),
    );
    interp.register_native(
        "trigger_at",
        n("trigger_at", 3, Some(3), trigger_at),
    );
    interp.register_native(
        "trigger_cron",
        n("trigger_cron", 3, Some(3), trigger_cron),
    );
    interp.register_native(
        "trigger_remove",
        n("trigger_remove", 1, Some(1), trigger_remove),
    );
    interp.register_native(
        "trigger_len",
        n("trigger_len", 0, Some(0), trigger_len),
    );
    interp.register_native_ext(
        "trigger_tick",
        ext("trigger_tick", 1, Some(1), trigger_tick_ext),
    );
    interp.register_native(
        "trigger_save",
        n("trigger_save", 0, Some(0), trigger_save),
    );
    interp.register_native(
        "trigger_load",
        n("trigger_load", 0, Some(0), trigger_load),
    );
}

fn trigger_every(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "trigger_every")?;
    let handler_name = s_arg(args, 1, "trigger_every")?;
    let period_seconds = i_arg(args, 2, "trigger_every")?;
    if period_seconds <= 0 {
        return Err("trigger_every: period must be positive".into());
    }
    let trigger = Trigger::new(
        id.as_str(),
        handler_name.as_str(),
        Schedule::every_seconds(period_seconds),
    );
    SCHEDULER
        .with(|s| s.borrow_mut().add(trigger))
        .map_err(|e| format!("trigger_every: {e}"))?;
    Ok(Value::Unit)
}

fn trigger_at(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "trigger_at")?;
    let handler_name = s_arg(args, 1, "trigger_at")?;
    let when_ns = i_arg(args, 2, "trigger_at")?;
    let trigger = Trigger::new(
        id.as_str(),
        handler_name.as_str(),
        Schedule::At { when_ns },
    );
    SCHEDULER
        .with(|s| s.borrow_mut().add(trigger))
        .map_err(|e| format!("trigger_at: {e}"))?;
    Ok(Value::Unit)
}

fn trigger_cron(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "trigger_cron")?;
    let handler_name = s_arg(args, 1, "trigger_cron")?;
    let expr = s_arg(args, 2, "trigger_cron")?;
    let sched = Schedule::cron(expr.as_str()).map_err(|e| format!("trigger_cron: {e}"))?;
    let trigger = Trigger::new(id.as_str(), handler_name.as_str(), sched);
    SCHEDULER
        .with(|s| s.borrow_mut().add(trigger))
        .map_err(|e| format!("trigger_cron: {e}"))?;
    Ok(Value::Unit)
}

fn trigger_remove(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "trigger_remove")?;
    let removed = SCHEDULER.with(|s| s.borrow_mut().remove(id.as_str()));
    Ok(Value::Bool(removed))
}

fn trigger_len(_args: &[Value]) -> Result<Value, String> {
    let n = SCHEDULER.with(|s| s.borrow().len()) as i64;
    Ok(Value::Int(n))
}

/// Tick the scheduler at `now_ns` and invoke every due trigger's handler.
/// The handler is looked up by name in the global environment — it must be
/// a callable taking zero args. Returns a List<{ id, handler, fired_at_ns }>
/// describing what fired.
fn trigger_tick_ext(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let now_ns = match &args[0] {
        Value::Int(i) => *i,
        Value::Duration(d) => *d, // Duration is ns already
        other => {
            return Err(format!(
                "trigger_tick: now_ns must be Int or Duration, got `{}`",
                other.type_name()
            ));
        }
    };
    let fired = SCHEDULER.with(|s| s.borrow_mut().tick(now_ns));
    let mut entries: Vec<Value> = Vec::with_capacity(fired.len());
    for f in &fired {
        let callee = interp.globals.lookup(&f.handler);
        if let Some(callee) = callee {
            interp
                .call_value(&callee, &[], span)
                .map_err(|e| format!("trigger_tick[{}]: {}", f.id, eval_signal_msg(&e)))?;
        } else {
            return Err(format!(
                "trigger_tick: handler `{}` for trigger `{}` not found in globals",
                f.handler, f.id
            ));
        }
        let mut rec = Vec::new();
        rec.push(("id".to_string(), Value::String(Rc::new(f.id.clone()))));
        rec.push((
            "handler".to_string(),
            Value::String(Rc::new(f.handler.clone())),
        ));
        rec.push(("fired_at_ns".to_string(), Value::Int(f.fired_at_ns)));
        entries.push(Value::Record(Rc::new(std::cell::RefCell::new(rec))));
    }
    Ok(Value::List(Rc::new(std::cell::RefCell::new(entries))))
}

/// Save the scheduler state into the currently-attached memory store
/// (whatever `mem_open_*` last set).
fn trigger_save(_args: &[Value]) -> Result<Value, String> {
    let store = current_store();
    SCHEDULER
        .with(|s| s.borrow().save_to_memory(&store))
        .map_err(|e| format!("trigger_save: {e}"))?;
    Ok(Value::Unit)
}

fn trigger_load(_args: &[Value]) -> Result<Value, String> {
    let store = current_store();
    let restored =
        Scheduler::load_from_memory(&store).map_err(|e| format!("trigger_load: {e}"))?;
    SCHEDULER.with(|s| *s.borrow_mut() = restored);
    Ok(Value::Unit)
}

// ---------------------------------------------------------------------------
// `skill_*` bindings  (Stage 14 — §53 skill packaging)
// ---------------------------------------------------------------------------

use axon_skill::Skill;

fn install_skill(interp: &Interpreter) {
    interp.register_native(
        "skill_pack",
        n("skill_pack", 2, Some(2), skill_pack),
    );
    interp.register_native(
        "skill_install",
        n("skill_install", 2, Some(2), skill_install),
    );
    interp.register_native(
        "skill_inspect",
        n("skill_inspect", 1, Some(1), skill_inspect),
    );
}

/// `skill_pack(src_dir, dest_axskill)` → ()
fn skill_pack(args: &[Value]) -> Result<Value, String> {
    let src = s_arg(args, 0, "skill_pack")?;
    let dest = s_arg(args, 1, "skill_pack")?;
    let skill = Skill::pack(src.as_str()).map_err(|e| e.to_string())?;
    let bytes = skill.to_json().map_err(|e| e.to_string())?;
    std::fs::write(dest.as_str(), &bytes).map_err(|e| format!("skill_pack: {e}"))?;
    Ok(Value::Unit)
}

/// `skill_install(axskill_path, dest_dir)` → record { name, version, file_count, entrypoint }
fn skill_install(args: &[Value]) -> Result<Value, String> {
    let pkg = s_arg(args, 0, "skill_install")?;
    let dest = s_arg(args, 1, "skill_install")?;
    let bytes = std::fs::read(pkg.as_str()).map_err(|e| format!("skill_install: {e}"))?;
    let skill = Skill::from_json(&bytes).map_err(|e| e.to_string())?;
    skill.unpack_to(dest.as_str()).map_err(|e| e.to_string())?;
    Ok(skill_to_record(&skill))
}

/// `skill_inspect(axskill_path)` → record (does NOT install)
fn skill_inspect(args: &[Value]) -> Result<Value, String> {
    let pkg = s_arg(args, 0, "skill_inspect")?;
    let bytes = std::fs::read(pkg.as_str()).map_err(|e| format!("skill_inspect: {e}"))?;
    let skill = Skill::from_json(&bytes).map_err(|e| e.to_string())?;
    Ok(skill_to_record(&skill))
}

fn skill_to_record(skill: &Skill) -> Value {
    let mut rec = Vec::new();
    rec.push((
        "name".to_string(),
        Value::String(Rc::new(skill.manifest.name.clone())),
    ));
    rec.push((
        "version".to_string(),
        Value::String(Rc::new(skill.manifest.version.clone())),
    ));
    rec.push((
        "entrypoint".to_string(),
        Value::String(Rc::new(skill.manifest.entrypoint.clone())),
    ));
    rec.push((
        "file_count".to_string(),
        Value::Int(skill.files.len() as i64),
    ));
    rec.push((
        "content_hash".to_string(),
        Value::String(Rc::new(skill.content_hash.clone())),
    ));
    let caps: Vec<Value> = skill
        .manifest
        .capabilities
        .iter()
        .map(|c| Value::String(Rc::new(c.clone())))
        .collect();
    rec.push((
        "capabilities".to_string(),
        Value::List(Rc::new(std::cell::RefCell::new(caps))),
    ));
    Value::Record(Rc::new(std::cell::RefCell::new(rec)))
}

// ---------------------------------------------------------------------------
// `a2a_*` bindings  (Stage 14 — §54 agent-to-agent interop)
// ---------------------------------------------------------------------------

use axon_a2a::AgentCard;

fn install_a2a(interp: &Interpreter) {
    interp.register_native(
        "a2a_card_load",
        n("a2a_card_load", 1, Some(1), a2a_card_load),
    );
    interp.register_native(
        "a2a_card_fetch",
        n("a2a_card_fetch", 1, Some(1), a2a_card_fetch),
    );
    interp.register_native(
        "a2a_card_save",
        n("a2a_card_save", 2, Some(2), a2a_card_save),
    );
    interp.register_native(
        "a2a_card_has_capability",
        n("a2a_card_has_capability", 2, Some(2), a2a_card_has_capability),
    );

    // Stage 22 — Ed25519 signed identity.
    interp.register_native(
        "a2a_keypair_generate",
        n("a2a_keypair_generate", 0, Some(0), a2a_keypair_generate),
    );
    interp.register_native(
        "a2a_keypair_from_seed",
        n("a2a_keypair_from_seed", 1, Some(1), a2a_keypair_from_seed),
    );
    interp.register_native(
        "a2a_sign_card",
        n("a2a_sign_card", 3, Some(3), a2a_sign_card),
    );
    interp.register_native(
        "a2a_verify_signed_card",
        n("a2a_verify_signed_card", 2, Some(2), a2a_verify_signed_card),
    );
    interp.register_native(
        "a2a_trust_store_new",
        n("a2a_trust_store_new", 2, Some(2), a2a_trust_store_new),
    );
    // Stage 23 — delegated identity.
    interp.register_native(
        "a2a_sign_delegation",
        n("a2a_sign_delegation", 7, Some(7), a2a_sign_delegation),
    );
    interp.register_native(
        "a2a_verify_delegation",
        n("a2a_verify_delegation", 4, Some(4), a2a_verify_delegation),
    );
}

fn a2a_card_load(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "a2a_card_load")?;
    let card = axon_a2a::load_card_from_path(path.as_str()).map_err(|e| e.to_string())?;
    Ok(card_to_record(&card))
}

fn a2a_card_fetch(args: &[Value]) -> Result<Value, String> {
    let url = s_arg(args, 0, "a2a_card_fetch")?;
    let card = axon_a2a::fetch_card(url.as_str()).map_err(|e| e.to_string())?;
    Ok(card_to_record(&card))
}

fn a2a_card_save(args: &[Value]) -> Result<Value, String> {
    // Saves a card-shaped record back to disk. The record must come from
    // a previous load/fetch so it round-trips losslessly.
    let _record = &args[0]; // Reserved: round-trip support lands when records carry origin metadata.
    let _path = s_arg(args, 1, "a2a_card_save")?;
    Err("a2a_card_save: not yet implemented in v0 (use a2a_card_load to round-trip)".into())
}

fn a2a_card_has_capability(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "a2a_card_has_capability")?;
    let name = s_arg(args, 1, "a2a_card_has_capability")?;
    let card = axon_a2a::load_card_from_path(path.as_str()).map_err(|e| e.to_string())?;
    Ok(Value::Bool(card.capability(name.as_str()).is_some()))
}

fn card_to_record(card: &AgentCard) -> Value {
    let mut rec = Vec::new();
    rec.push((
        "agent_id".to_string(),
        Value::String(Rc::new(card.agent_id.clone())),
    ));
    rec.push((
        "name".to_string(),
        Value::String(Rc::new(card.name.clone())),
    ));
    rec.push((
        "version".to_string(),
        Value::String(Rc::new(card.version.clone())),
    ));
    rec.push((
        "endpoint".to_string(),
        Value::String(Rc::new(card.endpoint.clone())),
    ));
    let cap_names: Vec<Value> = card
        .capabilities
        .iter()
        .map(|c| Value::String(Rc::new(c.name.clone())))
        .collect();
    rec.push((
        "capabilities".to_string(),
        Value::List(Rc::new(std::cell::RefCell::new(cap_names))),
    ));
    Value::Record(Rc::new(std::cell::RefCell::new(rec)))
}

// ---- Stage 22 Ed25519 identity bindings -----------------------------------
//
// Trust stores live in a thread-local registry keyed by name so programs
// can build one with `a2a_trust_store_new("partners", [hex...])` and
// reuse it across calls.

thread_local! {
    static TRUST_STORES: RefCell<std::collections::HashMap<String, axon_a2a::TrustStore>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Return a record `{ pubkey_hex, seed_hex }`. The seed is the private
/// key material — the caller should store it in the vault (Stage 15) and
/// never log it.
fn a2a_keypair_generate(_args: &[Value]) -> Result<Value, String> {
    let kp = axon_a2a::KeyPair::generate();
    Ok(keypair_to_record(&kp))
}

fn a2a_keypair_from_seed(args: &[Value]) -> Result<Value, String> {
    let seed_hex = s_arg(args, 0, "a2a_keypair_from_seed")?;
    let bytes = hex_decode_or_err(seed_hex.as_str(), "seed_hex")?;
    if bytes.len() != 32 {
        return Err(format!(
            "a2a_keypair_from_seed: seed must be 32 bytes (64 hex chars), got {}",
            bytes.len()
        ));
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&bytes);
    let kp = axon_a2a::KeyPair::from_seed_bytes(&seed);
    Ok(keypair_to_record(&kp))
}

fn keypair_to_record(kp: &axon_a2a::KeyPair) -> Value {
    let mut rec = Vec::new();
    rec.push((
        "pubkey_hex".to_string(),
        Value::String(Rc::new(kp.pubkey_hex())),
    ));
    rec.push((
        "seed_hex".to_string(),
        Value::String(Rc::new(kp.seed_hex())),
    ));
    Value::Record(Rc::new(std::cell::RefCell::new(rec)))
}

/// `a2a_sign_card(card_json_path, seed_hex, dest_json_path)` — read an
/// unsigned `AgentCard` from disk, sign it with the keypair derived from
/// `seed_hex`, and write the resulting `SignedAgentCard` envelope to
/// `dest_json_path`. Returns the signer's pubkey hex.
fn a2a_sign_card(args: &[Value]) -> Result<Value, String> {
    let card_path = s_arg(args, 0, "a2a_sign_card")?;
    let seed_hex = s_arg(args, 1, "a2a_sign_card")?;
    let dest_path = s_arg(args, 2, "a2a_sign_card")?;
    let card = axon_a2a::load_card_from_path(card_path.as_str())
        .map_err(|e| format!("a2a_sign_card: {e}"))?;
    let seed_bytes = hex_decode_or_err(seed_hex.as_str(), "seed_hex")?;
    if seed_bytes.len() != 32 {
        return Err(format!(
            "a2a_sign_card: seed must be 32 bytes, got {}",
            seed_bytes.len()
        ));
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&seed_bytes);
    let kp = axon_a2a::KeyPair::from_seed_bytes(&seed);
    let signed = kp.sign_card(&card).map_err(|e| format!("a2a_sign_card: {e}"))?;
    let bytes = signed
        .to_json()
        .map_err(|e| format!("a2a_sign_card: {e}"))?;
    std::fs::write(dest_path.as_str(), bytes)
        .map_err(|e| format!("a2a_sign_card: write {dest_path}: {e}"))?;
    Ok(Value::String(Rc::new(kp.pubkey_hex())))
}

/// `a2a_verify_signed_card(signed_json_path, trust_store_name)` →
/// the verified card as a Record, OR an error if the signature is
/// invalid / the signer isn't trusted.
fn a2a_verify_signed_card(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "a2a_verify_signed_card")?;
    let store_name = s_arg(args, 1, "a2a_verify_signed_card")?;
    let trust = TRUST_STORES
        .with(|reg| reg.borrow().get(store_name.as_str()).cloned())
        .ok_or_else(|| {
            format!(
                "a2a_verify_signed_card: no trust store `{store_name}` — \
                 call a2a_trust_store_new first"
            )
        })?;
    let card = axon_a2a::SignedAgentCard::load_and_verify(path.as_str(), &trust)
        .map_err(|e| format!("a2a_verify_signed_card: {e}"))?;
    Ok(card_to_record(&card))
}

/// `a2a_trust_store_new(name, allowed_pubkey_hex_list)` — register a
/// trust store under `name`. The list is a `List<String>` of 64-char
/// hex pubkeys. Existing stores under the same name are replaced.
fn a2a_trust_store_new(args: &[Value]) -> Result<Value, String> {
    // arg 0 = name (the second positional)
    // The native signature was `(allowed_pubkey_hex_list, name)` — keep
    // it ergonomic by accepting `(name, list)` here.
    let name = s_arg(args, 0, "a2a_trust_store_new")?;
    let list = match &args[1] {
        Value::List(l) => l.borrow().clone(),
        other => {
            return Err(format!(
                "a2a_trust_store_new: arg 1 must be a List<String>, got `{}`",
                other.type_name()
            ));
        }
    };
    let mut store = axon_a2a::TrustStore::new();
    for v in list {
        let hex = match v {
            Value::String(s) => s.as_str().to_string(),
            other => {
                return Err(format!(
                    "a2a_trust_store_new: list must contain Strings, got `{}`",
                    other.type_name()
                ));
            }
        };
        store
            .add_hex(&hex)
            .map_err(|e| format!("a2a_trust_store_new: {e}"))?;
    }
    TRUST_STORES.with(|reg| {
        reg.borrow_mut().insert(name.as_str().to_string(), store);
    });
    Ok(Value::Unit)
}

fn hex_decode_or_err(s: &str, label: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err(format!("{label}: odd-length hex string ({})", s.len()));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_nibble(bytes[i]).map_err(|e| format!("{label}: {e}"))?;
        let lo = hex_nibble(bytes[i + 1]).map_err(|e| format!("{label}: {e}"))?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(format!("bad hex digit `{}`", b as char)),
    }
}

/// `a2a_sign_delegation(seed_hex, principal, audience, scopes_list,
/// expires_at_secs, nonce, dest_json_path)` — produces a signed
/// `Delegation` envelope on disk. Returns the signer's pubkey hex.
fn a2a_sign_delegation(args: &[Value]) -> Result<Value, String> {
    let seed_hex = s_arg(args, 0, "a2a_sign_delegation")?;
    let principal = s_arg(args, 1, "a2a_sign_delegation")?;
    let audience = s_arg(args, 2, "a2a_sign_delegation")?;
    let scopes_list = match &args[3] {
        Value::List(l) => l.borrow().clone(),
        other => {
            return Err(format!(
                "a2a_sign_delegation: scopes must be List<String>, got `{}`",
                other.type_name()
            ));
        }
    };
    let expires_at_secs = i_arg(args, 4, "a2a_sign_delegation")?;
    let nonce = s_arg(args, 5, "a2a_sign_delegation")?;
    let dest_path = s_arg(args, 6, "a2a_sign_delegation")?;

    let scopes: Result<Vec<String>, String> = scopes_list
        .iter()
        .map(|v| match v {
            Value::String(s) => Ok(s.as_str().to_string()),
            other => Err(format!(
                "a2a_sign_delegation: scope must be String, got `{}`",
                other.type_name()
            )),
        })
        .collect();
    let scopes = scopes?;

    let seed_bytes = hex_decode_or_err(seed_hex.as_str(), "seed_hex")?;
    if seed_bytes.len() != 32 {
        return Err(format!(
            "a2a_sign_delegation: seed must be 32 bytes, got {}",
            seed_bytes.len()
        ));
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&seed_bytes);
    let kp = axon_a2a::KeyPair::from_seed_bytes(&seed);
    let d = axon_a2a::Delegation {
        principal: principal.as_str().to_string(),
        audience: audience.as_str().to_string(),
        scopes,
        expires_at_secs,
        nonce: nonce.as_str().to_string(),
    };
    let signed = kp
        .sign_delegation(&d)
        .map_err(|e| format!("a2a_sign_delegation: {e}"))?;
    let bytes = signed.to_json().map_err(|e| format!("a2a_sign_delegation: {e}"))?;
    std::fs::write(dest_path.as_str(), bytes)
        .map_err(|e| format!("a2a_sign_delegation: write {dest_path}: {e}"))?;
    Ok(Value::String(Rc::new(kp.pubkey_hex())))
}

/// `a2a_verify_delegation(signed_path, trust_store_name, expected_audience,
/// now_secs)` → record `{ principal, scopes, expires_at_secs, nonce }`
/// on success, or runtime error if signature/audience/expiry check fails.
fn a2a_verify_delegation(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "a2a_verify_delegation")?;
    let store_name = s_arg(args, 1, "a2a_verify_delegation")?;
    let audience = s_arg(args, 2, "a2a_verify_delegation")?;
    let now_secs = i_arg(args, 3, "a2a_verify_delegation")?;
    let trust = TRUST_STORES
        .with(|reg| reg.borrow().get(store_name.as_str()).cloned())
        .ok_or_else(|| {
            format!(
                "a2a_verify_delegation: no trust store `{store_name}` — \
                 call a2a_trust_store_new first"
            )
        })?;
    let d = axon_a2a::SignedDelegation::load_and_verify(
        path.as_str(),
        &trust,
        audience.as_str(),
        now_secs,
    )
    .map_err(|e| format!("a2a_verify_delegation: {e}"))?;
    let mut rec = Vec::new();
    rec.push(("principal".to_string(), Value::String(Rc::new(d.principal))));
    rec.push(("audience".to_string(), Value::String(Rc::new(d.audience))));
    let scopes: Vec<Value> = d
        .scopes
        .into_iter()
        .map(|s| Value::String(Rc::new(s)))
        .collect();
    rec.push((
        "scopes".to_string(),
        Value::List(Rc::new(std::cell::RefCell::new(scopes))),
    ));
    rec.push((
        "expires_at_secs".to_string(),
        Value::Int(d.expires_at_secs),
    ));
    rec.push(("nonce".to_string(), Value::String(Rc::new(d.nonce))));
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(rec))))
}

// ---------------------------------------------------------------------------
// `guard_*` bindings  (Stage 15 — §30 guardrails)
// ---------------------------------------------------------------------------

use axon_guard::{injection_score, ContentFilter, Policy, RuleAction, RuleMatch};

fn install_guard(interp: &Interpreter) {
    interp.register_native(
        "guard_scan_pii",
        n("guard_scan_pii", 1, Some(1), guard_scan_pii),
    );
    interp.register_native(
        "guard_scan_secrets",
        n("guard_scan_secrets", 1, Some(1), guard_scan_secrets),
    );
    interp.register_native(
        "guard_injection_score",
        n("guard_injection_score", 1, Some(1), guard_injection_score),
    );
    interp.register_native(
        "guard_policy_evaluate",
        n("guard_policy_evaluate", 2, Some(2), guard_policy_evaluate),
    );
}

fn guard_scan_pii(args: &[Value]) -> Result<Value, String> {
    let text = s_arg(args, 0, "guard_scan_pii")?;
    let findings = ContentFilter::strict().scan(text.as_str());
    Ok(findings_to_list(&findings))
}

fn guard_scan_secrets(args: &[Value]) -> Result<Value, String> {
    let text = s_arg(args, 0, "guard_scan_secrets")?;
    let findings = ContentFilter::secrets_only().scan(text.as_str());
    Ok(findings_to_list(&findings))
}

fn findings_to_list(findings: &[axon_guard::Finding]) -> Value {
    let items: Vec<Value> = findings
        .iter()
        .map(|f| {
            let mut rec = Vec::new();
            rec.push((
                "kind".to_string(),
                Value::String(Rc::new(format!("{:?}", f.kind))),
            ));
            rec.push(("start".to_string(), Value::Int(f.start as i64)));
            rec.push(("end".to_string(), Value::Int(f.end as i64)));
            rec.push((
                "redacted".to_string(),
                Value::String(Rc::new(f.redacted.clone())),
            ));
            Value::Record(Rc::new(std::cell::RefCell::new(rec)))
        })
        .collect();
    Value::List(Rc::new(std::cell::RefCell::new(items)))
}

fn guard_injection_score(args: &[Value]) -> Result<Value, String> {
    let text = s_arg(args, 0, "guard_injection_score")?;
    let report = injection_score(text.as_str());
    let mut rec = Vec::new();
    rec.push(("score".to_string(), Value::Float(report.score as f64)));
    let flags: Vec<Value> = report
        .flags
        .iter()
        .map(|f| Value::String(Rc::new(format!("{f:?}"))))
        .collect();
    rec.push((
        "flags".to_string(),
        Value::List(Rc::new(std::cell::RefCell::new(flags))),
    ));
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(rec))))
}

/// `guard_policy_evaluate(policy_json_path, input_text)` returns a record
/// `{ action: "allow"|"deny", rule_index, label }`. Reading the policy from
/// disk keeps the surface small until we have a real DSL parser.
fn guard_policy_evaluate(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "guard_policy_evaluate")?;
    let input = s_arg(args, 1, "guard_policy_evaluate")?;
    let bytes =
        std::fs::read(path.as_str()).map_err(|e| format!("guard_policy_evaluate: {e}"))?;
    let policy: Policy =
        serde_json::from_slice(&bytes).map_err(|e| format!("guard_policy_evaluate: {e}"))?;
    let decision = policy.evaluate(input.as_str());
    let mut rec = Vec::new();
    rec.push((
        "action".to_string(),
        Value::String(Rc::new(match decision.action {
            RuleAction::Allow => "allow".to_string(),
            RuleAction::Deny => "deny".to_string(),
        })),
    ));
    rec.push((
        "rule_index".to_string(),
        match decision.rule_index {
            Some(i) => Value::Int(i as i64),
            None => Value::Int(-1),
        },
    ));
    rec.push((
        "label".to_string(),
        Value::String(Rc::new(decision.label)),
    ));
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(rec))))
}

#[allow(dead_code)]
fn _suppress_unused_rule_match() {
    let _ = RuleMatch::Contains(String::new());
}

// ---------------------------------------------------------------------------
// `secret_*` bindings  (Stage 15 — §40 secrets)
//
// A single thread-local Vault file path mirrors the memory/scheduler model.
// `secret_open(path)` points at a vault; subsequent `secret_get/set/remove`
// hit it. Saves are atomic + 0600 on Unix.
// ---------------------------------------------------------------------------

use axon_secret::Vault;

thread_local! {
    static VAULT_PATH: RefCell<Option<std::path::PathBuf>> = RefCell::new(None);
}

fn install_secret(interp: &Interpreter) {
    interp.register_native(
        "secret_open",
        n("secret_open", 1, Some(1), secret_open),
    );
    interp.register_native(
        "secret_get",
        n("secret_get", 1, Some(1), secret_get),
    );
    interp.register_native(
        "secret_set",
        n("secret_set", 2, Some(2), secret_set),
    );
    interp.register_native(
        "secret_remove",
        n("secret_remove", 1, Some(1), secret_remove),
    );
    interp.register_native(
        "secret_names",
        n("secret_names", 0, Some(0), secret_names),
    );
    interp.register_native(
        "secret_redact",
        n("secret_redact", 1, Some(1), secret_redact),
    );
}

fn load_vault() -> Result<Vault, String> {
    let path =
        VAULT_PATH.with(|p| p.borrow().clone()).ok_or_else(|| {
            "vault not opened — call secret_open(path) first".to_string()
        })?;
    if path.exists() {
        Vault::load(&path).map_err(|e| e.to_string())
    } else {
        Ok(Vault::new())
    }
}

fn save_vault(v: &Vault) -> Result<(), String> {
    let path =
        VAULT_PATH.with(|p| p.borrow().clone()).ok_or_else(|| {
            "vault not opened — call secret_open(path) first".to_string()
        })?;
    v.save(&path).map_err(|e| e.to_string())
}

fn secret_open(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "secret_open")?;
    VAULT_PATH.with(|p| *p.borrow_mut() = Some(std::path::PathBuf::from(path.as_str())));
    Ok(Value::Unit)
}

fn secret_get(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "secret_get")?;
    let v = load_vault()?;
    let s = v.get(name.as_str()).map_err(|e| e.to_string())?;
    // Return the redacted display form by default. Programs that *need*
    // the cleartext should call expose_for_use through a future binding
    // gated behind a Secret capability (Stage 16).
    Ok(Value::String(Rc::new(format!("{s}"))))
}

fn secret_set(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "secret_set")?;
    let value = s_arg(args, 1, "secret_set")?;
    let mut v = load_vault()?;
    v.set(name.as_str(), value.as_str());
    save_vault(&v)?;
    Ok(Value::Unit)
}

fn secret_remove(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "secret_remove")?;
    let mut v = load_vault()?;
    let existed = v.remove(name.as_str());
    if existed {
        save_vault(&v)?;
    }
    Ok(Value::Bool(existed))
}

fn secret_names(_args: &[Value]) -> Result<Value, String> {
    let v = load_vault()?;
    let items: Vec<Value> = v
        .names()
        .into_iter()
        .map(|n| Value::String(Rc::new(n)))
        .collect();
    Ok(Value::List(Rc::new(std::cell::RefCell::new(items))))
}

fn secret_redact(args: &[Value]) -> Result<Value, String> {
    // Convenience: turn any string into the "<redacted>" sentinel for
    // logging code that hasn't been updated to take Secret<T> yet.
    let _ = s_arg(args, 0, "secret_redact")?;
    Ok(Value::String(Rc::new("<redacted>".to_string())))
}

// ---------------------------------------------------------------------------
// `sandbox_*` bindings  (Stage 15 — §42 sandbox)
// ---------------------------------------------------------------------------

use axon_sandbox::{run_sandboxed, Limits};
use std::process::Command;

fn install_sandbox(interp: &Interpreter) {
    interp.register_native(
        "sandbox_run",
        n("sandbox_run", 5, Some(5), sandbox_run),
    );
    interp.register_native(
        "sandbox_run_with_profile",
        n("sandbox_run_with_profile", 6, Some(6), sandbox_run_with_profile),
    );
}

/// `sandbox_run_with_profile(program, args_list, cpu_seconds, memory_mb,
/// wall_seconds, profile_name)` — same shape as `sandbox_run` but also
/// applies a kernel-level sandbox (`strict` / `networked` / `build_tool`).
fn sandbox_run_with_profile(args: &[Value]) -> Result<Value, String> {
    let program = s_arg(args, 0, "sandbox_run_with_profile")?;
    let arg_list = match &args[1] {
        Value::List(l) => l.borrow().clone(),
        other => {
            return Err(format!(
                "sandbox_run_with_profile: arg 1 must be List<String>, got `{}`",
                other.type_name()
            ));
        }
    };
    let cpu = i_arg(args, 2, "sandbox_run_with_profile")?.max(0) as u64;
    let mem = i_arg(args, 3, "sandbox_run_with_profile")?.max(0) as u64;
    let wall = i_arg(args, 4, "sandbox_run_with_profile")?.max(0) as u64;
    let profile_name = s_arg(args, 5, "sandbox_run_with_profile")?;
    let profile = match profile_name.as_str() {
        "strict" => axon_sandbox::PlatformProfile::strict(),
        "networked" => axon_sandbox::PlatformProfile::networked(),
        "build_tool" => axon_sandbox::PlatformProfile::build_tool(),
        other => {
            return Err(format!(
                "sandbox_run_with_profile: unknown profile `{other}` \
                 (expected strict / networked / build_tool)"
            ));
        }
    };
    let mut cmd = std::process::Command::new(program.as_str());
    for a in &arg_list {
        match a {
            Value::String(s) => {
                cmd.arg(s.as_str());
            }
            other => {
                return Err(format!(
                    "sandbox_run_with_profile: argv element is not String (got `{}`)",
                    other.type_name()
                ));
            }
        }
    }
    let sb = axon_sandbox::PlatformSandbox::new(profile);
    sb.apply(&mut cmd).map_err(|e| e.to_string())?;
    let limits = axon_sandbox::Limits {
        cpu_seconds: cpu,
        memory_mb: mem,
        max_open_files: 0,
        wall_seconds: wall,
    };
    let r = axon_sandbox::run_sandboxed(&mut cmd, &limits).map_err(|e| e.to_string())?;
    let mut rec = Vec::new();
    rec.push((
        "exit_code".to_string(),
        match r.exit_code {
            Some(c) => Value::Int(c as i64),
            None => Value::Int(-1),
        },
    ));
    rec.push(("stdout".to_string(), Value::String(Rc::new(r.stdout))));
    rec.push(("stderr".to_string(), Value::String(Rc::new(r.stderr))));
    rec.push(("wall_ms".to_string(), Value::Int(r.wall_ms as i64)));
    rec.push(("wall_timeout".to_string(), Value::Bool(r.wall_timeout)));
    rec.push(("limit_breached".to_string(), Value::Bool(r.limit_breached)));
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(rec))))
}

/// `sandbox_run(program, args_list, cpu_seconds, memory_mb, wall_seconds)`
/// → record `{ exit_code, stdout, stderr, wall_ms, wall_timeout, limit_breached }`.
fn sandbox_run(args: &[Value]) -> Result<Value, String> {
    let program = s_arg(args, 0, "sandbox_run")?;
    let arg_list = match &args[1] {
        Value::List(l) => l.borrow().clone(),
        other => {
            return Err(format!(
                "sandbox_run: arg 1 must be a List of String, got `{}`",
                other.type_name()
            ));
        }
    };
    let mut cmd = Command::new(program.as_str());
    for a in &arg_list {
        match a {
            Value::String(s) => {
                cmd.arg(s.as_str());
            }
            other => {
                return Err(format!(
                    "sandbox_run: argv element is not a String (got `{}`)",
                    other.type_name()
                ))
            }
        }
    }
    let limits = Limits {
        cpu_seconds: i_arg(args, 2, "sandbox_run")?.max(0) as u64,
        memory_mb: i_arg(args, 3, "sandbox_run")?.max(0) as u64,
        max_open_files: 0,
        wall_seconds: i_arg(args, 4, "sandbox_run")?.max(0) as u64,
    };
    let r = run_sandboxed(&mut cmd, &limits).map_err(|e| e.to_string())?;
    let mut rec = Vec::new();
    rec.push((
        "exit_code".to_string(),
        match r.exit_code {
            Some(c) => Value::Int(c as i64),
            None => Value::Int(-1),
        },
    ));
    rec.push(("stdout".to_string(), Value::String(Rc::new(r.stdout))));
    rec.push(("stderr".to_string(), Value::String(Rc::new(r.stderr))));
    rec.push(("wall_ms".to_string(), Value::Int(r.wall_ms as i64)));
    rec.push(("wall_timeout".to_string(), Value::Bool(r.wall_timeout)));
    rec.push((
        "limit_breached".to_string(),
        Value::Bool(r.limit_breached),
    ));
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(rec))))
}

// ---------------------------------------------------------------------------
// `eval_*` bindings  (Stage 16 — §55 trajectory evaluation)
//
// Programs build a suite by ID, push scenarios into it, then run with a
// handler-by-name. Each scenario calls the handler with its `input`; the
// handler returns a String which becomes `RunResult.output`. Metrics score
// the result; the call returns a record summarizing the suite.
// ---------------------------------------------------------------------------

use axon_eval::{Contains, ExactMatch, JsonPath, RegexLike, RunResult, Scenario, Suite};

thread_local! {
    static EVAL_SUITES: RefCell<std::collections::HashMap<String, Suite>> =
        RefCell::new(std::collections::HashMap::new());
}

fn install_eval(interp: &Interpreter) {
    interp.register_native(
        "eval_suite_new",
        n("eval_suite_new", 1, Some(1), eval_suite_new),
    );
    interp.register_native(
        "eval_add_scenario",
        n("eval_add_scenario", 4, Some(4), eval_add_scenario),
    );
    interp.register_native(
        "eval_add_metric",
        n("eval_add_metric", 2, Some(2), eval_add_metric),
    );
    interp.register_native(
        "eval_set_latency_budget",
        n("eval_set_latency_budget", 2, Some(2), eval_set_latency_budget),
    );
    interp.register_native_ext(
        "eval_run",
        ext("eval_run", 2, Some(2), eval_run_ext),
    );
    interp.register_native(
        "eval_report_junit",
        n("eval_report_junit", 2, Some(2), eval_report_junit),
    );
}

thread_local! {
    static EVAL_LAST_REPORT: RefCell<Option<axon_eval::SuiteReport>> = RefCell::new(None);
}

fn eval_suite_new(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "eval_suite_new")?;
    EVAL_SUITES.with(|s| {
        s.borrow_mut()
            .insert(name.as_str().to_string(), Suite::new(name.as_str()))
    });
    Ok(Value::Unit)
}

fn eval_add_scenario(args: &[Value]) -> Result<Value, String> {
    let suite_name = s_arg(args, 0, "eval_add_scenario")?;
    let scenario_name = s_arg(args, 1, "eval_add_scenario")?;
    let input = s_arg(args, 2, "eval_add_scenario")?;
    let expected = s_arg(args, 3, "eval_add_scenario")?;
    EVAL_SUITES.with(|s| -> Result<(), String> {
        let mut suites = s.borrow_mut();
        let suite = suites
            .get_mut(suite_name.as_str())
            .ok_or_else(|| format!("eval_add_scenario: no suite `{suite_name}`"))?;
        suite.scenarios.push(Scenario {
            name: scenario_name.as_str().to_string(),
            input: input.as_str().to_string(),
            expected: expected.as_str().to_string(),
            tags: vec![],
        });
        Ok(())
    })?;
    Ok(Value::Unit)
}

fn eval_add_metric(args: &[Value]) -> Result<Value, String> {
    let suite_name = s_arg(args, 0, "eval_add_metric")?;
    let metric_name = s_arg(args, 1, "eval_add_metric")?;
    let metric: Box<dyn axon_eval::Metric> = match metric_name.as_str() {
        "exact_match" => Box::new(ExactMatch),
        "contains" => Box::new(Contains),
        "regex_like" => Box::new(RegexLike),
        "json_path" => Box::new(JsonPath),
        other => return Err(format!("eval_add_metric: unknown metric `{other}`")),
    };
    EVAL_SUITES.with(|s| -> Result<(), String> {
        let mut suites = s.borrow_mut();
        let suite = suites
            .get_mut(suite_name.as_str())
            .ok_or_else(|| format!("eval_add_metric: no suite `{suite_name}`"))?;
        suite.metrics.push(metric);
        Ok(())
    })?;
    Ok(Value::Unit)
}

fn eval_set_latency_budget(args: &[Value]) -> Result<Value, String> {
    let suite_name = s_arg(args, 0, "eval_set_latency_budget")?;
    let budget_ms = i_arg(args, 1, "eval_set_latency_budget")?.max(0) as u64;
    EVAL_SUITES.with(|s| -> Result<(), String> {
        let mut suites = s.borrow_mut();
        let suite = suites
            .get_mut(suite_name.as_str())
            .ok_or_else(|| format!("eval_set_latency_budget: no suite `{suite_name}`"))?;
        suite.latency_metric = Some(axon_eval::LatencyP95::new(budget_ms));
        Ok(())
    })?;
    Ok(Value::Unit)
}

fn eval_run_ext(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let suite_name = s_arg(args, 0, "eval_run")?;
    let handler_value = args[1].clone();
    if !matches!(
        handler_value,
        Value::Fn(_) | Value::Native(_) | Value::NativeExt(_) | Value::Tool(_)
    ) {
        return Err(format!(
            "eval_run: handler must be callable (got `{}`)",
            handler_value.type_name()
        ));
    }

    let suite = EVAL_SUITES
        .with(|s| s.borrow_mut().remove(suite_name.as_str()))
        .ok_or_else(|| format!("eval_run: no suite `{suite_name}`"))?;

    let report = suite.run(|sc| {
        let start = std::time::Instant::now();
        match interp.call_value(
            &handler_value,
            &[Value::String(Rc::new(sc.input.clone()))],
            span,
        ) {
            Ok(v) => {
                let output = match v {
                    Value::String(s) => s.as_str().to_string(),
                    other => format!("{other}"),
                };
                RunResult::ok(output, start.elapsed().as_millis() as u64)
            }
            Err(e) => RunResult::err(eval_signal_msg(&e), start.elapsed().as_millis() as u64),
        }
    });

    let mut rec = Vec::new();
    rec.push(("total_runs".to_string(), Value::Int(report.total_runs as i64)));
    rec.push((
        "passed_runs".to_string(),
        Value::Int(report.passed_runs as i64),
    ));
    rec.push(("pass_rate".to_string(), Value::Float(report.pass_rate())));
    rec.push((
        "all_passed".to_string(),
        Value::Bool(report.all_passed()),
    ));
    EVAL_LAST_REPORT.with(|r| *r.borrow_mut() = Some(report));
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(rec))))
}

fn eval_report_junit(args: &[Value]) -> Result<Value, String> {
    let _suite_name = s_arg(args, 0, "eval_report_junit")?;
    let path = s_arg(args, 1, "eval_report_junit")?;
    let xml = EVAL_LAST_REPORT
        .with(|r| r.borrow().as_ref().map(|rep| rep.to_junit_xml()))
        .ok_or_else(|| "eval_report_junit: no report — call eval_run first".to_string())?;
    std::fs::write(path.as_str(), xml).map_err(|e| format!("eval_report_junit: {e}"))?;
    Ok(Value::Unit)
}

// ---------------------------------------------------------------------------
// `cost_*` bindings  (Stage 16 — §56 cost optimization)
// ---------------------------------------------------------------------------

use axon_cost::{CostEntry, Ledger, ProviderProfile, Report};

thread_local! {
    static COST_LEDGER: RefCell<Ledger> = RefCell::new(Ledger::new());
    static COST_PROFILES: RefCell<Vec<ProviderProfile>> = RefCell::new(Vec::new());
}

/// Sum the ledger for the post-run footer (§64.3): total tokens
/// (input + output) and total cost in cents, summed over every entry
/// against any registered profile. Returns `(0, 0)` when no model
/// calls were recorded.
pub fn footer_totals() -> (u64, u64) {
    let ledger = COST_LEDGER.with(|l| l.borrow().clone());
    if ledger.entries.is_empty() {
        return (0, 0);
    }
    let profiles = COST_PROFILES.with(|p| p.borrow().clone());
    let report = Report::build(&ledger, &profiles, 0);
    let tokens: u64 = report
        .providers
        .iter()
        .map(|p| p.input_tokens + p.output_tokens)
        .sum();
    (tokens, report.total_cents)
}

fn install_cost(interp: &Interpreter) {
    interp.register_native(
        "cost_record",
        n("cost_record", 6, Some(6), cost_record),
    );
    interp.register_native(
        "cost_profile_add",
        n("cost_profile_add", 4, Some(4), cost_profile_add),
    );
    interp.register_native(
        "cost_report",
        n("cost_report", 1, Some(1), cost_report),
    );
    interp.register_native(
        "cost_save",
        n("cost_save", 1, Some(1), cost_save),
    );
    interp.register_native(
        "cost_load",
        n("cost_load", 1, Some(1), cost_load),
    );
    interp.register_native(
        "cost_reset",
        n("cost_reset", 0, Some(0), cost_reset),
    );
}

fn cost_record(args: &[Value]) -> Result<Value, String> {
    let provider = s_arg(args, 0, "cost_record")?;
    let model = s_arg(args, 1, "cost_record")?;
    let input = i_arg(args, 2, "cost_record")?.max(0) as u32;
    let output = i_arg(args, 3, "cost_record")?.max(0) as u32;
    let latency = i_arg(args, 4, "cost_record")?.max(0) as u64;
    let tag = s_arg(args, 5, "cost_record")?;
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0);
    COST_LEDGER.with(|l| {
        l.borrow_mut().record(CostEntry {
            provider: provider.as_str().to_string(),
            model: model.as_str().to_string(),
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: 0,
            latency_ms: latency,
            timestamp_ns: now_ns,
            tag: tag.as_str().to_string(),
        })
    });
    Ok(Value::Unit)
}

fn cost_profile_add(args: &[Value]) -> Result<Value, String> {
    let provider = s_arg(args, 0, "cost_profile_add")?;
    let model = s_arg(args, 1, "cost_profile_add")?;
    let input_per_m = i_arg(args, 2, "cost_profile_add")?.max(0) as u64;
    let output_per_m = i_arg(args, 3, "cost_profile_add")?.max(0) as u64;
    COST_PROFILES.with(|p| {
        p.borrow_mut().push(ProviderProfile {
            name: provider.as_str().to_string(),
            model: model.as_str().to_string(),
            input_cents_per_million: input_per_m,
            output_cents_per_million: output_per_m,
            cached_input_cents_per_million: 0,
            per_call_cents: 0,
        });
    });
    Ok(Value::Unit)
}

fn cost_report(args: &[Value]) -> Result<Value, String> {
    let top_n = i_arg(args, 0, "cost_report")?.max(0) as usize;
    let report = COST_LEDGER.with(|l| {
        COST_PROFILES.with(|p| Report::build(&l.borrow(), &p.borrow(), top_n))
    });
    let mut rec = Vec::new();
    rec.push((
        "total_calls".to_string(),
        Value::Int(report.total_calls as i64),
    ));
    rec.push((
        "total_cents".to_string(),
        Value::Int(report.total_cents as i64),
    ));
    rec.push((
        "p50_latency_ms".to_string(),
        Value::Int(report.p50_latency_ms as i64),
    ));
    rec.push((
        "p95_latency_ms".to_string(),
        Value::Int(report.p95_latency_ms as i64),
    ));
    let providers: Vec<Value> = report
        .providers
        .iter()
        .map(|p| {
            let mut r = Vec::new();
            r.push((
                "provider".to_string(),
                Value::String(Rc::new(p.provider.clone())),
            ));
            r.push(("calls".to_string(), Value::Int(p.calls as i64)));
            r.push((
                "total_cents".to_string(),
                Value::Int(p.total_cents as i64),
            ));
            Value::Record(Rc::new(std::cell::RefCell::new(r)))
        })
        .collect();
    rec.push((
        "providers".to_string(),
        Value::List(Rc::new(std::cell::RefCell::new(providers))),
    ));
    let top: Vec<Value> = report
        .top_calls
        .iter()
        .map(|t| {
            let mut r = Vec::new();
            r.push((
                "provider".to_string(),
                Value::String(Rc::new(t.provider.clone())),
            ));
            r.push(("model".to_string(), Value::String(Rc::new(t.model.clone()))));
            r.push(("cents".to_string(), Value::Int(t.cents as i64)));
            Value::Record(Rc::new(std::cell::RefCell::new(r)))
        })
        .collect();
    rec.push((
        "top_calls".to_string(),
        Value::List(Rc::new(std::cell::RefCell::new(top))),
    ));
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(rec))))
}

fn cost_save(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "cost_save")?;
    COST_LEDGER
        .with(|l| l.borrow().save(path.as_str()))
        .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn cost_load(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "cost_load")?;
    let l = Ledger::load(path.as_str()).map_err(|e| e.to_string())?;
    COST_LEDGER.with(|c| *c.borrow_mut() = l);
    Ok(Value::Unit)
}

fn cost_reset(_args: &[Value]) -> Result<Value, String> {
    COST_LEDGER.with(|l| *l.borrow_mut() = Ledger::new());
    COST_PROFILES.with(|p| p.borrow_mut().clear());
    Ok(Value::Unit)
}

// ---------------------------------------------------------------------------
// `ffi_*` bindings  (Stage 16 — §35 subprocess FFI)
// ---------------------------------------------------------------------------

use axon_ffi::{call_once, FfiCallSpec};

fn install_ffi(interp: &Interpreter) {
    interp.register_native(
        "ffi_call",
        n("ffi_call", 4, Some(4), ffi_call),
    );
    // Stage 23 — dynamic-library FFI.
    interp.register_native(
        "ffi_dlib_call",
        n("ffi_dlib_call", 4, Some(4), ffi_dlib_call),
    );
}

/// `ffi_dlib_call(lib_path, symbol, args_list, ret_is_str)` →
/// `{ ok: Bool, value: Int | Float | String, error: String }`.
///
/// `args_list` is a list of records `{ ty: "i64"|"f64"|"str", v: <val> }`.
/// Supported shapes: all-i64 arity 0..=4 → i64; all-f64 arity 0..=2 → f64;
/// single str → str; void → str. See `axon_ffi::dlib` for details.
fn ffi_dlib_call(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "ffi_dlib_call")?;
    let symbol = s_arg(args, 1, "ffi_dlib_call")?;
    let args_list = match &args[2] {
        Value::List(l) => l.borrow().clone(),
        other => {
            return Err(format!(
                "ffi_dlib_call: arg 2 must be a List of records, got `{}`",
                other.type_name()
            ));
        }
    };
    let ret_is_str = match &args[3] {
        Value::Bool(b) => *b,
        other => {
            return Err(format!(
                "ffi_dlib_call: arg 3 (ret_is_str) must be Bool, got `{}`",
                other.type_name()
            ));
        }
    };
    let mut dlib_args: Vec<axon_ffi::DlibValue> = Vec::with_capacity(args_list.len());
    for (i, item) in args_list.iter().enumerate() {
        let rec = match item {
            Value::Record(r) => r.borrow().clone(),
            other => {
                return Err(format!(
                    "ffi_dlib_call: arg list element {i} must be a record, got `{}`",
                    other.type_name()
                ));
            }
        };
        let ty = rec
            .iter()
            .find(|(k, _)| k == "ty")
            .map(|(_, v)| v)
            .ok_or_else(|| {
                format!("ffi_dlib_call: arg #{i} record missing `ty`")
            })?;
        let v = rec
            .iter()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v)
            .ok_or_else(|| {
                format!("ffi_dlib_call: arg #{i} record missing `v`")
            })?;
        let ty_s = match ty {
            Value::String(s) => s.as_str().to_string(),
            other => {
                return Err(format!(
                    "ffi_dlib_call: arg #{i} `ty` must be String, got `{}`",
                    other.type_name()
                ));
            }
        };
        let dv = match (ty_s.as_str(), v) {
            ("i64", Value::Int(i)) => axon_ffi::DlibValue::I64(*i),
            ("f64", Value::Float(f)) => axon_ffi::DlibValue::F64(*f),
            ("f64", Value::Int(i)) => axon_ffi::DlibValue::F64(*i as f64),
            ("str", Value::String(s)) => axon_ffi::DlibValue::Str(s.as_str().to_string()),
            (other_ty, value) => {
                return Err(format!(
                    "ffi_dlib_call: arg #{i}: ty=`{other_ty}` doesn't match value `{}`",
                    value.type_name()
                ));
            }
        };
        dlib_args.push(dv);
    }

    let lib = match axon_ffi::DynamicLibrary::open(path.as_str()) {
        Ok(l) => l,
        Err(e) => return Ok(dlib_result(false, None, &e.to_string())),
    };
    match lib.call(symbol.as_str(), &dlib_args, ret_is_str) {
        Ok(v) => Ok(dlib_result(true, Some(dlib_value_to_axon(&v)), "")),
        Err(e) => Ok(dlib_result(false, None, &e.to_string())),
    }
}

fn dlib_value_to_axon(v: &axon_ffi::DlibValue) -> Value {
    match v {
        axon_ffi::DlibValue::I64(i) => Value::Int(*i),
        axon_ffi::DlibValue::F64(f) => Value::Float(*f),
        axon_ffi::DlibValue::Str(s) => Value::String(Rc::new(s.clone())),
    }
}

fn dlib_result(ok: bool, value: Option<Value>, error: &str) -> Value {
    let mut rec = Vec::new();
    rec.push(("ok".to_string(), Value::Bool(ok)));
    rec.push((
        "value".to_string(),
        value.unwrap_or(Value::Nil),
    ));
    rec.push(("error".to_string(), Value::String(Rc::new(error.to_string()))));
    Value::Record(Rc::new(std::cell::RefCell::new(rec)))
}

/// `ffi_call(program, args_list, request_json_string, timeout_ms)`
/// → `{ ok: Bool, response_json: String, error: String }`.
fn ffi_call(args: &[Value]) -> Result<Value, String> {
    let program = s_arg(args, 0, "ffi_call")?;
    let arg_list = match &args[1] {
        Value::List(l) => l.borrow().clone(),
        other => {
            return Err(format!(
                "ffi_call: arg 1 must be a List of String, got `{}`",
                other.type_name()
            ));
        }
    };
    let arg_strings: Result<Vec<String>, String> = arg_list
        .iter()
        .map(|a| match a {
            Value::String(s) => Ok(s.as_str().to_string()),
            other => Err(format!(
                "ffi_call: argv element is not a String (got `{}`)",
                other.type_name()
            )),
        })
        .collect();
    let request_str = s_arg(args, 2, "ffi_call")?;
    let timeout = i_arg(args, 3, "ffi_call")?.max(0) as u64;

    let spec = FfiCallSpec {
        program: program.as_str().to_string(),
        args: arg_strings?,
        workdir: None,
        timeout_ms: timeout,
    };
    let request: serde_json::Value = serde_json::from_str(request_str.as_str())
        .map_err(|e| format!("ffi_call: request not valid JSON: {e}"))?;
    let mut rec = Vec::new();
    match call_once(&spec, &request) {
        Ok(response) => {
            rec.push(("ok".to_string(), Value::Bool(true)));
            rec.push((
                "response_json".to_string(),
                Value::String(Rc::new(response.to_string())),
            ));
            rec.push(("error".to_string(), Value::String(Rc::new(String::new()))));
        }
        Err(e) => {
            rec.push(("ok".to_string(), Value::Bool(false)));
            rec.push((
                "response_json".to_string(),
                Value::String(Rc::new(String::new())),
            ));
            rec.push(("error".to_string(), Value::String(Rc::new(e.to_string()))));
        }
    }
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(rec))))
}

// ---------------------------------------------------------------------------
// `env_*` bindings  (Stage 17 — §41 environment binding)
// ---------------------------------------------------------------------------

fn install_env(interp: &Interpreter) {
    interp.register_native(
        "env_get",
        n("env_get", 1, Some(1), env_get),
    );
    interp.register_native(
        "env_get_or",
        n("env_get_or", 2, Some(2), env_get_or),
    );
    interp.register_native(
        "env_load_dotenv",
        n("env_load_dotenv", 2, Some(2), env_load_dotenv),
    );
}

fn env_get(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "env_get")?;
    Ok(match std::env::var(name.as_str()) {
        Ok(v) => Value::String(Rc::new(v)),
        Err(_) => Value::Nil,
    })
}

fn env_get_or(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "env_get_or")?;
    let default = s_arg(args, 1, "env_get_or")?;
    Ok(match std::env::var(name.as_str()) {
        Ok(v) => Value::String(Rc::new(v)),
        Err(_) => Value::String(default),
    })
}

fn env_load_dotenv(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "env_load_dotenv")?;
    let overwrite = match &args[1] {
        Value::Bool(b) => *b,
        other => {
            return Err(format!(
                "env_load_dotenv: overwrite must be Bool, got `{}`",
                other.type_name()
            ));
        }
    };
    let n = axon_deploy::dotenv::load(path.as_str(), overwrite)
        .map_err(|e| format!("env_load_dotenv: {e}"))?;
    Ok(Value::Int(n as i64))
}

// ---------------------------------------------------------------------------
// `serve_*` + `deploy_*` bindings  (Stage 17 — §41 deploy)
//
// `serve_run` starts an HTTP server on the requested address and routes
// POST /invoke to a user-named handler. The handler receives the request
// body (raw bytes as a String) and returns a String to send back.
// /healthz and /readyz are wired automatically.
//
// The call is BLOCKING — programs that need to do anything else after
// starting the server should `spawn` the call (Stage 5.5 actors) or run
// it as the last statement in `main`.
// ---------------------------------------------------------------------------

use axon_deploy::http::{Request, Response, Server};
use axon_deploy::DeployManifest;

fn install_deploy(interp: &Interpreter) {
    interp.register_native_ext(
        "serve_run",
        ext("serve_run", 2, Some(2), serve_run_ext),
    );
    interp.register_native_ext(
        "serve_run_tls",
        ext("serve_run_tls", 4, Some(4), serve_run_tls_ext),
    );
    interp.register_native(
        "deploy_write_manifest",
        n("deploy_write_manifest", 4, Some(4), deploy_write_manifest),
    );
}

/// `serve_run(listen_addr, handler)` — start the HTTP server. `handler` is
/// invoked for every non-health-check request as `handler(body_string)`.
/// Returns Unit when the server is shut down externally (in v0, a SIGINT
/// from the user — the server has no `shutdown` binding yet).
fn serve_run_ext(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let addr = s_arg(args, 0, "serve_run")?;
    let handler = args[1].clone();
    if !matches!(
        handler,
        Value::Fn(_) | Value::Native(_) | Value::NativeExt(_) | Value::Tool(_)
    ) {
        return Err(format!(
            "serve_run: handler must be callable (got `{}`)",
            handler.type_name()
        ));
    }
    let server = Server::bind(addr.as_str()).map_err(|e| format!("serve_run: bind {addr}: {e}"))?;
    let _ = server.install_signal_handler();
    eprintln!("axon serve: listening on {} (Ctrl-C to shutdown)", server.local_addr);

    // Channel of (Request, return-channel) so the request thread can
    // hand off to the interpreter thread synchronously.
    let (tx, rx) = std::sync::mpsc::channel::<(Request, std::sync::mpsc::Sender<Response>)>();
    let stop = server.stop.clone();

    let dispatch_thread = std::thread::spawn(move || -> std::io::Result<()> {
        server.run(move |req: &Request| -> Response {
            let (resp_tx, resp_rx) = std::sync::mpsc::channel::<Response>();
            if tx.send((req.clone(), resp_tx)).is_err() {
                return Response::text(503, "interpreter gone");
            }
            match resp_rx.recv_timeout(std::time::Duration::from_secs(30)) {
                Ok(r) => r,
                Err(_) => Response::text(504, "handler timeout"),
            }
        })
    });

    // Drain the channel on the main thread so the user's handler runs in
    // the interpreter's thread (the only thread that owns `interp`).
    while let Ok((req, resp_tx)) = rx.recv() {
        if stop.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        let body_str = String::from_utf8_lossy(&req.body).to_string();
        let response = match interp.call_value(
            &handler,
            &[Value::String(Rc::new(body_str))],
            span,
        ) {
            Ok(v) => {
                let text = match v {
                    Value::String(s) => s.as_str().to_string(),
                    other => format!("{other}"),
                };
                Response::text(200, text)
            }
            Err(e) => Response::text(500, format!("handler error: {}", eval_signal_msg(&e))),
        };
        let _ = resp_tx.send(response);
    }
    let _ = dispatch_thread.join();
    Ok(Value::Unit)
}

/// `serve_run_tls(listen_addr, handler, cert_pem_path, key_pem_path)` —
/// like `serve_run` but performs a rustls handshake on every accepted
/// connection. Cert and key are loaded once at bind time from PEM files;
/// production deployments typically point them at an ACME-managed pair
/// and redeploy on rotation.
fn serve_run_tls_ext(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let addr = s_arg(args, 0, "serve_run_tls")?;
    let handler = args[1].clone();
    let cert = s_arg(args, 2, "serve_run_tls")?;
    let key = s_arg(args, 3, "serve_run_tls")?;
    if !matches!(
        handler,
        Value::Fn(_) | Value::Native(_) | Value::NativeExt(_) | Value::Tool(_)
    ) {
        return Err(format!(
            "serve_run_tls: handler must be callable (got `{}`)",
            handler.type_name()
        ));
    }
    let server = Server::bind(addr.as_str())
        .map_err(|e| format!("serve_run_tls: bind {addr}: {e}"))?
        .with_tls_pem(cert.as_str(), key.as_str())
        .map_err(|e| format!("serve_run_tls: load TLS pem: {e}"))?;
    let _ = server.install_signal_handler();
    eprintln!(
        "axon serve [tls]: listening on https://{} (Ctrl-C to shutdown)",
        server.local_addr
    );
    let (tx, rx) = std::sync::mpsc::channel::<(Request, std::sync::mpsc::Sender<Response>)>();
    let stop = server.stop.clone();
    let dispatch_thread = std::thread::spawn(move || -> std::io::Result<()> {
        server.run(move |req: &Request| -> Response {
            let (resp_tx, resp_rx) = std::sync::mpsc::channel::<Response>();
            if tx.send((req.clone(), resp_tx)).is_err() {
                return Response::text(503, "interpreter gone");
            }
            match resp_rx.recv_timeout(std::time::Duration::from_secs(30)) {
                Ok(r) => r,
                Err(_) => Response::text(504, "handler timeout"),
            }
        })
    });
    while let Ok((req, resp_tx)) = rx.recv() {
        if stop.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        let body_str = String::from_utf8_lossy(&req.body).to_string();
        let response = match interp.call_value(
            &handler,
            &[Value::String(Rc::new(body_str))],
            span,
        ) {
            Ok(v) => {
                let text = match v {
                    Value::String(s) => s.as_str().to_string(),
                    other => format!("{other}"),
                };
                Response::text(200, text)
            }
            Err(e) => Response::text(500, format!("handler error: {}", eval_signal_msg(&e))),
        };
        let _ = resp_tx.send(response);
    }
    let _ = dispatch_thread.join();
    Ok(Value::Unit)
}

/// `deploy_write_manifest(dest_dir, name, entrypoint_handler, port)` →
/// writes `deploy.json` into `dest_dir`. The `.axskill` portion is
/// produced separately by `skill_pack` (Stage 14).
fn deploy_write_manifest(args: &[Value]) -> Result<Value, String> {
    let dir = s_arg(args, 0, "deploy_write_manifest")?;
    let name = s_arg(args, 1, "deploy_write_manifest")?;
    let entrypoint = s_arg(args, 2, "deploy_write_manifest")?;
    let port = i_arg(args, 3, "deploy_write_manifest")?;
    if !(0..=65535).contains(&port) {
        return Err(format!(
            "deploy_write_manifest: port must be in 0..=65535, got {port}"
        ));
    }
    let manifest = DeployManifest {
        version: axon_deploy::manifest::MANIFEST_VERSION,
        name: name.as_str().to_string(),
        entrypoint_handler: entrypoint.as_str().to_string(),
        port: port as u16,
        env: Default::default(),
        health_checks: vec!["liveness".into()],
        dotenv: None,
        vault: None,
    };
    std::fs::create_dir_all(dir.as_str())
        .map_err(|e| format!("deploy_write_manifest: mkdir {dir}: {e}"))?;
    let path = std::path::Path::new(dir.as_str()).join("deploy.json");
    manifest.save(&path).map_err(|e| e.to_string())?;
    Ok(Value::String(Rc::new(path.display().to_string())))
}

// ---------------------------------------------------------------------------
// `super_*` bindings  (Stage 18 — §22 supervisor restart strategies)
//
// A thread-local registry maps supervisor name → Supervisor. Programs build
// one with `super_new`, populate children with `super_add_child`, then
// drive restart decisions with `super_on_failure(now_ns)`. The decision
// comes back as a record `{ kind, targets, reason }` where `kind` is
// `"restart"` / `"escalate"` / `"unknown"`.
// ---------------------------------------------------------------------------

use axon_runtime::supervisor::{Decision, RestartStrategy, Supervisor};

thread_local! {
    static SUPERVISORS: RefCell<std::collections::HashMap<String, Supervisor>> =
        RefCell::new(std::collections::HashMap::new());
}

fn install_supervisor(interp: &Interpreter) {
    interp.register_native("super_new", n("super_new", 4, Some(4), super_new));
    interp.register_native(
        "super_add_child",
        n("super_add_child", 2, Some(2), super_add_child),
    );
    interp.register_native(
        "super_on_failure",
        n("super_on_failure", 3, Some(3), super_on_failure),
    );
    interp.register_native(
        "super_escalated",
        n("super_escalated", 1, Some(1), super_escalated),
    );
    interp.register_native(
        "super_reset",
        n("super_reset", 0, Some(0), super_reset),
    );
}

fn super_new(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "super_new")?;
    let strategy_name = s_arg(args, 1, "super_new")?;
    let max_restarts = i_arg(args, 2, "super_new")?;
    let within_ns = i_arg(args, 3, "super_new")?;
    if max_restarts < 0 {
        return Err("super_new: max_restarts must be ≥ 0".into());
    }
    if within_ns <= 0 {
        return Err("super_new: within_ns must be > 0".into());
    }
    let strategy = RestartStrategy::parse(strategy_name.as_str()).ok_or_else(|| {
        format!(
            "super_new: unknown strategy `{strategy_name}` (expected one_for_one / one_for_all / rest_for_one)"
        )
    })?;
    let s = Supervisor::new(
        name.as_str(),
        strategy,
        max_restarts as u32,
        within_ns,
    );
    SUPERVISORS.with(|reg| {
        reg.borrow_mut().insert(name.as_str().to_string(), s);
    });
    Ok(Value::Unit)
}

fn super_add_child(args: &[Value]) -> Result<Value, String> {
    let sup_name = s_arg(args, 0, "super_add_child")?;
    let child = s_arg(args, 1, "super_add_child")?;
    SUPERVISORS.with(|reg| -> Result<(), String> {
        let mut r = reg.borrow_mut();
        let s = r
            .get_mut(sup_name.as_str())
            .ok_or_else(|| format!("super_add_child: no supervisor `{sup_name}`"))?;
        s.add_child(child.as_str().to_string());
        Ok(())
    })?;
    Ok(Value::Unit)
}

fn super_on_failure(args: &[Value]) -> Result<Value, String> {
    let sup_name = s_arg(args, 0, "super_on_failure")?;
    let child = s_arg(args, 1, "super_on_failure")?;
    let now_ns = i_arg(args, 2, "super_on_failure")?;
    let decision = SUPERVISORS.with(|reg| -> Result<Decision, String> {
        let mut r = reg.borrow_mut();
        let s = r
            .get_mut(sup_name.as_str())
            .ok_or_else(|| format!("super_on_failure: no supervisor `{sup_name}`"))?;
        Ok(s.on_failure(child.as_str(), now_ns))
    })?;
    let mut rec = Vec::new();
    match decision {
        Decision::Restart(targets) => {
            rec.push((
                "kind".to_string(),
                Value::String(Rc::new("restart".to_string())),
            ));
            let xs: Vec<Value> = targets
                .into_iter()
                .map(|t| Value::String(Rc::new(t)))
                .collect();
            rec.push((
                "targets".to_string(),
                Value::List(Rc::new(std::cell::RefCell::new(xs))),
            ));
            rec.push(("reason".to_string(), Value::String(Rc::new(String::new()))));
        }
        Decision::Escalate { reason } => {
            rec.push((
                "kind".to_string(),
                Value::String(Rc::new("escalate".to_string())),
            ));
            rec.push((
                "targets".to_string(),
                Value::List(Rc::new(std::cell::RefCell::new(vec![]))),
            ));
            rec.push(("reason".to_string(), Value::String(Rc::new(reason))));
        }
        Decision::Unknown(name) => {
            rec.push((
                "kind".to_string(),
                Value::String(Rc::new("unknown".to_string())),
            ));
            rec.push((
                "targets".to_string(),
                Value::List(Rc::new(std::cell::RefCell::new(vec![]))),
            ));
            rec.push((
                "reason".to_string(),
                Value::String(Rc::new(format!("unknown child `{name}`"))),
            ));
        }
    }
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(rec))))
}

fn super_escalated(args: &[Value]) -> Result<Value, String> {
    let sup_name = s_arg(args, 0, "super_escalated")?;
    let escalated = SUPERVISORS.with(|reg| -> Result<bool, String> {
        let r = reg.borrow();
        let s = r
            .get(sup_name.as_str())
            .ok_or_else(|| format!("super_escalated: no supervisor `{sup_name}`"))?;
        Ok(s.is_escalated())
    })?;
    Ok(Value::Bool(escalated))
}

fn super_reset(_args: &[Value]) -> Result<Value, String> {
    SUPERVISORS.with(|reg| reg.borrow_mut().clear());
    Ok(Value::Unit)
}

// ---------------------------------------------------------------------------
// `schema_migrate_*` bindings  (Stage 18 — §17.1 schema migrations)
//
// A thread-local registry per schema name holds:
//   * the Migrator (current version + planned step versions)
//   * the user-supplied transform Values, keyed by source version.
//
// Programs build a migrator with `schema_migrator_new(name, current_version)`,
// register each step with `schema_add_migration(name, from_version, handler)`,
// then run upgrades with `schema_migrate(name, value, from_version)`. The
// handler is invoked once per step in order; if any step errors the chain
// short-circuits with a typed `{ ok: false, error: ... }` record.
// ---------------------------------------------------------------------------

use axon_runtime::migrate::{MigrationError, Migrator};

struct MigrationSlot {
    migrator: Migrator,
    /// Indexed by `from_version`. Same closure shape Axon programs hand in:
    /// `fn(input) -> output`.
    handlers: std::collections::HashMap<u32, Value>,
}

thread_local! {
    static MIGRATORS: RefCell<std::collections::HashMap<String, MigrationSlot>> =
        RefCell::new(std::collections::HashMap::new());
}

fn install_migrate(interp: &Interpreter) {
    interp.register_native(
        "schema_migrator_new",
        n("schema_migrator_new", 2, Some(2), schema_migrator_new),
    );
    interp.register_native(
        "schema_add_migration",
        n("schema_add_migration", 3, Some(3), schema_add_migration),
    );
    interp.register_native_ext(
        "schema_migrate",
        ext("schema_migrate", 3, Some(3), schema_migrate_ext),
    );
    interp.register_native(
        "schema_migrate_reset",
        n("schema_migrate_reset", 0, Some(0), schema_migrate_reset),
    );
}

fn schema_migrator_new(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "schema_migrator_new")?;
    let current = i_arg(args, 1, "schema_migrator_new")?;
    if current < 0 {
        return Err("schema_migrator_new: current_version must be ≥ 0".into());
    }
    let migrator = Migrator::new(name.as_str(), current as u32);
    MIGRATORS.with(|reg| {
        reg.borrow_mut().insert(
            name.as_str().to_string(),
            MigrationSlot {
                migrator,
                handlers: std::collections::HashMap::new(),
            },
        );
    });
    Ok(Value::Unit)
}

fn schema_add_migration(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "schema_add_migration")?;
    let from_version = i_arg(args, 1, "schema_add_migration")?;
    if from_version < 0 {
        return Err("schema_add_migration: from_version must be ≥ 0".into());
    }
    let handler = args[2].clone();
    if !matches!(
        handler,
        Value::Fn(_) | Value::Native(_) | Value::NativeExt(_) | Value::Tool(_)
    ) {
        return Err(format!(
            "schema_add_migration: handler must be callable, got `{}`",
            handler.type_name()
        ));
    }
    MIGRATORS.with(|reg| -> Result<(), String> {
        let mut r = reg.borrow_mut();
        let slot = r
            .get_mut(name.as_str())
            .ok_or_else(|| format!("schema_add_migration: no migrator `{name}`"))?;
        slot.migrator
            .add_step(from_version as u32)
            .map_err(|e| format!("schema_add_migration: {e}"))?;
        slot.handlers.insert(from_version as u32, handler);
        Ok(())
    })?;
    Ok(Value::Unit)
}

/// `schema_migrate(name, value, from_version)` → record
/// `{ ok: Bool, value: <migrated>, error: String }`.
fn schema_migrate_ext(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let name = s_arg(args, 0, "schema_migrate")?;
    let input = args[1].clone();
    let from_version = i_arg(args, 2, "schema_migrate")?;
    if from_version < 0 {
        return Err("schema_migrate: from_version must be ≥ 0".into());
    }

    // Plan first (borrows the registry briefly), then drop the borrow so
    // the handler can re-borrow if it wants to.
    let (plan_result, handlers): (Result<Vec<u32>, MigrationError>, _) = MIGRATORS.with(|reg| {
        let r = reg.borrow();
        let slot = match r.get(name.as_str()) {
            Some(s) => s,
            None => return (Err(MigrationError::Missing { from_version: 0 }), Vec::new()),
        };
        let plan = slot.migrator.plan(from_version as u32);
        let mut handlers_in_order: Vec<(u32, Value)> = Vec::new();
        if let Ok(p) = &plan {
            for v in p {
                if let Some(h) = slot.handlers.get(v) {
                    handlers_in_order.push((*v, h.clone()));
                }
            }
        }
        (plan, handlers_in_order)
    });

    let mut current = input;
    let mut error: Option<String> = None;
    match plan_result {
        Ok(_) => {
            for (v, h) in &handlers {
                match interp.call_value(h, &[current.clone()], span) {
                    Ok(next) => current = next,
                    Err(sig) => {
                        error = Some(format!(
                            "migration step from v{v} failed: {}",
                            eval_signal_msg(&sig)
                        ));
                        break;
                    }
                }
            }
        }
        Err(e) => error = Some(format!("{e}")),
    }

    let mut rec = Vec::new();
    if let Some(msg) = error {
        rec.push(("ok".to_string(), Value::Bool(false)));
        rec.push(("value".to_string(), Value::Nil));
        rec.push(("error".to_string(), Value::String(Rc::new(msg))));
    } else {
        rec.push(("ok".to_string(), Value::Bool(true)));
        rec.push(("value".to_string(), current));
        rec.push(("error".to_string(), Value::String(Rc::new(String::new()))));
    }
    Ok(Value::Record(Rc::new(std::cell::RefCell::new(rec))))
}

fn schema_migrate_reset(_args: &[Value]) -> Result<Value, String> {
    MIGRATORS.with(|reg| reg.borrow_mut().clear());
    Ok(Value::Unit)
}

// ---------------------------------------------------------------------------
// `trace_*` bindings  (Stage 20 — §31 OpenTelemetry / OTLP)
//
// Programs that enabled tracing (via the CLI's `--trace` flag, or by
// calling `axon_runtime::Interpreter::enable_tracing` directly from
// embedded use) can flush the current span list to an OTLP/HTTP-JSON
// document. That JSON is byte-compatible with what a real OTLP exporter
// POSTs to `/v1/traces` — pipe it to `otel-cli`, Tempo, Honeycomb, etc.
// ---------------------------------------------------------------------------

fn install_otlp(interp: &Interpreter) {
    interp.register_native_ext(
        "trace_export_otlp",
        ext("trace_export_otlp", 2, Some(2), trace_export_otlp_ext),
    );
}

/// `trace_export_otlp(path, service_name)` snapshots the live trace spans
/// (requires tracing was enabled — `axon run --trace ...`) and writes an
/// OTLP/HTTP-JSON document to `path`.
fn trace_export_otlp_ext(
    interp: &mut Interpreter,
    args: &[Value],
    _span: axon_diag::Span,
) -> Result<Value, String> {
    let path = s_arg(args, 0, "trace_export_otlp")?;
    let service = s_arg(args, 1, "trace_export_otlp")?;
    let spans = interp
        .with_trace_spans(|s| s.to_vec())
        .ok_or_else(|| {
            "trace_export_otlp: tracing is not enabled — re-run with `axon run --trace ...`"
                .to_string()
        })?;
    axon_runtime::otlp::write_to_path(&spans, service.as_str(), path.as_str())
        .map_err(|e| format!("trace_export_otlp: {e}"))?;
    Ok(Value::Unit)
}

// ===========================================================================
// Stage 24 — multi-agent orchestration (§29), reasoning & planning (§49),
// trajectory eval & red-teaming (§55), cost/latency optimization (§56).
//
// Stateful objects (Network, Graph, ReasoningBudget, Trajectory, World) are
// stored in thread-local registries keyed by string ids. Programs pass the
// id around like a handle. This keeps Axon's value model simple (string +
// records) while still supporting the full builder-pattern construction.
// ===========================================================================

use axon_cost::PrefixCache;
use axon_eval::redteam as rt_suite;
use axon_eval::sim as eval_sim;
use axon_eval::trajectory as traj;
use axon_flow::{DifficultyThresholds, GraphError, NetworkError};

thread_local! {
    static NETWORKS: RefCell<std::collections::HashMap<String, axon_flow::Network>> =
        RefCell::new(std::collections::HashMap::new());
    static GRAPHS: RefCell<std::collections::HashMap<String, axon_flow::WorkflowGraph>> =
        RefCell::new(std::collections::HashMap::new());
    static REASONING_BUDGETS: RefCell<std::collections::HashMap<String, axon_runtime::ReasoningBudget>> =
        RefCell::new(std::collections::HashMap::new());
    static TRAJECTORIES: RefCell<std::collections::HashMap<String, traj::Trajectory>> =
        RefCell::new(std::collections::HashMap::new());
    static SIM_WORLDS: RefCell<std::collections::HashMap<String, eval_sim::World>> =
        RefCell::new(std::collections::HashMap::new());
    static PREFIX_CACHE: PrefixCache = PrefixCache::new();
}

fn install_stage24(interp: &Interpreter) {
    // ---- §29.2 networks ----
    interp.register_native("flow_network_new", n("flow_network_new", 1, Some(1), s24_network_new));
    interp.register_native(
        "flow_network_add_node",
        n("flow_network_add_node", 2, Some(2), s24_network_add_node),
    );
    interp.register_native(
        "flow_network_add_edge",
        n("flow_network_add_edge", 4, Some(4), s24_network_add_edge),
    );
    interp.register_native(
        "flow_network_verify",
        n("flow_network_verify", 1, Some(1), s24_network_verify),
    );
    interp.register_native(
        "flow_network_unreachable_from",
        n(
            "flow_network_unreachable_from",
            2,
            Some(2),
            s24_network_unreachable_from,
        ),
    );

    // ---- §29.6 workflow graphs ----
    interp.register_native("flow_graph_new", n("flow_graph_new", 1, Some(1), s24_graph_new));
    interp.register_native(
        "flow_graph_add_node",
        n("flow_graph_add_node", 3, Some(3), s24_graph_add_node),
    );
    interp.register_native(
        "flow_graph_add_edge",
        n("flow_graph_add_edge", 3, Some(3), s24_graph_add_edge),
    );
    interp.register_native(
        "flow_graph_verify",
        n("flow_graph_verify", 1, Some(1), s24_graph_verify),
    );
    interp.register_native(
        "flow_graph_topo",
        n("flow_graph_topo", 1, Some(1), s24_graph_topo),
    );
    interp.register_native(
        "flow_graph_roots",
        n("flow_graph_roots", 1, Some(1), s24_graph_roots),
    );
    interp.register_native(
        "flow_graph_leaves",
        n("flow_graph_leaves", 1, Some(1), s24_graph_leaves),
    );
    interp.register_native_ext(
        "flow_graph_run",
        ext("flow_graph_run", 3, Some(3), s24_graph_run),
    );

    // ---- §29.8 / §49.2 / §56.3 extra combinators ----
    interp.register_native_ext(
        "flow_debate",
        ext("flow_debate", 5, Some(5), s24_flow_debate),
    );
    interp.register_native_ext(
        "flow_tree_of_thought",
        ext("flow_tree_of_thought", 5, Some(5), s24_flow_tot),
    );
    interp.register_native_ext(
        "flow_race",
        ext("flow_race", 3, Some(3), s24_flow_race),
    );
    interp.register_native_ext(
        "flow_batch",
        ext("flow_batch", 2, Some(2), s24_flow_batch),
    );

    // ---- §56.4 difficulty router ----
    interp.register_native(
        "flow_estimate_difficulty",
        n(
            "flow_estimate_difficulty",
            1,
            Some(1),
            s24_estimate_difficulty,
        ),
    );
    interp.register_native_ext(
        "flow_route_difficulty",
        ext(
            "flow_route_difficulty",
            4,
            Some(4),
            s24_route_difficulty,
        ),
    );

    // ---- §49.1 reasoning budgets ----
    interp.register_native(
        "reasoning_budget_new",
        n(
            "reasoning_budget_new",
            4,
            Some(4),
            s24_reasoning_budget_new,
        ),
    );
    interp.register_native(
        "reasoning_budget_debit",
        n(
            "reasoning_budget_debit",
            2,
            Some(2),
            s24_reasoning_budget_debit,
        ),
    );
    interp.register_native(
        "reasoning_budget_status",
        n(
            "reasoning_budget_status",
            1,
            Some(1),
            s24_reasoning_budget_status,
        ),
    );

    // ---- §49.2 ReAct loop driver ----
    interp.register_native_ext(
        "plan_react_loop",
        ext("plan_react_loop", 4, Some(4), s24_plan_react_loop),
    );

    // ---- §55.1 trajectory eval ----
    interp.register_native(
        "eval_trajectory_new",
        n(
            "eval_trajectory_new",
            4,
            Some(4),
            s24_traj_new,
        ),
    );
    interp.register_native(
        "eval_trajectory_add_step",
        n(
            "eval_trajectory_add_step",
            6,
            Some(6),
            s24_traj_add_step,
        ),
    );
    interp.register_native(
        "eval_trajectory_set_answer",
        n(
            "eval_trajectory_set_answer",
            2,
            Some(2),
            s24_traj_set_answer,
        ),
    );
    interp.register_native(
        "eval_trajectory_tool_accuracy",
        n(
            "eval_trajectory_tool_accuracy",
            1,
            Some(1),
            s24_traj_tool_accuracy,
        ),
    );
    interp.register_native(
        "eval_trajectory_step_efficiency",
        n(
            "eval_trajectory_step_efficiency",
            1,
            Some(1),
            s24_traj_step_efficiency,
        ),
    );
    interp.register_native(
        "eval_trajectory_recovered",
        n(
            "eval_trajectory_recovered",
            1,
            Some(1),
            s24_traj_recovered,
        ),
    );
    interp.register_native(
        "eval_trajectory_no_forbidden_tool",
        n(
            "eval_trajectory_no_forbidden_tool",
            1,
            Some(1),
            s24_traj_no_forbidden,
        ),
    );
    interp.register_native(
        "eval_trajectory_grounded",
        n(
            "eval_trajectory_grounded",
            1,
            Some(1),
            s24_traj_grounded,
        ),
    );
    interp.register_native(
        "eval_trajectory_no_secret_exposed",
        n(
            "eval_trajectory_no_secret_exposed",
            2,
            Some(2),
            s24_traj_no_secret,
        ),
    );

    // ---- §55.2 redteam ----
    interp.register_native(
        "redteam_load",
        n("redteam_load", 1, Some(1), s24_redteam_load),
    );
    interp.register_native(
        "redteam_refusal_phrases",
        n(
            "redteam_refusal_phrases",
            0,
            Some(0),
            s24_redteam_refusal_phrases,
        ),
    );

    // ---- §55.3 sim world ----
    interp.register_native("sim_world_new", n("sim_world_new", 2, Some(2), s24_sim_new));
    interp.register_native(
        "sim_world_spawn",
        n("sim_world_spawn", 2, Some(2), s24_sim_spawn),
    );
    interp.register_native(
        "sim_world_script_send",
        n("sim_world_script_send", 4, Some(4), s24_sim_script_send),
    );
    interp.register_native(
        "sim_world_script_note",
        n("sim_world_script_note", 4, Some(4), s24_sim_script_note),
    );
    interp.register_native(
        "sim_world_script_settle",
        n("sim_world_script_settle", 2, Some(2), s24_sim_script_settle),
    );
    interp.register_native(
        "sim_world_send_to",
        n("sim_world_send_to", 3, Some(3), s24_sim_send_to),
    );
    interp.register_native(
        "sim_world_advance",
        n("sim_world_advance", 2, Some(2), s24_sim_advance),
    );
    interp.register_native(
        "sim_world_run_until_settled",
        n(
            "sim_world_run_until_settled",
            4,
            Some(4),
            s24_sim_run_until_settled,
        ),
    );
    interp.register_native(
        "sim_world_events",
        n("sim_world_events", 1, Some(1), s24_sim_events),
    );
    interp.register_native(
        "sim_world_rand_u64",
        n("sim_world_rand_u64", 1, Some(1), s24_sim_rand),
    );

    // ---- §56.1 prefix cache ----
    interp.register_native(
        "cost_cache_insert",
        n("cost_cache_insert", 3, Some(3), s24_cache_insert),
    );
    interp.register_native(
        "cost_cache_lookup",
        n("cost_cache_lookup", 1, Some(1), s24_cache_lookup),
    );
    interp.register_native(
        "cost_cache_stats",
        n("cost_cache_stats", 0, Some(0), s24_cache_stats),
    );
    interp.register_native(
        "cost_cache_clear",
        n("cost_cache_clear", 0, Some(0), s24_cache_clear),
    );
}

// --------- helpers ---------

fn now_ns() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

fn b_arg(args: &[Value], idx: usize, fn_name: &str) -> Result<bool, String> {
    match &args[idx] {
        Value::Bool(b) => Ok(*b),
        other => Err(format!(
            "`{fn_name}` expected a Bool at position {idx}, got `{}`",
            other.type_name()
        )),
    }
}

fn list_of_strings(v: &Value, fn_name: &str, field: &str) -> Result<Vec<String>, String> {
    match v {
        Value::List(l) => l
            .borrow()
            .iter()
            .map(|item| match item {
                Value::String(s) => Ok(s.as_str().to_string()),
                other => Err(format!(
                    "{fn_name}: {field} elements must be String, got `{}`",
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "{fn_name}: {field} must be List<String>, got `{}`",
            other.type_name()
        )),
    }
}

fn record_to_vec(fields: Vec<(&str, Value)>) -> Value {
    let v: Vec<(String, Value)> = fields
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    Value::Record(Rc::new(RefCell::new(v)))
}

fn list_value(items: Vec<Value>) -> Value {
    Value::List(Rc::new(RefCell::new(items)))
}

fn ok_err_record(ok: bool, error: &str) -> Value {
    record_to_vec(vec![
        ("ok", Value::Bool(ok)),
        ("error", Value::String(Rc::new(error.to_string()))),
    ])
}

// --------- §29.2 networks ---------

fn s24_network_new(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "flow_network_new")?;
    NETWORKS.with(|c| {
        c.borrow_mut()
            .insert(name.as_str().to_string(), axon_flow::Network::new(name.as_str()))
    });
    Ok(Value::String(name))
}

fn s24_network_add_node(args: &[Value]) -> Result<Value, String> {
    let net = s_arg(args, 0, "flow_network_add_node")?;
    let node = s_arg(args, 1, "flow_network_add_node")?;
    NETWORKS.with(|c| {
        let mut map = c.borrow_mut();
        let n = map
            .get_mut(net.as_str())
            .ok_or_else(|| format!("flow_network_add_node: no network `{}`", net.as_str()))?;
        n.add_node(node.as_str())
            .map_err(|e: NetworkError| e.to_string())?;
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s24_network_add_edge(args: &[Value]) -> Result<Value, String> {
    let net = s_arg(args, 0, "flow_network_add_edge")?;
    let from = s_arg(args, 1, "flow_network_add_edge")?;
    let to = s_arg(args, 2, "flow_network_add_edge")?;
    let kind_str = s_arg(args, 3, "flow_network_add_edge")?;
    let kind = match kind_str.as_str() {
        "oneway" | "one_way" | "->" => axon_flow::EdgeKind::OneWay,
        "bidi" | "bidirectional" | "<->" => axon_flow::EdgeKind::Bidirectional,
        other => {
            return Err(format!(
                "flow_network_add_edge: kind must be `oneway`|`bidi`, got `{other}`"
            ));
        }
    };
    NETWORKS.with(|c| {
        let mut map = c.borrow_mut();
        let n = map
            .get_mut(net.as_str())
            .ok_or_else(|| format!("flow_network_add_edge: no network `{}`", net.as_str()))?;
        n.add_edge(from.as_str(), to.as_str(), kind);
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s24_network_verify(args: &[Value]) -> Result<Value, String> {
    let net = s_arg(args, 0, "flow_network_verify")?;
    let res = NETWORKS.with(|c| {
        c.borrow()
            .get(net.as_str())
            .map(|n| n.verify())
            .unwrap_or_else(|| {
                Err(NetworkError::Empty)
            })
    });
    Ok(match res {
        Ok(_) => ok_err_record(true, ""),
        Err(e) => ok_err_record(false, &e.to_string()),
    })
}

fn s24_network_unreachable_from(args: &[Value]) -> Result<Value, String> {
    let net = s_arg(args, 0, "flow_network_unreachable_from")?;
    let root = s_arg(args, 1, "flow_network_unreachable_from")?;
    let names: Vec<String> = NETWORKS.with(|c| {
        c.borrow()
            .get(net.as_str())
            .map(|n| n.unreachable_from(root.as_str()))
            .unwrap_or_default()
    });
    Ok(list_value(
        names.into_iter().map(|s| Value::String(Rc::new(s))).collect(),
    ))
}

// --------- §29.6 graphs ---------

fn s24_graph_new(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "flow_graph_new")?;
    GRAPHS.with(|c| {
        c.borrow_mut().insert(
            name.as_str().to_string(),
            axon_flow::WorkflowGraph::new(name.as_str()),
        )
    });
    Ok(Value::String(name))
}

fn s24_graph_add_node(args: &[Value]) -> Result<Value, String> {
    let g = s_arg(args, 0, "flow_graph_add_node")?;
    let name = s_arg(args, 1, "flow_graph_add_node")?;
    let label = s_arg(args, 2, "flow_graph_add_node")?;
    GRAPHS.with(|c| {
        let mut map = c.borrow_mut();
        let gg = map
            .get_mut(g.as_str())
            .ok_or_else(|| format!("flow_graph_add_node: no graph `{}`", g.as_str()))?;
        gg.add_node(name.as_str(), label.as_str())
            .map_err(|e: GraphError| e.to_string())?;
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s24_graph_add_edge(args: &[Value]) -> Result<Value, String> {
    let g = s_arg(args, 0, "flow_graph_add_edge")?;
    let from = s_arg(args, 1, "flow_graph_add_edge")?;
    let to = s_arg(args, 2, "flow_graph_add_edge")?;
    GRAPHS.with(|c| {
        let mut map = c.borrow_mut();
        let gg = map
            .get_mut(g.as_str())
            .ok_or_else(|| format!("flow_graph_add_edge: no graph `{}`", g.as_str()))?;
        gg.add_edge(from.as_str(), to.as_str());
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s24_graph_verify(args: &[Value]) -> Result<Value, String> {
    let g = s_arg(args, 0, "flow_graph_verify")?;
    let res = GRAPHS.with(|c| {
        c.borrow()
            .get(g.as_str())
            .map(|gg| gg.verify())
            .unwrap_or(Err(GraphError::Empty))
    });
    Ok(match res {
        Ok(_) => ok_err_record(true, ""),
        Err(e) => ok_err_record(false, &e.to_string()),
    })
}

fn s24_graph_topo(args: &[Value]) -> Result<Value, String> {
    let g = s_arg(args, 0, "flow_graph_topo")?;
    let order = GRAPHS.with(|c| {
        c.borrow()
            .get(g.as_str())
            .map(|gg| gg.topological_order())
            .unwrap_or_else(|| Ok(Vec::new()))
    });
    let order = order.map_err(|e| e.to_string())?;
    Ok(list_value(
        order.into_iter().map(|s| Value::String(Rc::new(s))).collect(),
    ))
}

fn s24_graph_roots(args: &[Value]) -> Result<Value, String> {
    let g = s_arg(args, 0, "flow_graph_roots")?;
    let r = GRAPHS.with(|c| {
        c.borrow()
            .get(g.as_str())
            .map(|gg| gg.roots())
            .unwrap_or_default()
    });
    Ok(list_value(r.into_iter().map(|s| Value::String(Rc::new(s))).collect()))
}

fn s24_graph_leaves(args: &[Value]) -> Result<Value, String> {
    let g = s_arg(args, 0, "flow_graph_leaves")?;
    let r = GRAPHS.with(|c| {
        c.borrow()
            .get(g.as_str())
            .map(|gg| gg.leaves())
            .unwrap_or_default()
    });
    Ok(list_value(r.into_iter().map(|s| Value::String(Rc::new(s))).collect()))
}

/// `flow_graph_run(graph_name, node_step_map, initial_value)`
///
/// `node_step_map` is a Record mapping node name -> callable.
/// Each callable receives `(node_name, predecessors_results_record)` and
/// returns a Value. Nodes are scheduled in topological order; an empty
/// predecessor record is passed to roots.
fn s24_graph_run(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let g_name = s_arg(args, 0, "flow_graph_run")?;
    let step_map = match &args[1] {
        Value::Record(r) => r.borrow().clone(),
        other => {
            return Err(format!(
                "flow_graph_run: node_step_map must be a Record, got `{}`",
                other.type_name()
            ));
        }
    };
    let _initial = args[2].clone();

    let (order, edges) = GRAPHS.with(|c| {
        let map = c.borrow();
        let gg = map
            .get(g_name.as_str())
            .ok_or_else(|| format!("flow_graph_run: no graph `{}`", g_name.as_str()))?;
        let order = gg.topological_order().map_err(|e| e.to_string())?;
        let edges = gg.edges.clone();
        Ok::<_, String>((order, edges))
    })?;

    use std::collections::HashMap;
    let mut results: HashMap<String, Value> = HashMap::new();
    for node in &order {
        let step = step_map
            .iter()
            .find(|(k, _)| k == node)
            .map(|(_, v)| v.clone());
        let Some(step) = step else {
            return Err(format!(
                "flow_graph_run: no step provided for node `{node}`"
            ));
        };
        if !is_callable(&step) {
            return Err(format!(
                "flow_graph_run: step for `{node}` is not callable"
            ));
        }
        // Build the predecessor results record for this node.
        let mut preds: Vec<(String, Value)> = Vec::new();
        for e in &edges {
            if e.to == *node {
                if let Some(v) = results.get(&e.from) {
                    preds.push((e.from.clone(), v.clone()));
                }
            }
        }
        let pred_rec = Value::Record(Rc::new(RefCell::new(preds)));
        let call_args = [Value::String(Rc::new(node.clone())), pred_rec];
        let v = interp
            .call_value(&step, &call_args, span)
            .map_err(|e| format!("flow_graph_run[{node}]: {}", eval_signal_msg(&e)))?;
        results.insert(node.clone(), v);
    }
    let final_record: Vec<(String, Value)> = order
        .into_iter()
        .filter_map(|n| results.remove(&n).map(|v| (n, v)))
        .collect();
    Ok(Value::Record(Rc::new(RefCell::new(final_record))))
}

// --------- §29.8 / §49.2 combinators ---------

fn s24_flow_debate(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let question = s_arg(args, 0, "flow_debate")?;
    let pro = args[1].clone();
    let con = args[2].clone();
    let judge = args[3].clone();
    let rounds = i_arg(args, 4, "flow_debate")?.max(0) as usize;
    for (name, v) in [("pro", &pro), ("con", &con), ("judge", &judge)] {
        if !is_callable(v) {
            return Err(format!("flow_debate: `{name}` is not callable"));
        }
    }
    // Build wrappers that delegate to the interpreter — closures over
    // `interp` aren't safe across the Step trait, so we keep state out.
    // We implement the loop inline; flow::debate is the reference shape.
    let mut transcript: Vec<axon_flow::Statement> = Vec::new();
    for round in 0..rounds {
        let transcript_v = transcript_to_value(&transcript);
        let pro_text = interp
            .call_value(
                &pro,
                &[Value::String(Rc::new(question.as_str().to_string())), transcript_v.clone()],
                span,
            )
            .map_err(|e| format!("flow_debate[pro:{round}]: {}", eval_signal_msg(&e)))?;
        let pro_s = match &pro_text {
            Value::String(s) => s.as_str().to_string(),
            _ => format!("{}", pro_text),
        };
        transcript.push(axon_flow::Statement {
            side: axon_flow::Side::Pro,
            round,
            text: pro_s,
        });
        let transcript_v = transcript_to_value(&transcript);
        let con_text = interp
            .call_value(
                &con,
                &[Value::String(Rc::new(question.as_str().to_string())), transcript_v],
                span,
            )
            .map_err(|e| format!("flow_debate[con:{round}]: {}", eval_signal_msg(&e)))?;
        let con_s = match &con_text {
            Value::String(s) => s.as_str().to_string(),
            _ => format!("{}", con_text),
        };
        transcript.push(axon_flow::Statement {
            side: axon_flow::Side::Con,
            round,
            text: con_s,
        });
    }
    let transcript_v = transcript_to_value(&transcript);
    let verdict = interp
        .call_value(
            &judge,
            &[Value::String(Rc::new(question.as_str().to_string())), transcript_v.clone()],
            span,
        )
        .map_err(|e| format!("flow_debate[judge]: {}", eval_signal_msg(&e)))?;
    let verdict_s = match &verdict {
        Value::String(s) => s.as_str().to_string(),
        _ => format!("{}", verdict),
    };
    Ok(record_to_vec(vec![
        ("transcript", transcript_v),
        ("verdict", Value::String(Rc::new(verdict_s))),
    ]))
}

fn transcript_to_value(t: &[axon_flow::Statement]) -> Value {
    let items: Vec<Value> = t
        .iter()
        .map(|s| {
            let side = match s.side {
                axon_flow::Side::Pro => "pro",
                axon_flow::Side::Con => "con",
            };
            record_to_vec(vec![
                ("side", Value::String(Rc::new(side.to_string()))),
                ("round", Value::Int(s.round as i64)),
                ("text", Value::String(Rc::new(s.text.clone()))),
            ])
        })
        .collect();
    list_value(items)
}

fn s24_flow_tot(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let seed = args[0].clone();
    let expand = args[1].clone();
    let score = args[2].clone();
    let width = i_arg(args, 3, "flow_tree_of_thought")?.max(1) as usize;
    let depth = i_arg(args, 4, "flow_tree_of_thought")?.max(0) as usize;
    for (name, v) in [("expand", &expand), ("score", &score)] {
        if !is_callable(v) {
            return Err(format!("flow_tree_of_thought: `{name}` is not callable"));
        }
    }
    // Run beam search inline since closures over `interp` can't satisfy
    // the Step trait directly.
    fn score_call(
        interp: &mut Interpreter,
        score: &Value,
        v: &Value,
        span: axon_diag::Span,
    ) -> Result<f64, String> {
        let r = interp
            .call_value(score, &[v.clone()], span)
            .map_err(|e| format!("flow_tree_of_thought[score]: {}", eval_signal_msg(&e)))?;
        match numeric_value(&r) {
            Some(f) if f.is_finite() => Ok(f),
            _ => Ok(f64::NEG_INFINITY),
        }
    }
    let root_score = score_call(interp, &score, &seed, span)?;
    #[derive(Clone)]
    struct ST {
        thought: Value,
        score: f64,
        depth: usize,
    }
    let mut frontier: Vec<ST> = vec![ST {
        thought: seed.clone(),
        score: root_score,
        depth: 0,
    }];
    let mut best = frontier[0].clone();
    let mut expansions = 0usize;
    for d in 1..=depth {
        let mut next: Vec<ST> = Vec::new();
        for parent in &frontier {
            let children_v = interp
                .call_value(
                    &expand,
                    &[parent.thought.clone(), Value::Int((d - 1) as i64)],
                    span,
                )
                .map_err(|e| {
                    format!("flow_tree_of_thought[expand:{d}]: {}", eval_signal_msg(&e))
                })?;
            expansions += 1;
            let children = match children_v {
                Value::List(l) => l.borrow().clone(),
                other => {
                    return Err(format!(
                        "flow_tree_of_thought: expand must return a List, got `{}`",
                        other.type_name()
                    ));
                }
            };
            for child in children {
                let s = score_call(interp, &score, &child, span)?;
                let st = ST {
                    thought: child,
                    score: s,
                    depth: d,
                };
                if s > best.score {
                    best = st.clone();
                }
                next.push(st);
            }
        }
        if next.is_empty() {
            break;
        }
        next.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        next.truncate(width);
        frontier = next;
    }
    let frontier_v = list_value(
        frontier
            .into_iter()
            .map(|st| {
                record_to_vec(vec![
                    ("thought", st.thought),
                    ("score", Value::Float(st.score)),
                    ("depth", Value::Int(st.depth as i64)),
                ])
            })
            .collect(),
    );
    Ok(record_to_vec(vec![
        (
            "best",
            record_to_vec(vec![
                ("thought", best.thought),
                ("score", Value::Float(best.score)),
                ("depth", Value::Int(best.depth as i64)),
            ]),
        ),
        ("frontier", frontier_v),
        ("expansions", Value::Int(expansions as i64)),
    ]))
}

fn s24_flow_race(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let input = args[0].clone();
    let cands = callables_arg(args, 1, "flow_race")?;
    let accept = args[2].clone();
    if !is_callable(&accept) {
        return Err("flow_race: accept must be callable".into());
    }
    if cands.is_empty() {
        return Err("flow_race: no candidates".into());
    }
    let mut last: Option<(usize, Value)> = None;
    for (i, c) in cands.iter().enumerate() {
        if !is_callable(c) {
            return Err(format!("flow_race: candidate {i} is not callable"));
        }
        let r = interp
            .call_value(c, &[input.clone()], span)
            .map_err(|e| format!("flow_race[candidate={i}]: {}", eval_signal_msg(&e)))?;
        let ok = interp
            .call_value(&accept, &[r.clone()], span)
            .map_err(|e| format!("flow_race[accept]: {}", eval_signal_msg(&e)))?;
        let accepted = matches!(ok, Value::Bool(true));
        if accepted {
            return Ok(record_to_vec(vec![
                ("winner_index", Value::Int(i as i64)),
                ("value", r),
                ("considered", Value::Int((i + 1) as i64)),
                ("accepted", Value::Bool(true)),
            ]));
        }
        last = Some((i, r));
    }
    let (i, v) = last.unwrap();
    let considered = cands.len();
    Ok(record_to_vec(vec![
        ("winner_index", Value::Int(i as i64)),
        ("value", v),
        ("considered", Value::Int(considered as i64)),
        ("accepted", Value::Bool(false)),
    ]))
}

fn s24_flow_batch(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let step = args[0].clone();
    if !is_callable(&step) {
        return Err("flow_batch: step must be callable".into());
    }
    let inputs = match &args[1] {
        Value::List(l) => l.borrow().clone(),
        other => {
            return Err(format!(
                "flow_batch: inputs must be a List, got `{}`",
                other.type_name()
            ));
        }
    };
    let mut out: Vec<Value> = Vec::with_capacity(inputs.len());
    for (i, inp) in inputs.into_iter().enumerate() {
        let r = interp
            .call_value(&step, &[inp], span)
            .map_err(|e| format!("flow_batch[{i}]: {}", eval_signal_msg(&e)))?;
        out.push(r);
    }
    Ok(list_value(out))
}

// --------- §56.4 difficulty router ---------

fn s24_estimate_difficulty(args: &[Value]) -> Result<Value, String> {
    let prompt = s_arg(args, 0, "flow_estimate_difficulty")?;
    let t = DifficultyThresholds::default();
    let d = axon_flow::estimate_difficulty(prompt.as_str(), &t);
    let name = match d {
        axon_flow::Difficulty::Trivial => "trivial",
        axon_flow::Difficulty::Normal => "normal",
        axon_flow::Difficulty::Hard => "hard",
    };
    Ok(Value::String(Rc::new(name.to_string())))
}

fn s24_route_difficulty(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let prompt = s_arg(args, 0, "flow_route_difficulty")?;
    let trivial = args[1].clone();
    let normal = args[2].clone();
    let hard = args[3].clone();
    for (n_, v) in [("trivial", &trivial), ("normal", &normal), ("hard", &hard)] {
        if !is_callable(v) {
            return Err(format!("flow_route_difficulty: `{n_}` is not callable"));
        }
    }
    let t = DifficultyThresholds::default();
    let tier = axon_flow::estimate_difficulty(prompt.as_str(), &t);
    let (tier_name, step) = match tier {
        axon_flow::Difficulty::Trivial => ("trivial", &trivial),
        axon_flow::Difficulty::Normal => ("normal", &normal),
        axon_flow::Difficulty::Hard => ("hard", &hard),
    };
    let v = interp
        .call_value(step, &[Value::String(prompt)], span)
        .map_err(|e| format!("flow_route_difficulty[{tier_name}]: {}", eval_signal_msg(&e)))?;
    Ok(record_to_vec(vec![
        ("tier", Value::String(Rc::new(tier_name.to_string()))),
        ("value", v),
    ]))
}

// --------- §49.1 reasoning budgets ---------

fn s24_reasoning_budget_new(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "reasoning_budget_new")?;
    let effort_s = s_arg(args, 1, "reasoning_budget_new")?;
    let max_thinking = i_arg(args, 2, "reasoning_budget_new")?.max(0) as u64;
    let expose = b_arg(args, 3, "reasoning_budget_new")?;
    let effort = match effort_s.as_str() {
        "low" => axon_runtime::Effort::Low,
        "medium" => axon_runtime::Effort::Medium,
        "high" => axon_runtime::Effort::High,
        "adaptive" => axon_runtime::Effort::Adaptive,
        other => {
            return Err(format!(
                "reasoning_budget_new: effort must be low|medium|high|adaptive, got `{other}`"
            ));
        }
    };
    REASONING_BUDGETS.with(|c| {
        c.borrow_mut().insert(
            id.as_str().to_string(),
            axon_runtime::ReasoningBudget::new(effort, max_thinking, expose),
        )
    });
    Ok(Value::String(id))
}

fn s24_reasoning_budget_debit(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "reasoning_budget_debit")?;
    let tokens = i_arg(args, 1, "reasoning_budget_debit")?.max(0) as u64;
    let breach = REASONING_BUDGETS.with(|c| {
        let mut map = c.borrow_mut();
        let b = map
            .get_mut(id.as_str())
            .ok_or_else(|| format!("reasoning_budget_debit: no budget `{}`", id.as_str()))?;
        Ok::<_, String>(b.debit(tokens))
    })?;
    Ok(Value::Bool(breach.is_some()))
}

fn s24_reasoning_budget_status(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "reasoning_budget_status")?;
    REASONING_BUDGETS.with(|c| {
        let map = c.borrow();
        let b = map
            .get(id.as_str())
            .ok_or_else(|| format!("reasoning_budget_status: no budget `{}`", id.as_str()))?;
        let effort_s = match b.effort {
            axon_runtime::Effort::Low => "low",
            axon_runtime::Effort::Medium => "medium",
            axon_runtime::Effort::High => "high",
            axon_runtime::Effort::Adaptive => "adaptive",
        };
        Ok::<Value, String>(record_to_vec(vec![
            ("spent", Value::Int(b.spent_thinking_tokens as i64)),
            ("max", Value::Int(b.max_thinking_tokens as i64)),
            (
                "remaining",
                Value::Int(b.remaining().min(i64::MAX as u64) as i64),
            ),
            ("breached", Value::Bool(b.breach().is_some())),
            ("effort", Value::String(Rc::new(effort_s.to_string()))),
            ("expose", Value::Bool(b.expose)),
        ]))
    })
}

// --------- §49.2 ReAct loop driver ---------

/// `plan_react_loop(max_steps, think, act, observe)` — typed ReAct driver
/// implementing think -> act -> observe. `observe(thought, action)` must
/// return a record `{ observation: String, done: Bool }`. Returns a list
/// of step records.
fn s24_plan_react_loop(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let max_steps = i_arg(args, 0, "plan_react_loop")?.max(0) as usize;
    let think = args[1].clone();
    let act = args[2].clone();
    let observe = args[3].clone();
    for (n_, v) in [("think", &think), ("act", &act), ("observe", &observe)] {
        if !is_callable(v) {
            return Err(format!("plan_react_loop: `{n_}` is not callable"));
        }
    }
    let mut log: Vec<Value> = Vec::new();
    for step in 0..max_steps {
        let log_v = list_value(log.clone());
        let thought = interp
            .call_value(&think, &[log_v.clone()], span)
            .map_err(|e| format!("plan_react_loop[think:{step}]: {}", eval_signal_msg(&e)))?;
        let action = interp
            .call_value(&act, &[thought.clone(), log_v.clone()], span)
            .map_err(|e| format!("plan_react_loop[act:{step}]: {}", eval_signal_msg(&e)))?;
        let obs_rec = interp
            .call_value(&observe, &[thought.clone(), action.clone()], span)
            .map_err(|e| {
                format!("plan_react_loop[observe:{step}]: {}", eval_signal_msg(&e))
            })?;
        let (obs, done) = match &obs_rec {
            Value::Record(r) => {
                let r = r.borrow();
                let obs = r
                    .iter()
                    .find(|(k, _)| k == "observation")
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::String(Rc::new(String::new())));
                let done = r
                    .iter()
                    .find(|(k, _)| k == "done")
                    .map(|(_, v)| matches!(v, Value::Bool(true)))
                    .unwrap_or(false);
                (obs, done)
            }
            other => {
                return Err(format!(
                    "plan_react_loop: observe must return Record{{observation, done}}, got `{}`",
                    other.type_name()
                ));
            }
        };
        log.push(record_to_vec(vec![
            ("step_index", Value::Int(step as i64)),
            ("thought", thought),
            ("action", action),
            ("observation", obs),
        ]));
        if done {
            break;
        }
    }
    Ok(list_value(log))
}

// --------- §55.1 trajectory eval ---------

fn s24_traj_new(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "eval_trajectory_new")?;
    let task = s_arg(args, 1, "eval_trajectory_new")?;
    let allowed = list_of_strings(&args[2], "eval_trajectory_new", "allowed_tools")?;
    let forbidden = list_of_strings(&args[3], "eval_trajectory_new", "forbidden_tools")?;
    TRAJECTORIES.with(|c| {
        c.borrow_mut().insert(
            id.as_str().to_string(),
            traj::Trajectory {
                task: task.as_str().to_string(),
                steps: Vec::new(),
                answer: String::new(),
                allowed_tools: allowed,
                forbidden_tools: forbidden,
                optimal_steps: 0,
            },
        )
    });
    Ok(Value::String(id))
}

fn s24_traj_add_step(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "eval_trajectory_add_step")?;
    let thought = s_arg(args, 1, "eval_trajectory_add_step")?;
    let tool_name = s_arg(args, 2, "eval_trajectory_add_step")?;
    let args_json = s_arg(args, 3, "eval_trajectory_add_step")?;
    let errored = b_arg(args, 4, "eval_trajectory_add_step")?;
    let observation = s_arg(args, 5, "eval_trajectory_add_step")?;
    TRAJECTORIES.with(|c| {
        let mut map = c.borrow_mut();
        let t = map
            .get_mut(id.as_str())
            .ok_or_else(|| format!("eval_trajectory_add_step: no trajectory `{}`", id.as_str()))?;
        let tool_call = if tool_name.is_empty() {
            None
        } else {
            Some(traj::ToolCall {
                name: tool_name.as_str().to_string(),
                args_json: args_json.as_str().to_string(),
                errored,
            })
        };
        let idx = t.steps.len();
        t.steps.push(traj::TrajectoryStep {
            index: idx,
            thought: thought.as_str().to_string(),
            tool_call,
            observation: observation.as_str().to_string(),
            error: None,
        });
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s24_traj_set_answer(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "eval_trajectory_set_answer")?;
    let ans = s_arg(args, 1, "eval_trajectory_set_answer")?;
    TRAJECTORIES.with(|c| {
        let mut map = c.borrow_mut();
        let t = map
            .get_mut(id.as_str())
            .ok_or_else(|| format!("eval_trajectory_set_answer: no trajectory `{}`", id.as_str()))?;
        t.answer = ans.as_str().to_string();
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s24_traj_tool_accuracy(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "eval_trajectory_tool_accuracy")?;
    let v = TRAJECTORIES.with(|c| {
        c.borrow()
            .get(id.as_str())
            .map(traj::tool_accuracy)
            .unwrap_or(0.0)
    });
    Ok(Value::Float(v))
}

fn s24_traj_step_efficiency(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "eval_trajectory_step_efficiency")?;
    let v = TRAJECTORIES.with(|c| {
        c.borrow()
            .get(id.as_str())
            .map(traj::step_efficiency)
            .unwrap_or(0.0)
    });
    Ok(Value::Float(v))
}

fn s24_traj_recovered(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "eval_trajectory_recovered")?;
    let v = TRAJECTORIES.with(|c| {
        c.borrow()
            .get(id.as_str())
            .map(traj::recovered_from_errors)
            .unwrap_or(false)
    });
    Ok(Value::Bool(v))
}

fn s24_traj_no_forbidden(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "eval_trajectory_no_forbidden_tool")?;
    let v = TRAJECTORIES.with(|c| {
        c.borrow()
            .get(id.as_str())
            .map(traj::no_forbidden_tool_called)
            .unwrap_or(true)
    });
    Ok(Value::Bool(v))
}

fn s24_traj_grounded(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "eval_trajectory_grounded")?;
    let v = TRAJECTORIES.with(|c| {
        c.borrow()
            .get(id.as_str())
            .map(traj::grounded_in_observations)
            .unwrap_or(1.0)
    });
    Ok(Value::Float(v))
}

fn s24_traj_no_secret(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "eval_trajectory_no_secret_exposed")?;
    let secrets = list_of_strings(&args[1], "eval_trajectory_no_secret_exposed", "secrets")?;
    let v = TRAJECTORIES.with(|c| {
        c.borrow()
            .get(id.as_str())
            .map(|t| traj::no_secret_exposed(t, &secrets))
            .unwrap_or(true)
    });
    Ok(Value::Bool(v))
}

// --------- §55.2 redteam ---------

fn s24_redteam_load(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "redteam_load")?;
    let cases = rt_suite::redteam_suite(name.as_str());
    let items: Vec<Value> = cases
        .into_iter()
        .map(|c| {
            let cat = match c.category {
                rt_suite::AttackCategory::PromptInjection => "prompt_injection",
                rt_suite::AttackCategory::Jailbreak => "jailbreak",
                rt_suite::AttackCategory::ToolAbuse => "tool_abuse",
                rt_suite::AttackCategory::Exfiltration => "exfiltration",
                rt_suite::AttackCategory::PiiTrap => "pii_trap",
            };
            let assertion_str = match c.assertion {
                rt_suite::SafetyAssertion::NoToolCalled { .. } => "no_tool_called",
                rt_suite::SafetyAssertion::NoSecretExposed => "no_secret_exposed",
                rt_suite::SafetyAssertion::AnswerOmitsCanary => "answer_omits_canary",
                rt_suite::SafetyAssertion::Refuses => "refuses",
            };
            record_to_vec(vec![
                ("id", Value::String(Rc::new(c.id))),
                ("category", Value::String(Rc::new(cat.to_string()))),
                ("payload", Value::String(Rc::new(c.payload))),
                (
                    "watched_tools",
                    list_value(
                        c.watched_tools
                            .into_iter()
                            .map(|s| Value::String(Rc::new(s)))
                            .collect(),
                    ),
                ),
                (
                    "secrets",
                    list_value(
                        c.secrets
                            .into_iter()
                            .map(|s| Value::String(Rc::new(s)))
                            .collect(),
                    ),
                ),
                ("assertion", Value::String(Rc::new(assertion_str.to_string()))),
            ])
        })
        .collect();
    Ok(list_value(items))
}

fn s24_redteam_refusal_phrases(_args: &[Value]) -> Result<Value, String> {
    let phrases: Vec<Value> = rt_suite::refusal_phrases()
        .iter()
        .map(|p| Value::String(Rc::new((*p).to_string())))
        .collect();
    Ok(list_value(phrases))
}

// --------- §55.3 sim world ---------

fn s24_sim_new(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "sim_world_new")?;
    let seed = i_arg(args, 1, "sim_world_new")?;
    SIM_WORLDS.with(|c| {
        c.borrow_mut().insert(
            id.as_str().to_string(),
            eval_sim::World::new(seed as u64, 0),
        )
    });
    Ok(Value::String(id))
}

fn s24_sim_spawn(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "sim_world_spawn")?;
    let name = s_arg(args, 1, "sim_world_spawn")?;
    SIM_WORLDS.with(|c| {
        let mut map = c.borrow_mut();
        let w = map
            .get_mut(id.as_str())
            .ok_or_else(|| format!("sim_world_spawn: no world `{}`", id.as_str()))?;
        w.spawn(name.as_str(), Vec::new());
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s24_sim_script_send(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "sim_world_script_send")?;
    let agent = s_arg(args, 1, "sim_world_script_send")?;
    let to = s_arg(args, 2, "sim_world_script_send")?;
    let payload = s_arg(args, 3, "sim_world_script_send")?;
    SIM_WORLDS.with(|c| {
        let mut map = c.borrow_mut();
        let w = map
            .get_mut(id.as_str())
            .ok_or_else(|| format!("sim_world_script_send: no world `{}`", id.as_str()))?;
        let a = w
            .agents
            .iter_mut()
            .find(|a| a.name == agent.as_str())
            .ok_or_else(|| {
                format!("sim_world_script_send: no agent `{}`", agent.as_str())
            })?;
        a.script.push(eval_sim::ScriptedAction::Send {
            to: to.as_str().to_string(),
            payload: payload.as_str().to_string(),
        });
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s24_sim_script_note(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "sim_world_script_note")?;
    let agent = s_arg(args, 1, "sim_world_script_note")?;
    let kind = s_arg(args, 2, "sim_world_script_note")?;
    let payload = s_arg(args, 3, "sim_world_script_note")?;
    SIM_WORLDS.with(|c| {
        let mut map = c.borrow_mut();
        let w = map
            .get_mut(id.as_str())
            .ok_or_else(|| format!("sim_world_script_note: no world `{}`", id.as_str()))?;
        let a = w
            .agents
            .iter_mut()
            .find(|a| a.name == agent.as_str())
            .ok_or_else(|| {
                format!("sim_world_script_note: no agent `{}`", agent.as_str())
            })?;
        a.script.push(eval_sim::ScriptedAction::Note {
            kind: kind.as_str().to_string(),
            payload: payload.as_str().to_string(),
        });
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s24_sim_script_settle(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "sim_world_script_settle")?;
    let agent = s_arg(args, 1, "sim_world_script_settle")?;
    SIM_WORLDS.with(|c| {
        let mut map = c.borrow_mut();
        let w = map
            .get_mut(id.as_str())
            .ok_or_else(|| format!("sim_world_script_settle: no world `{}`", id.as_str()))?;
        let a = w
            .agents
            .iter_mut()
            .find(|a| a.name == agent.as_str())
            .ok_or_else(|| {
                format!("sim_world_script_settle: no agent `{}`", agent.as_str())
            })?;
        a.script.push(eval_sim::ScriptedAction::Settle);
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s24_sim_send_to(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "sim_world_send_to")?;
    let agent = s_arg(args, 1, "sim_world_send_to")?;
    let payload = s_arg(args, 2, "sim_world_send_to")?;
    SIM_WORLDS.with(|c| {
        let mut map = c.borrow_mut();
        let w = map
            .get_mut(id.as_str())
            .ok_or_else(|| format!("sim_world_send_to: no world `{}`", id.as_str()))?;
        w.send_to(agent.as_str(), payload.as_str().to_string())
    })?;
    Ok(Value::Unit)
}

fn s24_sim_advance(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "sim_world_advance")?;
    let dt_ns = i_arg(args, 1, "sim_world_advance")?.max(0) as u64;
    let n = SIM_WORLDS.with(|c| {
        let mut map = c.borrow_mut();
        let w = map
            .get_mut(id.as_str())
            .ok_or_else(|| format!("sim_world_advance: no world `{}`", id.as_str()))?;
        Ok::<_, String>(w.advance(dt_ns))
    })?;
    Ok(Value::Int(n as i64))
}

fn s24_sim_run_until_settled(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "sim_world_run_until_settled")?;
    let agent_name = s_arg(args, 1, "sim_world_run_until_settled")?;
    let dt_ns = i_arg(args, 2, "sim_world_run_until_settled")?.max(0) as u64;
    let max_ticks = i_arg(args, 3, "sim_world_run_until_settled")?.max(0) as usize;
    let hit = SIM_WORLDS.with(|c| {
        let mut map = c.borrow_mut();
        let w = map
            .get_mut(id.as_str())
            .ok_or_else(|| format!("sim_world_run_until_settled: no world `{}`", id.as_str()))?;
        let target = agent_name.as_str().to_string();
        Ok::<_, String>(w.run_until(dt_ns, max_ticks, |w| {
            w.agent_settled(&target)
        }))
    })?;
    Ok(Value::Bool(hit))
}

fn s24_sim_events(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "sim_world_events")?;
    let evts: Vec<Value> = SIM_WORLDS.with(|c| {
        c.borrow()
            .get(id.as_str())
            .map(|w| w.events.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|e| {
                record_to_vec(vec![
                    ("step", Value::Int(e.step as i64)),
                    ("at_ns", Value::Int(e.at_ns as i64)),
                    ("agent", Value::String(Rc::new(e.agent))),
                    ("action", Value::String(Rc::new(e.action))),
                    ("payload", Value::String(Rc::new(e.payload))),
                ])
            })
            .collect()
    });
    Ok(list_value(evts))
}

fn s24_sim_rand(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "sim_world_rand_u64")?;
    let v = SIM_WORLDS.with(|c| {
        let mut map = c.borrow_mut();
        let w = map
            .get_mut(id.as_str())
            .ok_or_else(|| format!("sim_world_rand_u64: no world `{}`", id.as_str()))?;
        Ok::<_, String>(w.rand_u64())
    })?;
    // Mask to i64 range for Axon's Int.
    Ok(Value::Int((v as i64).wrapping_abs()))
}

// --------- §56.1 prefix cache ---------

fn s24_cache_insert(args: &[Value]) -> Result<Value, String> {
    let text = s_arg(args, 0, "cost_cache_insert")?;
    let tokens = i_arg(args, 1, "cost_cache_insert")?.max(0) as u32;
    let ttl = i_arg(args, 2, "cost_cache_insert")?.max(0);
    let key = PREFIX_CACHE.with(|c| c.insert(text.as_str(), tokens, now_ns(), ttl));
    Ok(Value::String(Rc::new(format!("{:016x}", key.0))))
}

fn s24_cache_lookup(args: &[Value]) -> Result<Value, String> {
    let text = s_arg(args, 0, "cost_cache_lookup")?;
    let r = PREFIX_CACHE.with(|c| c.lookup(text.as_str(), now_ns()));
    Ok(match r {
        Some((tokens, hits)) => record_to_vec(vec![
            ("hit", Value::Bool(true)),
            ("tokens", Value::Int(tokens as i64)),
            ("hits", Value::Int(hits as i64)),
        ]),
        None => record_to_vec(vec![
            ("hit", Value::Bool(false)),
            ("tokens", Value::Int(0)),
            ("hits", Value::Int(0)),
        ]),
    })
}

fn s24_cache_stats(_args: &[Value]) -> Result<Value, String> {
    let s = PREFIX_CACHE.with(|c| c.stats());
    Ok(record_to_vec(vec![
        ("lookups", Value::Int(s.lookups as i64)),
        ("hits", Value::Int(s.hits as i64)),
        ("misses", Value::Int(s.misses as i64)),
        ("tokens_saved", Value::Int(s.tokens_saved as i64)),
        ("entries", Value::Int(s.entries as i64)),
        ("hit_rate", Value::Float(s.hit_rate())),
    ]))
}

fn s24_cache_clear(_args: &[Value]) -> Result<Value, String> {
    PREFIX_CACHE.with(|c| c.clear());
    Ok(Value::Unit)
}

// ===========================================================================
// Stage 25 — context policy (§27.3), saga (§52), durable timers (§52.2),
// RAG grounding (§50.2/50.3), media generation (§51.2/51.3), skill_use
// (§53), agent-card auto-publish (§54.1), /metrics + serverless render
// (§41). C ABI (§35.4) is a separate cdylib + axvm.h header.
// ===========================================================================

use axon_a2a::auto_publish as a2a_auto;
use axon_deploy::{metrics::MetricsRegistry, serverless as svless};
use axon_flow::{run_saga as flow_run_saga, SagaStep, StepState};
use axon_media::generate as media_gen;
use axon_rag::grounding as rag_ground;
use axon_runtime::context_policy as ctx;
use axon_skill::use_skill as sk_use;
use axon_trigger::durable_timer as dtimer;

thread_local! {
    static TIMER_TABLE: RefCell<dtimer::DurableTimerTable> =
        RefCell::new(dtimer::DurableTimerTable::new());
    static METRICS: std::sync::Arc<MetricsRegistry> = MetricsRegistry::new();
    static MEDIA_PROVIDER: RefCell<Box<dyn media_gen::MediaProvider + Send>> =
        RefCell::new(Box::new(media_gen::MockProvider::new("default")));
}

fn install_stage25(interp: &Interpreter) {
    // ---- §27.3 context policy ----
    interp.register_native(
        "context_policy_plan",
        n("context_policy_plan", 2, Some(2), s25_ctx_plan),
    );

    // ---- §52 saga ----
    interp.register_native_ext(
        "flow_saga_run",
        ext("flow_saga_run", 3, Some(3), s25_saga_run),
    );

    // ---- §52.2 durable timers ----
    interp.register_native(
        "timer_arm",
        n("timer_arm", 4, Some(4), s25_timer_arm),
    );
    interp.register_native(
        "timer_cancel",
        n("timer_cancel", 1, Some(1), s25_timer_cancel),
    );
    interp.register_native(
        "timer_due",
        n("timer_due", 1, Some(1), s25_timer_due),
    );
    interp.register_native(
        "timer_mark_fired",
        n("timer_mark_fired", 1, Some(1), s25_timer_mark_fired),
    );
    interp.register_native(
        "timer_pending_count",
        n("timer_pending_count", 0, Some(0), s25_timer_pending_count),
    );
    interp.register_native(
        "timer_save",
        n("timer_save", 1, Some(1), s25_timer_save),
    );
    interp.register_native(
        "timer_load",
        n("timer_load", 1, Some(1), s25_timer_load),
    );

    // ---- §50.2 / §50.3 RAG grounding ----
    interp.register_native(
        "rag_assess_grounding",
        n("rag_assess_grounding", 4, Some(4), s25_rag_assess),
    );

    // ---- §51.2 / §51.3 media generation ----
    interp.register_native(
        "media_generate_image",
        n("media_generate_image", 6, Some(6), s25_media_image),
    );
    interp.register_native(
        "media_generate_audio",
        n("media_generate_audio", 5, Some(5), s25_media_audio),
    );

    // ---- §53 skill use ----
    interp.register_native(
        "skill_bind",
        n("skill_bind", 3, Some(3), s25_skill_bind),
    );
    interp.register_native(
        "skill_narrow_effects",
        n("skill_narrow_effects", 2, Some(2), s25_skill_narrow),
    );

    // ---- §54.1 agent-card auto publish ----
    interp.register_native(
        "agent_card_derive",
        n("agent_card_derive", 2, Some(2), s25_card_derive),
    );
    interp.register_native(
        "agent_card_well_known_path",
        n(
            "agent_card_well_known_path",
            0,
            Some(0),
            s25_well_known_path,
        ),
    );

    // ---- §41 metrics + serverless ----
    interp.register_native(
        "metrics_record",
        n("metrics_record", 4, Some(4), s25_metrics_record),
    );
    interp.register_native(
        "metrics_render_prometheus",
        n(
            "metrics_render_prometheus",
            0,
            Some(0),
            s25_metrics_render,
        ),
    );
    interp.register_native(
        "serverless_render",
        n("serverless_render", 3, Some(3), s25_serverless_render),
    );
}

// --------- §27.3 ---------

fn s25_ctx_plan(args: &[Value]) -> Result<Value, String> {
    // Args: (policy_record, messages_list)
    let policy_json = value_to_json(&args[0]);
    let policy: ctx::ContextPolicy = serde_json::from_value(policy_json)
        .map_err(|e| format!("context_policy_plan: bad policy: {e}"))?;
    let msgs_v = match &args[1] {
        Value::List(l) => l.borrow().clone(),
        other => {
            return Err(format!(
                "context_policy_plan: messages must be a List, got `{}`",
                other.type_name()
            ));
        }
    };
    let mut messages: Vec<ctx::Message> = Vec::with_capacity(msgs_v.len());
    for (i, m) in msgs_v.into_iter().enumerate() {
        let m_json = value_to_json(&m);
        let mut msg: ctx::Message = serde_json::from_value(m_json)
            .map_err(|e| format!("context_policy_plan: msg #{i}: {e}"))?;
        if msg.tokens == 0 {
            msg.tokens = ctx::estimate_tokens(&msg.text);
        }
        if msg.seq == 0 {
            msg.seq = i as u64;
        }
        messages.push(msg);
    }
    let outcome = policy
        .plan(&messages)
        .map_err(|e| format!("context_policy_plan: {e}"))?;
    let j = serde_json::to_value(&outcome)
        .map_err(|e| format!("context_policy_plan: encode: {e}"))?;
    Ok(json_to_value(&j))
}

// --------- §52 saga ---------

fn s25_saga_run(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let input = args[0].clone();
    let names = list_of_strings(&args[1], "flow_saga_run", "step names")?;
    let actions = match &args[2] {
        Value::List(l) => l.borrow().clone(),
        other => {
            return Err(format!(
                "flow_saga_run: actions must be a List of records, got `{}`",
                other.type_name()
            ));
        }
    };
    if names.len() != actions.len() {
        return Err(format!(
            "flow_saga_run: {} names vs {} actions",
            names.len(),
            actions.len()
        ));
    }

    // Walk forward; on failure, walk compensations in reverse.
    #[derive(Clone)]
    struct StepLog {
        name: String,
        state: StepState,
        message: String,
    }
    let mut trail: Vec<StepLog> = names
        .iter()
        .map(|n| StepLog {
            name: n.clone(),
            state: StepState::Skipped,
            message: String::new(),
        })
        .collect();

    // Each entry is a record `{ action: fn, compensate: fn|nil }`.
    let parsed: Vec<(Value, Option<Value>)> = actions
        .into_iter()
        .enumerate()
        .map(|(i, v)| match v {
            Value::Record(r) => {
                let r = r.borrow();
                let action = r
                    .iter()
                    .find(|(k, _)| k == "action")
                    .map(|(_, v)| v.clone())
                    .ok_or_else(|| {
                        format!("flow_saga_run: actions[{i}] missing `action`")
                    })?;
                let comp = r
                    .iter()
                    .find(|(k, _)| k == "compensate")
                    .map(|(_, v)| v.clone())
                    .filter(|v| !matches!(v, Value::Nil | Value::Unit));
                Ok::<_, String>((action, comp))
            }
            other => Err(format!(
                "flow_saga_run: actions[{i}] must be a record, got `{}`",
                other.type_name()
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut succeeded: Vec<(usize, Value)> = Vec::new();
    let mut failed_at: Option<usize> = None;
    for (i, (action, _comp)) in parsed.iter().enumerate() {
        if !is_callable(action) {
            return Err(format!("flow_saga_run: actions[{i}].action not callable"));
        }
        match interp.call_value(action, &[input.clone()], span) {
            Ok(v) => {
                trail[i].state = StepState::Succeeded;
                succeeded.push((i, v));
            }
            Err(sig) => {
                trail[i].state = StepState::Failed;
                trail[i].message = eval_signal_msg(&sig);
                failed_at = Some(i);
                break;
            }
        }
    }

    let mut status = "committed";
    let mut any_comp_failed = false;
    if let Some(_fail_idx) = failed_at {
        status = "compensated";
        for (i, value) in succeeded.into_iter().rev() {
            let Some(comp) = parsed[i].1.clone() else {
                continue;
            };
            if !is_callable(&comp) {
                return Err(format!(
                    "flow_saga_run: actions[{i}].compensate not callable"
                ));
            }
            match interp.call_value(&comp, &[value], span) {
                Ok(_) => trail[i].state = StepState::Compensated,
                Err(sig) => {
                    any_comp_failed = true;
                    trail[i].state = StepState::CompensationFailed;
                    trail[i].message = eval_signal_msg(&sig);
                }
            }
        }
        if any_comp_failed {
            status = "aborted";
        }
    }

    let trail_v = list_value(
        trail
            .into_iter()
            .map(|r| {
                let state_s = match r.state {
                    StepState::Succeeded => "succeeded",
                    StepState::Failed => "failed",
                    StepState::Compensated => "compensated",
                    StepState::CompensationFailed => "compensation_failed",
                    StepState::Skipped => "skipped",
                };
                record_to_vec(vec![
                    ("name", Value::String(Rc::new(r.name))),
                    ("state", Value::String(Rc::new(state_s.to_string()))),
                    ("message", Value::String(Rc::new(r.message))),
                ])
            })
            .collect(),
    );
    Ok(record_to_vec(vec![
        ("status", Value::String(Rc::new(status.to_string()))),
        ("trail", trail_v),
    ]))
}

// Keep flow_run_saga / SagaStep referenced (and silence warnings) so the
// Rust API stays exposed for callers that link the crate directly.
#[allow(dead_code)]
fn _saga_keep_alive() {
    let act = |_: i64| Ok::<_, axon_flow::FlowError>(1i64);
    let comp = |_: i64| Ok::<_, axon_flow::FlowError>(());
    let steps = vec![SagaStep::new("x", &act).with_compensation(&comp)];
    let _ = flow_run_saga(0i64, steps);
}

// --------- §52.2 durable timers ---------

fn s25_timer_arm(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "timer_arm")?;
    let name = s_arg(args, 1, "timer_arm")?;
    let deadline_ns = i_arg(args, 2, "timer_arm")?;
    let payload = s_arg(args, 3, "timer_arm")?;
    let armed_ns = now_ns();
    TIMER_TABLE.with(|c| {
        c.borrow_mut().arm(dtimer::DurableTimer {
            id: id.as_str().to_string(),
            name: name.as_str().to_string(),
            deadline_ns,
            armed_ns,
            fired: false,
            cancelled: false,
            payload: payload.as_str().to_string(),
        })
    })?;
    Ok(Value::String(id))
}

fn s25_timer_cancel(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "timer_cancel")?;
    let cancelled = TIMER_TABLE.with(|c| c.borrow_mut().cancel(id.as_str()));
    Ok(Value::Bool(cancelled))
}

fn s25_timer_due(args: &[Value]) -> Result<Value, String> {
    let now = i_arg(args, 0, "timer_due")?;
    let ids = TIMER_TABLE.with(|c| c.borrow().due(now));
    Ok(list_value(
        ids.into_iter().map(|s| Value::String(Rc::new(s))).collect(),
    ))
}

fn s25_timer_mark_fired(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "timer_mark_fired")?;
    let ok = TIMER_TABLE.with(|c| c.borrow_mut().mark_fired(id.as_str()));
    Ok(Value::Bool(ok))
}

fn s25_timer_pending_count(_args: &[Value]) -> Result<Value, String> {
    let n = TIMER_TABLE.with(|c| c.borrow().pending_count());
    Ok(Value::Int(n as i64))
}

fn s25_timer_save(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "timer_save")?;
    let bytes = TIMER_TABLE.with(|c| serde_json::to_vec_pretty(&*c.borrow()))
        .map_err(|e| format!("timer_save: encode: {e}"))?;
    std::fs::write(path.as_str(), bytes)
        .map_err(|e| format!("timer_save: write `{}`: {e}", path.as_str()))?;
    Ok(Value::Unit)
}

fn s25_timer_load(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "timer_load")?;
    let bytes = std::fs::read(path.as_str())
        .map_err(|e| format!("timer_load: read `{}`: {e}", path.as_str()))?;
    let tbl: dtimer::DurableTimerTable = serde_json::from_slice(&bytes)
        .map_err(|e| format!("timer_load: parse: {e}"))?;
    TIMER_TABLE.with(|c| *c.borrow_mut() = tbl);
    let n = TIMER_TABLE.with(|c| c.borrow().pending_count());
    Ok(Value::Int(n as i64))
}

// --------- §50.2 / §50.3 RAG grounding ---------

fn s25_rag_assess(args: &[Value]) -> Result<Value, String> {
    let answer = s_arg(args, 0, "rag_assess_grounding")?;
    let passages = match &args[1] {
        Value::List(l) => l.borrow().clone(),
        other => {
            return Err(format!(
                "rag_assess_grounding: passages must be a List, got `{}`",
                other.type_name()
            ));
        }
    };
    let citations = match &args[2] {
        Value::List(l) => l.borrow().clone(),
        other => {
            return Err(format!(
                "rag_assess_grounding: citations must be a List, got `{}`",
                other.type_name()
            ));
        }
    };
    let cfg_v = value_to_json(&args[3]);
    let cfg: rag_ground::GroundingConfig =
        serde_json::from_value(cfg_v).unwrap_or_default();

    let ps: Vec<rag_ground::CitationPassage> = passages
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            let j = value_to_json(&v);
            serde_json::from_value(j)
                .map_err(|e| format!("rag_assess_grounding: passages[{i}]: {e}"))
        })
        .collect::<Result<_, _>>()?;
    let cs: Vec<rag_ground::Citation> = citations
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            let j = value_to_json(&v);
            serde_json::from_value(j)
                .map_err(|e| format!("rag_assess_grounding: citations[{i}]: {e}"))
        })
        .collect::<Result<_, _>>()?;
    let report = rag_ground::assess_grounding(answer.as_str(), &ps, &cs, &cfg);
    let j = serde_json::to_value(&report)
        .map_err(|e| format!("rag_assess_grounding: encode: {e}"))?;
    Ok(json_to_value(&j))
}

// --------- §51.2 / §51.3 media generation ---------

fn s25_media_image(args: &[Value]) -> Result<Value, String> {
    let prompt = s_arg(args, 0, "media_generate_image")?;
    let width = i_arg(args, 1, "media_generate_image")?.max(0) as u32;
    let height = i_arg(args, 2, "media_generate_image")?.max(0) as u32;
    let format_s = s_arg(args, 3, "media_generate_image")?;
    let seed = i_arg(args, 4, "media_generate_image")?.max(0) as u64;
    let n = i_arg(args, 5, "media_generate_image")?.max(1) as u32;
    let format = match format_s.as_str() {
        "png" => media_gen::GenImageFormat::Png,
        "jpeg" | "jpg" => media_gen::GenImageFormat::Jpeg,
        "webp" => media_gen::GenImageFormat::Webp,
        other => {
            return Err(format!(
                "media_generate_image: unsupported format `{other}`"
            ));
        }
    };
    let req = media_gen::GenerateImageRequest {
        prompt: prompt.as_str().to_string(),
        width,
        height,
        format,
        negative_prompt: String::new(),
        seed,
        n,
    };
    let out = MEDIA_PROVIDER
        .with(|cell| cell.borrow().generate_image(&req))
        .map_err(|e| format!("media_generate_image: {e}"))?;
    let images: Vec<Value> = out
        .into_iter()
        .map(|g| {
            let fmt_s = match g.format {
                media_gen::GenImageFormat::Png => "png",
                media_gen::GenImageFormat::Jpeg => "jpeg",
                media_gen::GenImageFormat::Webp => "webp",
            };
            record_to_vec(vec![
                ("bytes_len", Value::Int(g.bytes.len() as i64)),
                ("format", Value::String(Rc::new(fmt_s.to_string()))),
                ("width", Value::Int(g.width as i64)),
                ("height", Value::Int(g.height as i64)),
                ("provider_id", Value::String(Rc::new(g.provider_id))),
            ])
        })
        .collect();
    Ok(list_value(images))
}

fn s25_media_audio(args: &[Value]) -> Result<Value, String> {
    let prompt = s_arg(args, 0, "media_generate_audio")?;
    let voice = s_arg(args, 1, "media_generate_audio")?;
    let sample_rate = i_arg(args, 2, "media_generate_audio")?.max(0) as u32;
    let format_s = s_arg(args, 3, "media_generate_audio")?;
    let seed = i_arg(args, 4, "media_generate_audio")?.max(0) as u64;
    let format = match format_s.as_str() {
        "mp3" => media_gen::GenAudioFormat::Mp3,
        "wav" => media_gen::GenAudioFormat::Wav,
        "flac" => media_gen::GenAudioFormat::Flac,
        "opus" => media_gen::GenAudioFormat::Opus,
        other => {
            return Err(format!(
                "media_generate_audio: unsupported format `{other}`"
            ));
        }
    };
    let req = media_gen::GenerateAudioRequest {
        prompt: prompt.as_str().to_string(),
        voice: voice.as_str().to_string(),
        sample_rate,
        format,
        max_duration_secs: 0,
        seed,
    };
    let out = MEDIA_PROVIDER
        .with(|cell| cell.borrow().generate_audio(&req))
        .map_err(|e| format!("media_generate_audio: {e}"))?;
    let fmt_s = match out.format {
        media_gen::GenAudioFormat::Mp3 => "mp3",
        media_gen::GenAudioFormat::Wav => "wav",
        media_gen::GenAudioFormat::Flac => "flac",
        media_gen::GenAudioFormat::Opus => "opus",
    };
    Ok(record_to_vec(vec![
        ("bytes_len", Value::Int(out.bytes.len() as i64)),
        ("format", Value::String(Rc::new(fmt_s.to_string()))),
        ("sample_rate", Value::Int(out.sample_rate as i64)),
        ("duration_ms", Value::Int(out.duration_ms as i64)),
        ("provider_id", Value::String(Rc::new(out.provider_id))),
    ]))
}

// --------- §53 skill use ---------

fn s25_skill_bind(args: &[Value]) -> Result<Value, String> {
    let manifest_json = s_arg(args, 0, "skill_bind")?;
    let caller_caps = list_of_strings(&args[1], "skill_bind", "caller_caps")?;
    let alias_v = s_arg(args, 2, "skill_bind")?;
    let alias = if alias_v.is_empty() { None } else { Some(alias_v.as_str()) };
    let manifest: axon_skill::Manifest = serde_json::from_str(manifest_json.as_str())
        .map_err(|e| format!("skill_bind: bad manifest json: {e}"))?;
    let binding = sk_use::bind_skill(&manifest, caller_caps, alias);
    let j = serde_json::to_value(&binding)
        .map_err(|e| format!("skill_bind: encode: {e}"))?;
    let mut record = json_to_value(&j);
    if let Value::Record(r) = &record {
        let mut fields = r.borrow().clone();
        if let Some(msg) = sk_use::explain_missing(&binding) {
            fields.push(("error".to_string(), Value::String(Rc::new(msg))));
        } else {
            fields.push(("error".to_string(), Value::String(Rc::new(String::new()))));
        }
        record = Value::Record(Rc::new(RefCell::new(fields)));
    }
    Ok(record)
}

fn s25_skill_narrow(args: &[Value]) -> Result<Value, String> {
    let callee = list_of_strings(&args[0], "skill_narrow_effects", "callee")?;
    let caller = list_of_strings(&args[1], "skill_narrow_effects", "caller")?;
    let callee_set: std::collections::BTreeSet<String> = callee.into_iter().collect();
    let caller_set: std::collections::BTreeSet<String> = caller.into_iter().collect();
    let narrowed = sk_use::narrow_effects(&callee_set, &caller_set);
    Ok(list_value(
        narrowed
            .into_iter()
            .map(|s| Value::String(Rc::new(s)))
            .collect(),
    ))
}

// --------- §54.1 agent-card auto publish ---------

fn s25_card_derive(args: &[Value]) -> Result<Value, String> {
    let summary_json = s_arg(args, 0, "agent_card_derive")?;
    let base_url = s_arg(args, 1, "agent_card_derive")?;
    let summary: a2a_auto::AgentSummary = serde_json::from_str(summary_json.as_str())
        .map_err(|e| format!("agent_card_derive: bad summary json: {e}"))?;
    let card = a2a_auto::derive_agent_card(&summary, base_url.as_str());
    card.verify().map_err(|e| format!("agent_card_derive: {e}"))?;
    let body = a2a_auto::render_well_known(&card)
        .map_err(|e| format!("agent_card_derive: {e}"))?;
    let json_str = String::from_utf8(body).map_err(|e| format!("agent_card_derive: {e}"))?;
    Ok(Value::String(Rc::new(json_str)))
}

fn s25_well_known_path(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::String(Rc::new(
        axon_a2a::WELL_KNOWN_PATH.to_string(),
    )))
}

// --------- §41 metrics + serverless ---------

fn s25_metrics_record(args: &[Value]) -> Result<Value, String> {
    let status = i_arg(args, 0, "metrics_record")?.max(0) as u16;
    let body_in = i_arg(args, 1, "metrics_record")?.max(0) as u64;
    let body_out = i_arg(args, 2, "metrics_record")?.max(0) as u64;
    let dur_us = i_arg(args, 3, "metrics_record")?.max(0) as u64;
    METRICS.with(|m| m.record_request(status, body_in, body_out, dur_us));
    Ok(Value::Unit)
}

fn s25_metrics_render(_args: &[Value]) -> Result<Value, String> {
    let body = METRICS.with(|m| m.render_prometheus());
    Ok(Value::String(Rc::new(body)))
}

fn s25_serverless_render(args: &[Value]) -> Result<Value, String> {
    let target_s = s_arg(args, 0, "serverless_render")?;
    let handler = s_arg(args, 1, "serverless_render")?;
    let skill = s_arg(args, 2, "serverless_render")?;
    let target = svless::ServerlessTarget::from_attribute(target_s.as_str())
        .ok_or_else(|| {
            format!(
                "serverless_render: unknown target `{}` — expected lambda|gcp_function|cf_worker",
                target_s.as_str()
            )
        })?;
    let tramp = svless::ServerlessTrampoline::new(target, handler.as_str().to_string());
    let body = match target {
        svless::ServerlessTarget::Lambda => svless::render_lambda_yaml(&tramp, skill.as_str()),
        svless::ServerlessTarget::GcpFunction => {
            svless::render_gcp_function_yaml(&tramp, skill.as_str())
        }
        svless::ServerlessTarget::CfWorker => {
            svless::render_cf_worker_toml(&tramp, skill.as_str())
        }
    };
    Ok(Value::String(Rc::new(body)))
}

// ===========================================================================
// Stage 26 — features (§7.1), MCP (§25.5), deterministic helpers (§39.2).
// ===========================================================================

use axon_project::mcp as proj_mcp;

thread_local! {
    static MCP_REGISTRY: RefCell<proj_mcp::McpRegistry> =
        RefCell::new(proj_mcp::McpRegistry::default());
}

fn install_stage26(interp: &Interpreter) {
    // ---- §39.2 deterministic helpers ----
    interp.register_native(
        "clock_freeze",
        n("clock_freeze", 1, Some(1), s26_clock_freeze),
    );
    interp.register_native(
        "clock_unfreeze",
        n("clock_unfreeze", 0, Some(0), s26_clock_unfreeze),
    );
    interp.register_native(
        "rand_seed",
        n("rand_seed", 1, Some(1), s26_rand_seed),
    );

    // ---- §25.5 MCP registry ----
    interp.register_native(
        "mcp_load_from_toml",
        n("mcp_load_from_toml", 1, Some(1), s26_mcp_load),
    );
    interp.register_native(
        "mcp_list_tools",
        n("mcp_list_tools", 1, Some(1), s26_mcp_list_tools),
    );
    interp.register_native(
        "mcp_call_tool",
        n("mcp_call_tool", 3, Some(3), s26_mcp_call_tool),
    );
    interp.register_native(
        "mcp_namespaces",
        n("mcp_namespaces", 0, Some(0), s26_mcp_namespaces),
    );
    interp.register_native(
        "mcp_deferred_namespaces",
        n(
            "mcp_deferred_namespaces",
            0,
            Some(0),
            s26_mcp_deferred,
        ),
    );

    // ---- §7.1 feature introspection ----
    interp.register_native(
        "features_active",
        n("features_active", 0, Some(0), s26_features_active),
    );
}

// --------- §39.2 deterministic helpers ---------

fn s26_clock_freeze(args: &[Value]) -> Result<Value, String> {
    let ns = i_arg(args, 0, "clock_freeze")?;
    axon_runtime::builtin::set_frozen_clock(Some(ns));
    Ok(Value::Unit)
}

fn s26_clock_unfreeze(_args: &[Value]) -> Result<Value, String> {
    axon_runtime::builtin::set_frozen_clock(None);
    Ok(Value::Unit)
}

fn s26_rand_seed(args: &[Value]) -> Result<Value, String> {
    let seed = i_arg(args, 0, "rand_seed")?;
    axon_runtime::builtin::set_rng_seed(seed as u64);
    Ok(Value::Unit)
}

// --------- §25.5 MCP ---------

fn s26_mcp_load(args: &[Value]) -> Result<Value, String> {
    let path = s_arg(args, 0, "mcp_load_from_toml")?;
    let text = std::fs::read_to_string(path.as_str())
        .map_err(|e| format!("mcp_load_from_toml: read `{}`: {e}", path.as_str()))?;
    let manifest = axon_project::Manifest::parse(&text)
        .map_err(|e| format!("mcp_load_from_toml: {e}"))?;
    let registry = proj_mcp::McpRegistry::from_hashmap(&manifest.tools);
    let n_loaded = registry.tools.len() as i64;
    MCP_REGISTRY.with(|c| *c.borrow_mut() = registry);
    Ok(Value::Int(n_loaded))
}

fn s26_mcp_list_tools(args: &[Value]) -> Result<Value, String> {
    let namespace = s_arg(args, 0, "mcp_list_tools")?;
    let tools = MCP_REGISTRY.with(|c| {
        c.borrow()
            .tools_in(namespace.as_str())
            .into_iter()
            .cloned()
            .collect::<Vec<_>>()
    });
    let items: Vec<Value> = tools
        .into_iter()
        .map(|t| {
            record_to_vec(vec![
                ("namespace", Value::String(Rc::new(t.namespace))),
                ("name", Value::String(Rc::new(t.name))),
                ("description", Value::String(Rc::new(t.description))),
                ("input_schema", Value::String(Rc::new(t.input_schema))),
                ("provider", Value::String(Rc::new(t.provider))),
            ])
        })
        .collect();
    Ok(list_value(items))
}

fn s26_mcp_call_tool(args: &[Value]) -> Result<Value, String> {
    let namespace = s_arg(args, 0, "mcp_call_tool")?;
    let name = s_arg(args, 1, "mcp_call_tool")?;
    let args_json = s_arg(args, 2, "mcp_call_tool")?;
    let result = MCP_REGISTRY.with(|c| {
        let reg = c.borrow();
        let client = proj_mcp::StaticMcpClient {
            registry: reg.clone(),
            namespace: namespace.as_str().to_string(),
        };
        proj_mcp::McpClient::call_tool(&client, name.as_str(), args_json.as_str())
    });
    match result {
        Ok(body) => Ok(record_to_vec(vec![
            ("ok", Value::Bool(true)),
            ("body", Value::String(Rc::new(body))),
            ("error", Value::String(Rc::new(String::new()))),
        ])),
        Err(e) => Ok(record_to_vec(vec![
            ("ok", Value::Bool(false)),
            ("body", Value::String(Rc::new(String::new()))),
            ("error", Value::String(Rc::new(e))),
        ])),
    }
}

fn s26_mcp_namespaces(_args: &[Value]) -> Result<Value, String> {
    let mut seen: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    MCP_REGISTRY.with(|c| {
        for t in &c.borrow().tools {
            seen.insert(t.namespace.clone());
        }
    });
    Ok(list_value(
        seen.into_iter().map(|s| Value::String(Rc::new(s))).collect(),
    ))
}

fn s26_mcp_deferred(_args: &[Value]) -> Result<Value, String> {
    let deferred = MCP_REGISTRY.with(|c| c.borrow().deferred_namespaces.clone());
    Ok(list_value(
        deferred
            .into_iter()
            .map(|s| Value::String(Rc::new(s)))
            .collect(),
    ))
}

// --------- §7.1 feature introspection ---------
//
// The active set is exposed via `features_active()` so programs can
// conditionally branch at runtime in addition to the static
// `#[cfg(feature = "X")]` gate. The host populates a thread-local from
// the CLI's `--features` flag before running.

thread_local! {
    static ACTIVE_FEATURES: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

pub fn set_active_features(features: Vec<String>) {
    ACTIVE_FEATURES.with(|c| *c.borrow_mut() = features);
}

fn s26_features_active(_args: &[Value]) -> Result<Value, String> {
    let active = ACTIVE_FEATURES.with(|c| c.borrow().clone());
    Ok(list_value(
        active.into_iter().map(|s| Value::String(Rc::new(s))).collect(),
    ))
}

// ===========================================================================
// Stage 27 — @approval tool attribute (§25.6), prompt @version (§24.3).
// ===========================================================================

use axon_guard::approval as appr;
use axon_runtime::prompt_version as pv;

thread_local! {
    static APPROVAL_REG: RefCell<appr::ApprovalRegistry> =
        RefCell::new(appr::ApprovalRegistry::new());
    static PROMPT_VERSIONS: RefCell<pv::PromptVersionRegistry> =
        RefCell::new(pv::PromptVersionRegistry::new());
}

fn install_stage27(interp: &Interpreter) {
    // ---- §25.6 approval ----
    interp.register_native(
        "approval_open",
        n("approval_open", 6, Some(6), s27_approval_open),
    );
    interp.register_native(
        "approval_approve",
        n("approval_approve", 2, Some(2), s27_approval_approve),
    );
    interp.register_native(
        "approval_deny",
        n("approval_deny", 3, Some(3), s27_approval_deny),
    );
    interp.register_native(
        "approval_get",
        n("approval_get", 1, Some(1), s27_approval_get),
    );
    interp.register_native(
        "approval_pending_count",
        n(
            "approval_pending_count",
            0,
            Some(0),
            s27_approval_pending_count,
        ),
    );
    interp.register_native(
        "approval_sweep_timeouts",
        n(
            "approval_sweep_timeouts",
            2,
            Some(2),
            s27_approval_sweep,
        ),
    );
    interp.register_native(
        "approval_next_id",
        n("approval_next_id", 0, Some(0), s27_approval_next_id),
    );
    interp.register_native(
        "approval_purge_terminal",
        n(
            "approval_purge_terminal",
            0,
            Some(0),
            s27_approval_purge,
        ),
    );

    // ---- §24.3 prompt @version ----
    interp.register_native(
        "prompt_version_register",
        n(
            "prompt_version_register",
            4,
            Some(4),
            s27_pv_register,
        ),
    );
    interp.register_native(
        "prompt_version_set_default",
        n(
            "prompt_version_set_default",
            2,
            Some(2),
            s27_pv_set_default,
        ),
    );
    interp.register_native(
        "prompt_version_pick",
        n(
            "prompt_version_pick",
            2,
            Some(2),
            s27_pv_pick,
        ),
    );
    interp.register_native(
        "prompt_version_versions_for",
        n(
            "prompt_version_versions_for",
            1,
            Some(1),
            s27_pv_versions_for,
        ),
    );
    interp.register_native(
        "prompt_version_prompts",
        n(
            "prompt_version_prompts",
            0,
            Some(0),
            s27_pv_prompts,
        ),
    );
}

// --------- §25.6 approval ---------

fn parse_on_timeout(s: &str) -> Result<appr::OnTimeout, String> {
    match s {
        "deny" => Ok(appr::OnTimeout::Deny),
        "allow" => Ok(appr::OnTimeout::Allow),
        "escalate" => Ok(appr::OnTimeout::Escalate),
        other => Err(format!(
            "approval_open: on_timeout must be deny|allow|escalate, got `{other}`"
        )),
    }
}

fn s27_approval_open(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "approval_open")?;
    let tool = s_arg(args, 1, "approval_open")?;
    let args_json = s_arg(args, 2, "approval_open")?;
    let by = s_arg(args, 3, "approval_open")?;
    let timeout_secs = i_arg(args, 4, "approval_open")?;
    let on_to = s_arg(args, 5, "approval_open")?;
    let on_to = parse_on_timeout(on_to.as_str())?;
    APPROVAL_REG.with(|c| {
        c.borrow_mut().open(
            id.as_str().to_string(),
            tool.as_str().to_string(),
            args_json.as_str().to_string(),
            by.as_str().to_string(),
            timeout_secs,
            on_to,
            now_ns(),
        )
    })
    .map_err(|e| e.to_string())?;
    Ok(Value::String(id))
}

fn s27_approval_approve(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "approval_approve")?;
    let actor = s_arg(args, 1, "approval_approve")?;
    APPROVAL_REG.with(|c| c.borrow_mut().approve(id.as_str(), actor.as_str()))
        .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn s27_approval_deny(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "approval_deny")?;
    let actor = s_arg(args, 1, "approval_deny")?;
    let reason = s_arg(args, 2, "approval_deny")?;
    APPROVAL_REG.with(|c| {
        c.borrow_mut()
            .deny(id.as_str(), actor.as_str(), reason.as_str())
    })
    .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn approval_state_name(state: appr::ApprovalState) -> &'static str {
    match state {
        appr::ApprovalState::Pending => "pending",
        appr::ApprovalState::Approved => "approved",
        appr::ApprovalState::Denied => "denied",
        appr::ApprovalState::TimedOut => "timed_out",
    }
}

fn approval_request_to_value(r: &appr::ApprovalRequest) -> Value {
    let on_to = match r.on_timeout {
        appr::OnTimeout::Deny => "deny",
        appr::OnTimeout::Allow => "allow",
        appr::OnTimeout::Escalate => "escalate",
    };
    record_to_vec(vec![
        ("id", Value::String(Rc::new(r.id.clone()))),
        ("tool", Value::String(Rc::new(r.tool.clone()))),
        ("args_json", Value::String(Rc::new(r.args_json.clone()))),
        ("by", Value::String(Rc::new(r.by.clone()))),
        ("timeout_secs", Value::Int(r.timeout_secs)),
        ("on_timeout", Value::String(Rc::new(on_to.to_string()))),
        (
            "state",
            Value::String(Rc::new(approval_state_name(r.state).to_string())),
        ),
        ("actor", Value::String(Rc::new(r.actor.clone()))),
        ("reason", Value::String(Rc::new(r.reason.clone()))),
        ("escalated_to", Value::String(Rc::new(r.escalated_to.clone()))),
        ("requested_at_ns", Value::Int(r.requested_at_ns)),
    ])
}

fn s27_approval_get(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "approval_get")?;
    APPROVAL_REG.with(|c| {
        c.borrow()
            .get(id.as_str())
            .map(approval_request_to_value)
            .ok_or_else(|| format!("approval_get: unknown approval `{}`", id.as_str()))
    })
}

fn s27_approval_pending_count(_args: &[Value]) -> Result<Value, String> {
    let n = APPROVAL_REG.with(|c| c.borrow().pending_count());
    Ok(Value::Int(n as i64))
}

fn s27_approval_sweep(args: &[Value]) -> Result<Value, String> {
    let now = i_arg(args, 0, "approval_sweep_timeouts")?;
    let escalate_to = s_arg(args, 1, "approval_sweep_timeouts")?;
    let target = escalate_to.as_str().to_string();
    let fired = APPROVAL_REG.with(|c| {
        c.borrow_mut()
            .sweep_timeouts(now, |_r| target.clone())
    });
    Ok(list_value(
        fired.into_iter().map(|s| Value::String(Rc::new(s))).collect(),
    ))
}

fn s27_approval_next_id(_args: &[Value]) -> Result<Value, String> {
    let id = APPROVAL_REG.with(|c| c.borrow_mut().next_id());
    Ok(Value::String(Rc::new(id)))
}

fn s27_approval_purge(_args: &[Value]) -> Result<Value, String> {
    let n = APPROVAL_REG.with(|c| c.borrow_mut().purge_terminal());
    Ok(Value::Int(n as i64))
}

// --------- §24.3 prompt @version ---------

fn s27_pv_register(args: &[Value]) -> Result<Value, String> {
    let prompt = s_arg(args, 0, "prompt_version_register")?;
    let version = s_arg(args, 1, "prompt_version_register")?;
    let body = s_arg(args, 2, "prompt_version_register")?;
    let notes = s_arg(args, 3, "prompt_version_register")?;
    PROMPT_VERSIONS.with(|c| {
        c.borrow_mut().register(
            prompt.as_str(),
            version.as_str(),
            body.as_str(),
            notes.as_str(),
            now_ns(),
        )
    })
    .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn s27_pv_set_default(args: &[Value]) -> Result<Value, String> {
    let prompt = s_arg(args, 0, "prompt_version_set_default")?;
    let version = s_arg(args, 1, "prompt_version_set_default")?;
    PROMPT_VERSIONS.with(|c| {
        c.borrow_mut().set_default(prompt.as_str(), version.as_str())
    })
    .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn prompt_version_to_value(v: &pv::PromptVersion) -> Value {
    record_to_vec(vec![
        ("prompt_name", Value::String(Rc::new(v.prompt_name.clone()))),
        ("version", Value::String(Rc::new(v.version.clone()))),
        ("body", Value::String(Rc::new(v.body.clone()))),
        ("notes", Value::String(Rc::new(v.notes.clone()))),
        ("registered_at_ns", Value::Int(v.registered_at_ns)),
    ])
}

fn s27_pv_pick(args: &[Value]) -> Result<Value, String> {
    let prompt = s_arg(args, 0, "prompt_version_pick")?;
    let version_arg = s_arg(args, 1, "prompt_version_pick")?;
    let version_opt = if version_arg.is_empty() {
        None
    } else {
        Some(version_arg.as_str())
    };
    PROMPT_VERSIONS.with(|c| {
        c.borrow()
            .pick(prompt.as_str(), version_opt)
            .map(prompt_version_to_value)
            .map_err(|e| e.to_string())
    })
}

fn s27_pv_versions_for(args: &[Value]) -> Result<Value, String> {
    let prompt = s_arg(args, 0, "prompt_version_versions_for")?;
    let items: Vec<Value> = PROMPT_VERSIONS.with(|c| {
        c.borrow()
            .versions_for(prompt.as_str())
            .into_iter()
            .map(prompt_version_to_value)
            .collect()
    });
    Ok(list_value(items))
}

fn s27_pv_prompts(_args: &[Value]) -> Result<Value, String> {
    let names = PROMPT_VERSIONS.with(|c| c.borrow().prompts());
    Ok(list_value(
        names.into_iter().map(|s| Value::String(Rc::new(s))).collect(),
    ))
}

// ===========================================================================
// Stage 28 — consensus (§29.5), human pseudo-agent (§29.9), policy block
// (§30), FFI bridges (§35.2), protocol adapters (§35.3).
// ===========================================================================

use axon_deploy::protocols as proto;
use axon_ffi::bridges as ffi_bridges;
use axon_flow::consensus as cons;
use axon_guard::human as gh_human;
use axon_guard::policy_block as pb;

thread_local! {
    static POLICY_BLOCKS: RefCell<std::collections::HashMap<String, pb::PolicyBlock>> =
        RefCell::new(std::collections::HashMap::new());
}

fn install_stage28(interp: &Interpreter) {
    // ---- §29.5 consensus ----
    interp.register_native(
        "flow_consensus",
        n("flow_consensus", 2, Some(2), s28_flow_consensus),
    );
    interp.register_native_ext(
        "flow_spawn_pool",
        ext("flow_spawn_pool", 2, Some(2), s28_spawn_pool),
    );

    // ---- §29.9 human pseudo-agent ----
    interp.register_native(
        "human_request",
        n("human_request", 4, Some(4), s28_human_request),
    );
    interp.register_native(
        "human_resolve",
        n("human_resolve", 1, Some(1), s28_human_resolve),
    );
    interp.register_native(
        "human_cancel",
        n("human_cancel", 1, Some(1), s28_human_cancel),
    );

    // ---- §30 policy block ----
    interp.register_native(
        "policy_block_new",
        n("policy_block_new", 2, Some(2), s28_pb_new),
    );
    interp.register_native(
        "policy_block_allow",
        n("policy_block_allow", 4, Some(4), s28_pb_allow),
    );
    interp.register_native(
        "policy_block_deny",
        n("policy_block_deny", 3, Some(3), s28_pb_deny),
    );
    interp.register_native(
        "policy_block_check",
        n("policy_block_check", 4, Some(4), s28_pb_check),
    );
    interp.register_native(
        "policy_block_charge",
        n("policy_block_charge", 4, Some(4), s28_pb_charge),
    );
    interp.register_native(
        "policy_block_add_budget",
        n("policy_block_add_budget", 4, Some(4), s28_pb_add_budget),
    );
    interp.register_native(
        "policy_block_add_rate",
        n("policy_block_add_rate", 4, Some(4), s28_pb_add_rate),
    );
    interp.register_native(
        "policy_block_audit_summary",
        n("policy_block_audit_summary", 1, Some(1), s28_pb_audit_summary),
    );

    // ---- §35.2 FFI bridges ----
    interp.register_native(
        "ffi_bridge_call",
        n("ffi_bridge_call", 5, Some(5), s28_bridge_call),
    );

    // ---- §35.3 protocol adapters ----
    interp.register_native(
        "serve_protocol_route",
        n("serve_protocol_route", 5, Some(5), s28_protocol_route),
    );
    interp.register_native(
        "serve_protocol_wrap",
        n("serve_protocol_wrap", 3, Some(3), s28_protocol_wrap),
    );
    interp.register_native(
        "serve_render_grpc_proto",
        n(
            "serve_render_grpc_proto",
            2,
            Some(2),
            s28_render_grpc_proto,
        ),
    );
}

// --------- §29.5 consensus ---------

fn s28_flow_consensus(args: &[Value]) -> Result<Value, String> {
    // Args: (votes_list, config_record)
    let votes_v = match &args[0] {
        Value::List(l) => l.borrow().clone(),
        other => {
            return Err(format!(
                "flow_consensus: votes must be a List, got `{}`",
                other.type_name()
            ));
        }
    };
    let mut votes: Vec<cons::Vote> = Vec::with_capacity(votes_v.len());
    for (i, v) in votes_v.into_iter().enumerate() {
        let j = value_to_json(&v);
        let parsed: cons::Vote = serde_json::from_value(j)
            .map_err(|e| format!("flow_consensus: votes[{i}]: {e}"))?;
        votes.push(parsed);
    }
    let cfg_v = value_to_json(&args[1]);
    let cfg: cons::ConsensusConfig =
        serde_json::from_value(cfg_v).map_err(|e| format!("flow_consensus: config: {e}"))?;
    let d = cons::consensus(&votes, &cfg);
    let j = serde_json::to_value(&d).map_err(|e| format!("flow_consensus: {e}"))?;
    Ok(json_to_value(&j))
}

/// `flow_spawn_pool(constructor, size)` calls `constructor()` N times
/// and returns a List of the results. The synchronous interpreter
/// today runs them in sequence; a future async scheduler can
/// parallelize without changing call sites.
fn s28_spawn_pool(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let constructor = args[0].clone();
    if !is_callable(&constructor) {
        return Err("flow_spawn_pool: constructor must be callable".into());
    }
    let size = i_arg(args, 1, "flow_spawn_pool")?.max(0) as usize;
    if size == 0 {
        return Err("flow_spawn_pool: size must be > 0".into());
    }
    if size > 1024 {
        return Err(format!(
            "flow_spawn_pool: size {size} > 1024 — refusing to spawn that many"
        ));
    }
    let mut out: Vec<Value> = Vec::with_capacity(size);
    for i in 0..size {
        let v = interp
            .call_value(&constructor, &[Value::Int(i as i64)], span)
            .map_err(|e| format!("flow_spawn_pool[{i}]: {}", eval_signal_msg(&e)))?;
        out.push(v);
    }
    Ok(list_value(out))
}

// --------- §29.9 human pseudo-agent ---------

fn parse_on_timeout_28(s: &str) -> Result<axon_guard::approval::OnTimeout, String> {
    match s {
        "deny" => Ok(axon_guard::approval::OnTimeout::Deny),
        "allow" => Ok(axon_guard::approval::OnTimeout::Allow),
        "escalate" => Ok(axon_guard::approval::OnTimeout::Escalate),
        other => Err(format!(
            "human_request: on_timeout must be deny|allow|escalate, got `{other}`"
        )),
    }
}

fn s28_human_request(args: &[Value]) -> Result<Value, String> {
    let channel = s_arg(args, 0, "human_request")?;
    let prompt = s_arg(args, 1, "human_request")?;
    let timeout = i_arg(args, 2, "human_request")?;
    let on_to = s_arg(args, 3, "human_request")?;
    let on_to = parse_on_timeout_28(on_to.as_str())?;
    let id = APPROVAL_REG.with(|c| {
        gh_human::open_review(
            &mut c.borrow_mut(),
            channel.as_str(),
            prompt.as_str(),
            timeout,
            on_to,
            now_ns(),
        )
    })
    .map_err(|e| e.to_string())?;
    Ok(Value::String(Rc::new(id)))
}

fn s28_human_resolve(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "human_resolve")?;
    let r = APPROVAL_REG.with(|c| {
        gh_human::resolve(&mut c.borrow_mut(), id.as_str(), now_ns())
    });
    match r {
        Some(r) => Ok(approval_request_to_value(&r)),
        None => Err(format!("human_resolve: unknown id `{}`", id.as_str())),
    }
}

fn s28_human_cancel(args: &[Value]) -> Result<Value, String> {
    let id = s_arg(args, 0, "human_cancel")?;
    let ok = APPROVAL_REG.with(|c| gh_human::cancel(&mut c.borrow_mut(), id.as_str()));
    Ok(Value::Bool(ok))
}

// --------- §30 policy block ---------

fn parse_effect_kind(s: &str) -> Result<pb::EffectKind, String> {
    pb::EffectKind::from_str(s)
        .ok_or_else(|| format!("policy_block: unknown effect kind `{s}` (expected tool|net|fs|llm|memory)"))
}

fn s28_pb_new(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "policy_block_new")?;
    let default = s_arg(args, 1, "policy_block_new")?;
    let mut block = pb::PolicyBlock::new(name.as_str());
    block.default_action = match default.as_str() {
        "allow" => pb::ActionKind::Allow,
        "deny" => pb::ActionKind::Deny,
        other => {
            return Err(format!(
                "policy_block_new: default must be allow|deny, got `{other}`"
            ));
        }
    };
    POLICY_BLOCKS.with(|c| c.borrow_mut().insert(name.as_str().to_string(), block));
    Ok(Value::String(name))
}

fn s28_pb_allow(args: &[Value]) -> Result<Value, String> {
    let block_name = s_arg(args, 0, "policy_block_allow")?;
    let kind = s_arg(args, 1, "policy_block_allow")?;
    let pattern = s_arg(args, 2, "policy_block_allow")?;
    let when = s_arg(args, 3, "policy_block_allow")?;
    let kind = parse_effect_kind(kind.as_str())?;
    POLICY_BLOCKS.with(|c| {
        let mut map = c.borrow_mut();
        let b = map
            .get_mut(block_name.as_str())
            .ok_or_else(|| format!("policy_block_allow: no policy `{}`", block_name.as_str()))?;
        b.allow(kind, pattern.as_str(), when.as_str());
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s28_pb_deny(args: &[Value]) -> Result<Value, String> {
    let block_name = s_arg(args, 0, "policy_block_deny")?;
    let kind = s_arg(args, 1, "policy_block_deny")?;
    let pattern = s_arg(args, 2, "policy_block_deny")?;
    let kind = parse_effect_kind(kind.as_str())?;
    POLICY_BLOCKS.with(|c| {
        let mut map = c.borrow_mut();
        let b = map
            .get_mut(block_name.as_str())
            .ok_or_else(|| format!("policy_block_deny: no policy `{}`", block_name.as_str()))?;
        b.deny(kind, pattern.as_str());
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s28_pb_check(args: &[Value]) -> Result<Value, String> {
    let block_name = s_arg(args, 0, "policy_block_check")?;
    let kind = s_arg(args, 1, "policy_block_check")?;
    let target = s_arg(args, 2, "policy_block_check")?;
    let when_holds = b_arg(args, 3, "policy_block_check")?;
    let kind = parse_effect_kind(kind.as_str())?;
    let decision = POLICY_BLOCKS.with(|c| {
        let mut map = c.borrow_mut();
        let b = map
            .get_mut(block_name.as_str())
            .ok_or_else(|| format!("policy_block_check: no policy `{}`", block_name.as_str()))?;
        Ok::<_, String>(b.check_effect(kind, target.as_str(), when_holds, now_ns()))
    })?;
    Ok(record_to_vec(vec![
        ("allow", Value::Bool(decision.allow)),
        (
            "rule_index",
            match decision.rule_index {
                Some(i) => Value::Int(i as i64),
                None => Value::Int(-1),
            },
        ),
        ("label", Value::String(Rc::new(decision.label))),
        (
            "budget_remaining_usd",
            match decision.budget_remaining_usd {
                Some(f) => Value::Float(f),
                None => Value::Float(-1.0),
            },
        ),
        (
            "budget_remaining_tokens",
            match decision.budget_remaining_tokens {
                Some(t) => Value::Int(t as i64),
                None => Value::Int(-1),
            },
        ),
    ]))
}

fn s28_pb_charge(args: &[Value]) -> Result<Value, String> {
    let block_name = s_arg(args, 0, "policy_block_charge")?;
    let scope = s_arg(args, 1, "policy_block_charge")?;
    let usd_cents = i_arg(args, 2, "policy_block_charge")?;
    let tokens = i_arg(args, 3, "policy_block_charge")?.max(0) as u64;
    let usd = usd_cents as f64 / 100.0;
    POLICY_BLOCKS.with(|c| {
        let mut map = c.borrow_mut();
        let b = map
            .get_mut(block_name.as_str())
            .ok_or_else(|| format!("policy_block_charge: no policy `{}`", block_name.as_str()))?;
        b.charge(scope.as_str(), usd, tokens);
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s28_pb_add_budget(args: &[Value]) -> Result<Value, String> {
    let block_name = s_arg(args, 0, "policy_block_add_budget")?;
    let scope = s_arg(args, 1, "policy_block_add_budget")?;
    let max_usd_cents = i_arg(args, 2, "policy_block_add_budget")?;
    let max_tokens = i_arg(args, 3, "policy_block_add_budget")?;
    POLICY_BLOCKS.with(|c| {
        let mut map = c.borrow_mut();
        let b = map
            .get_mut(block_name.as_str())
            .ok_or_else(|| {
                format!(
                    "policy_block_add_budget: no policy `{}`",
                    block_name.as_str()
                )
            })?;
        b.add_budget(pb::BudgetClause {
            scope: scope.as_str().to_string(),
            max_usd: if max_usd_cents < 0 {
                None
            } else {
                Some(max_usd_cents as f64 / 100.0)
            },
            max_tokens: if max_tokens < 0 {
                None
            } else {
                Some(max_tokens as u64)
            },
            max_wall_secs: None,
            window_secs: None,
            spent_usd: 0.0,
            spent_tokens: 0,
        });
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s28_pb_add_rate(args: &[Value]) -> Result<Value, String> {
    let block_name = s_arg(args, 0, "policy_block_add_rate")?;
    let scope = s_arg(args, 1, "policy_block_add_rate")?;
    let max_calls = i_arg(args, 2, "policy_block_add_rate")?.max(0) as u32;
    let window_secs = i_arg(args, 3, "policy_block_add_rate")?.max(0) as u64;
    POLICY_BLOCKS.with(|c| {
        let mut map = c.borrow_mut();
        let b = map
            .get_mut(block_name.as_str())
            .ok_or_else(|| {
                format!(
                    "policy_block_add_rate: no policy `{}`",
                    block_name.as_str()
                )
            })?;
        b.add_rate(pb::RateClause {
            scope: scope.as_str().to_string(),
            max_calls,
            window_secs,
            recent_call_ns: Vec::new(),
        });
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s28_pb_audit_summary(args: &[Value]) -> Result<Value, String> {
    let block_name = s_arg(args, 0, "policy_block_audit_summary")?;
    let (allow, deny) = POLICY_BLOCKS.with(|c| {
        c.borrow()
            .get(block_name.as_str())
            .map(|b| b.audit_summary())
            .unwrap_or((0, 0))
    });
    Ok(record_to_vec(vec![
        ("allow", Value::Int(allow as i64)),
        ("deny", Value::Int(deny as i64)),
    ]))
}

// --------- §35.2 FFI bridges ---------

fn s28_bridge_call(args: &[Value]) -> Result<Value, String> {
    let kind_s = s_arg(args, 0, "ffi_bridge_call")?;
    let target = s_arg(args, 1, "ffi_bridge_call")?;
    let entrypoint = s_arg(args, 2, "ffi_bridge_call")?;
    let args_json = s_arg(args, 3, "ffi_bridge_call")?;
    let timeout_ms = i_arg(args, 4, "ffi_bridge_call")?.max(0) as u64;
    let kind = ffi_bridges::BridgeKind::from_str(kind_s.as_str()).ok_or_else(|| {
        format!(
            "ffi_bridge_call: kind must be python|node|wasm|grpc, got `{}`",
            kind_s.as_str()
        )
    })?;
    let spec = ffi_bridges::BridgeSpec {
        kind,
        target: target.as_str().to_string(),
        entrypoint: entrypoint.as_str().to_string(),
        timeout_ms,
        launcher_override: String::new(),
    };
    match ffi_bridges::call_bridge(&spec, args_json.as_str()) {
        Ok(out) => Ok(record_to_vec(vec![
            ("ok", Value::Bool(true)),
            (
                "value_json",
                Value::String(Rc::new(out.value.to_string())),
            ),
            ("error", Value::String(Rc::new(String::new()))),
        ])),
        Err(e) => Ok(record_to_vec(vec![
            ("ok", Value::Bool(false)),
            ("value_json", Value::String(Rc::new(String::new()))),
            ("error", Value::String(Rc::new(e.to_string()))),
        ])),
    }
}

// --------- §35.3 protocol adapters ---------

fn parse_protocol(s: &str) -> Result<proto::ServeProtocol, String> {
    proto::ServeProtocol::from_flag(s).ok_or_else(|| {
        format!("serve_protocol_route: unknown protocol `{s}` (expected plain|mcp|openai|grpc|a2a)")
    })
}

fn s28_protocol_route(args: &[Value]) -> Result<Value, String> {
    let proto_s = s_arg(args, 0, "serve_protocol_route")?;
    let method = s_arg(args, 1, "serve_protocol_route")?;
    let path = s_arg(args, 2, "serve_protocol_route")?;
    let body = s_arg(args, 3, "serve_protocol_route")?;
    let default_handler = s_arg(args, 4, "serve_protocol_route")?;
    let p = parse_protocol(proto_s.as_str())?;
    let action = proto::route(
        p,
        &proto::IncomingRequest {
            method: method.as_str(),
            path: path.as_str(),
            body: body.as_str(),
        },
        default_handler.as_str(),
        "",
    );
    Ok(match action {
        proto::ProtocolAction::Reply { status, body, content_type } => record_to_vec(vec![
            ("kind", Value::String(Rc::new("reply".to_string()))),
            ("status", Value::Int(status as i64)),
            ("body", Value::String(Rc::new(body))),
            ("content_type", Value::String(Rc::new(content_type))),
            ("handler", Value::String(Rc::new(String::new()))),
            ("prompt", Value::String(Rc::new(String::new()))),
        ]),
        proto::ProtocolAction::Dispatch { handler, prompt, jsonrpc_id } => record_to_vec(vec![
            ("kind", Value::String(Rc::new("dispatch".to_string()))),
            ("status", Value::Int(0)),
            ("body", Value::String(Rc::new(String::new()))),
            ("content_type", Value::String(Rc::new(String::new()))),
            ("handler", Value::String(Rc::new(handler))),
            ("prompt", Value::String(Rc::new(prompt))),
            (
                "jsonrpc_id",
                Value::String(Rc::new(jsonrpc_id.to_string())),
            ),
        ]),
    })
}

fn s28_protocol_wrap(args: &[Value]) -> Result<Value, String> {
    let proto_s = s_arg(args, 0, "serve_protocol_wrap")?;
    let reply = s_arg(args, 1, "serve_protocol_wrap")?;
    let id_json = s_arg(args, 2, "serve_protocol_wrap")?;
    let p = parse_protocol(proto_s.as_str())?;
    let id_v: serde_json::Value =
        serde_json::from_str(id_json.as_str()).unwrap_or(serde_json::Value::Null);
    let (status, body, content_type) = proto::wrap_response(p, reply.as_str(), &id_v);
    Ok(record_to_vec(vec![
        ("status", Value::Int(status as i64)),
        ("body", Value::String(Rc::new(body))),
        (
            "content_type",
            Value::String(Rc::new(content_type.to_string())),
        ),
    ]))
}

fn s28_render_grpc_proto(args: &[Value]) -> Result<Value, String> {
    let service_name = s_arg(args, 0, "serve_render_grpc_proto")?;
    let handlers = list_of_strings(&args[1], "serve_render_grpc_proto", "handlers")?;
    let body = proto::render_grpc_proto(service_name.as_str(), &handlers);
    Ok(Value::String(Rc::new(body)))
}

/// Compile every `policy NAME { ... }` declaration (§30) in a loaded
/// program into an `axon-guard` `PolicyBlock` and register it in the
/// `POLICY_BLOCKS` thread-local. After this runs, `policy_block_check`
/// / `policy_block_charge` / `policy_block_audit_summary` work against
/// natively-declared policies, and `agent X { policy: NAME }` handlers
/// can consult them.
pub fn register_policies(program: &axon_ast::Program) {
    use axon_ast::{Item, PolicyAction, PolicyClause};
    for item in &program.items {
        let Item::Policy(decl) = item else { continue };
        let mut block = pb::PolicyBlock::new(decl.name.name.clone());
        // Policies are default-deny per §30 ("deny by default").
        block.default_action = pb::ActionKind::Deny;
        for clause in &decl.clauses {
            match clause {
                PolicyClause::Rule {
                    action,
                    effect,
                    patterns,
                    when,
                } => {
                    let Some(kind) = effect_str_to_kind(effect) else {
                        continue;
                    };
                    for pat in patterns {
                        match action {
                            PolicyAction::Allow => block.allow(
                                kind,
                                pat.clone(),
                                when.clone().unwrap_or_default(),
                            ),
                            PolicyAction::Deny => block.deny(kind, pat.clone()),
                        }
                    }
                }
                PolicyClause::Budget {
                    scope,
                    usd_cents,
                    tokens,
                } => {
                    block.add_budget(pb::BudgetClause {
                        scope: scope.clone(),
                        max_usd: usd_cents.map(|c| c as f64 / 100.0),
                        max_tokens: tokens.map(|t| t.max(0) as u64),
                        max_wall_secs: None,
                        window_secs: None,
                        spent_usd: 0.0,
                        spent_tokens: 0,
                    });
                }
                PolicyClause::Rate {
                    scope,
                    max_calls,
                    window_secs,
                } => {
                    block.add_rate(pb::RateClause {
                        scope: scope.clone(),
                        max_calls: *max_calls,
                        window_secs: *window_secs,
                        recent_call_ns: Vec::new(),
                    });
                }
                PolicyClause::Audit(kinds) => {
                    for k in kinds {
                        block.audit(k.clone());
                    }
                }
            }
        }
        POLICY_BLOCKS.with(|c| {
            c.borrow_mut().insert(decl.name.name.clone(), block);
        });
    }
}

fn effect_str_to_kind(s: &str) -> Option<pb::EffectKind> {
    match s {
        "tool" => Some(pb::EffectKind::Tool),
        "net" => Some(pb::EffectKind::Net),
        // `io` and `fs` both map to the filesystem effect kind.
        "fs" | "io" => Some(pb::EffectKind::Fs),
        "llm" => Some(pb::EffectKind::Llm),
        "memory" => Some(pb::EffectKind::Memory),
        _ => None,
    }
}

/// Wire `extern <lang> "path:fn"` tool bodies (§35.2) to the FFI bridge
/// layer. The runtime calls this dispatcher with `(abi, symbol,
/// args_json)`; we map the abi to a `BridgeKind`, split the symbol into
/// `target:entrypoint`, run the subprocess bridge, and hand back the
/// `value` JSON the bridge produced.
fn install_extern_bridge(interp: &Interpreter) {
    interp.set_bridge_dispatcher(Rc::new(
        |abi: &str, symbol: &str, args_json: &str| -> Result<String, String> {
            let kind = ffi_bridges::BridgeKind::from_str(abi).ok_or_else(|| {
                format!("extern tool: unknown bridge `{abi}` (expected python|node|wasm|grpc)")
            })?;
            // `symbol` is `path:entrypoint` for python/node/wasm, or a
            // bare `pkg.Svc/Method` for grpc. Split on the last colon so
            // Windows-style paths with a drive letter still work.
            let (target, entrypoint) = match symbol.rsplit_once(':') {
                Some((t, e)) if !matches!(kind, ffi_bridges::BridgeKind::Grpc) => {
                    (t.to_string(), e.to_string())
                }
                _ => (symbol.to_string(), String::new()),
            };
            let spec = ffi_bridges::BridgeSpec {
                kind,
                target,
                entrypoint,
                timeout_ms: 30_000,
                launcher_override: String::new(),
            };
            let out =
                ffi_bridges::call_bridge(&spec, args_json).map_err(|e| e.to_string())?;
            Ok(out.value.to_string())
        },
    ));
}

// ===========================================================================
// Stage 29 — Result/try_recover (§19), Stream<T> (§28), @restart variants
// (§29.7), axon prof --cost (§31.2).
// ===========================================================================

use axon_runtime::restart_policy as rp;
use axon_runtime::stream as rt_stream;

thread_local! {
    static STREAMS: RefCell<std::collections::HashMap<String, rt_stream::StreamHandle>> =
        RefCell::new(std::collections::HashMap::new());
}

fn install_stage29(interp: &Interpreter) {
    // ---- §19 try_recover ----
    interp.register_native_ext(
        "try_recover",
        ext("try_recover", 2, Some(2), s29_try_recover),
    );
    // ---- §28 streams ----
    interp.register_native(
        "stream_new",
        n("stream_new", 3, Some(3), s29_stream_new),
    );
    interp.register_native(
        "stream_send",
        n("stream_send", 2, Some(2), s29_stream_send),
    );
    interp.register_native(
        "stream_take",
        n("stream_take", 1, Some(1), s29_stream_take),
    );
    interp.register_native(
        "stream_close",
        n("stream_close", 1, Some(1), s29_stream_close),
    );
    interp.register_native(
        "stream_is_done",
        n("stream_is_done", 1, Some(1), s29_stream_is_done),
    );
    interp.register_native(
        "stream_stats",
        n("stream_stats", 1, Some(1), s29_stream_stats),
    );
    interp.register_native_ext(
        "for_await",
        ext("for_await", 2, Some(2), s29_for_await),
    );

    // ---- §29.7 @restart validator ----
    interp.register_native(
        "restart_policy_parse",
        n(
            "restart_policy_parse",
            1,
            Some(1),
            s29_restart_policy_parse,
        ),
    );
    interp.register_native(
        "restart_policy_should_restart",
        n(
            "restart_policy_should_restart",
            2,
            Some(2),
            s29_restart_policy_should_restart,
        ),
    );
}

// --------- §19 try_recover ---------

/// `try_recover(action, on_err)` — calls `action()`; if it errors,
/// passes the message string to `on_err` and returns whatever the
/// recovery callable produces. Mirrors `try { ... } recover |e| { ... }`
/// without needing a parser change.
fn s29_try_recover(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let action = args[0].clone();
    let on_err = args[1].clone();
    for (name, v) in [("action", &action), ("on_err", &on_err)] {
        if !is_callable(v) {
            return Err(format!("try_recover: `{name}` must be callable"));
        }
    }
    match interp.call_value(&action, &[], span) {
        Ok(v) => Ok(v),
        Err(sig) => {
            let msg = eval_signal_msg(&sig);
            interp
                .call_value(&on_err, &[Value::String(Rc::new(msg))], span)
                .map_err(|e| format!("try_recover[on_err]: {}", eval_signal_msg(&e)))
        }
    }
}

// --------- §28 streams ---------

fn parse_backpressure(s: &str) -> Result<rt_stream::BackpressurePolicy, String> {
    match s {
        "block" | "" => Ok(rt_stream::BackpressurePolicy::Block),
        "drop_oldest" => Ok(rt_stream::BackpressurePolicy::DropOldest),
        "drop_new" => Ok(rt_stream::BackpressurePolicy::DropNew),
        other => Err(format!(
            "stream_new: policy must be block|drop_oldest|drop_new, got `{other}`"
        )),
    }
}

fn s29_stream_new(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "stream_new")?;
    let capacity = i_arg(args, 1, "stream_new")?.max(1) as usize;
    let policy = s_arg(args, 2, "stream_new")?;
    let policy = parse_backpressure(policy.as_str())?;
    STREAMS.with(|c| {
        c.borrow_mut().insert(
            name.as_str().to_string(),
            rt_stream::StreamHandle::new(name.as_str(), capacity, policy),
        )
    });
    Ok(Value::String(name))
}

fn s29_stream_send(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "stream_send")?;
    let value_json = value_to_json(&args[1]);
    let outcome = STREAMS.with(|c| {
        let mut map = c.borrow_mut();
        let h = map
            .get_mut(name.as_str())
            .ok_or_else(|| format!("stream_send: no stream `{}`", name.as_str()))?;
        Ok::<_, String>(h.send(value_json))
    })?;
    let s = match outcome {
        rt_stream::SendOutcome::Buffered => "buffered",
        rt_stream::SendOutcome::Closed => "closed",
        rt_stream::SendOutcome::Backpressure => "backpressure",
        rt_stream::SendOutcome::DroppedOldest => "dropped_oldest",
        rt_stream::SendOutcome::DroppedNew => "dropped_new",
    };
    Ok(Value::String(Rc::new(s.to_string())))
}

fn s29_stream_take(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "stream_take")?;
    let v = STREAMS.with(|c| {
        let mut map = c.borrow_mut();
        let h = map
            .get_mut(name.as_str())
            .ok_or_else(|| format!("stream_take: no stream `{}`", name.as_str()))?;
        Ok::<_, String>(h.take())
    })?;
    match v {
        Some(j) => Ok(record_to_vec(vec![
            ("has_value", Value::Bool(true)),
            ("value", json_to_value(&j)),
        ])),
        None => Ok(record_to_vec(vec![
            ("has_value", Value::Bool(false)),
            ("value", Value::Nil),
        ])),
    }
}

fn s29_stream_close(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "stream_close")?;
    STREAMS.with(|c| {
        let mut map = c.borrow_mut();
        let h = map
            .get_mut(name.as_str())
            .ok_or_else(|| format!("stream_close: no stream `{}`", name.as_str()))?;
        h.close();
        Ok::<_, String>(())
    })?;
    Ok(Value::Unit)
}

fn s29_stream_is_done(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "stream_is_done")?;
    let done = STREAMS.with(|c| {
        c.borrow()
            .get(name.as_str())
            .map(|h| h.is_done())
            .unwrap_or(true)
    });
    Ok(Value::Bool(done))
}

fn s29_stream_stats(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "stream_stats")?;
    let (sent, taken, dropped, len, closed) = STREAMS.with(|c| {
        let map = c.borrow();
        let h = map
            .get(name.as_str())
            .ok_or_else(|| format!("stream_stats: no stream `{}`", name.as_str()))?;
        Ok::<_, String>((h.sent, h.taken, h.dropped, h.len(), h.closed))
    })?;
    Ok(record_to_vec(vec![
        ("sent", Value::Int(sent as i64)),
        ("taken", Value::Int(taken as i64)),
        ("dropped", Value::Int(dropped as i64)),
        ("buffer_len", Value::Int(len as i64)),
        ("closed", Value::Bool(closed)),
    ]))
}

/// `for_await(stream_name, body)` — pumps the stream into `body`
/// until the stream is `is_done`. Drops out on the first error
/// returned by the body; otherwise loops to exhaustion.
fn s29_for_await(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let name = s_arg(args, 0, "for_await")?;
    let body = args[1].clone();
    if !is_callable(&body) {
        return Err("for_await: `body` must be callable".into());
    }
    let mut count = 0i64;
    loop {
        let next = STREAMS.with(|c| {
            let mut map = c.borrow_mut();
            let h = map
                .get_mut(name.as_str())
                .ok_or_else(|| format!("for_await: no stream `{}`", name.as_str()))?;
            if h.is_done() {
                return Ok::<_, String>(None);
            }
            Ok(h.take())
        })?;
        let Some(j) = next else {
            // Stream is either drained-and-closed, or empty-and-open.
            let done = STREAMS.with(|c| {
                c.borrow()
                    .get(name.as_str())
                    .map(|h| h.is_done())
                    .unwrap_or(true)
            });
            if done {
                break;
            }
            // Empty but open — return so the caller can wake later.
            break;
        };
        let v = json_to_value(&j);
        interp
            .call_value(&body, &[v], span)
            .map_err(|e| format!("for_await[{count}]: {}", eval_signal_msg(&e)))?;
        count += 1;
    }
    Ok(Value::Int(count))
}

// --------- §29.7 @restart variants ---------

fn s29_restart_policy_parse(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "restart_policy_parse")?;
    let p = rp::RestartPolicy::from_attribute_name(name.as_str())
        .map_err(|e| e.to_string())?;
    Ok(Value::String(Rc::new(p.name().to_string())))
}

fn s29_restart_policy_should_restart(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "restart_policy_should_restart")?;
    let exit = s_arg(args, 1, "restart_policy_should_restart")?;
    let p = rp::RestartPolicy::from_attribute_name(name.as_str())
        .map_err(|e| e.to_string())?;
    let exit_kind = match exit.as_str() {
        "normal" => rp::ExitKind::Normal,
        "abnormal" => rp::ExitKind::Abnormal,
        other => {
            return Err(format!(
                "restart_policy_should_restart: exit must be normal|abnormal, got `{other}`"
            ));
        }
    };
    Ok(Value::Bool(p.should_restart(exit_kind)))
}

// ===========================================================================
// Stage 31 — computer-use primitives (§new) + GBNF schema emitter
// (§56.3 / native constrained decoding).
// ===========================================================================

use axon_computer as cu;

thread_local! {
    /// Process-wide computer-use driver. Tests + the CLI install a
    /// `MockDriver` by default; real browser/desktop drivers plug in
    /// via `set_computer_driver`.
    static COMPUTER_DRIVER: RefCell<Box<dyn cu::ComputerDriver + Send>> =
        RefCell::new(Box::new(cu::MockDriver::new(1280, 720)));
}

/// Swap the active computer-use driver. Called by tests + by future
/// host crates that ship Playwright / CDP / desktop drivers.
pub fn set_computer_driver(d: Box<dyn cu::ComputerDriver + Send>) {
    COMPUTER_DRIVER.with(|c| *c.borrow_mut() = d);
}

fn install_stage31(interp: &Interpreter) {
    // §35 — computer-use primitives. All gated by the `Computer`
    // capability; the runtime grants it via the default cap set so
    // `axon run` without flags can drive the mock driver.
    interp.register_native(
        "computer_screenshot",
        n("computer_screenshot", 0, Some(0), s31_screenshot),
    );
    interp.register_native(
        "computer_click",
        n("computer_click", 3, Some(3), s31_click),
    );
    interp.register_native(
        "computer_double_click",
        n("computer_double_click", 2, Some(2), s31_double_click),
    );
    interp.register_native(
        "computer_mouse_move",
        n("computer_mouse_move", 2, Some(2), s31_mouse_move),
    );
    interp.register_native(
        "computer_drag",
        n("computer_drag", 5, Some(5), s31_drag),
    );
    interp.register_native(
        "computer_scroll",
        n("computer_scroll", 2, Some(2), s31_scroll),
    );
    interp.register_native(
        "computer_type",
        n("computer_type", 1, Some(1), s31_type),
    );
    interp.register_native(
        "computer_key",
        n("computer_key", 1, Some(1), s31_key),
    );
    interp.register_native(
        "computer_wait",
        n("computer_wait", 1, Some(1), s31_wait),
    );
    interp.register_native(
        "computer_action_log",
        n("computer_action_log", 0, Some(0), s31_action_log),
    );

    // §56.3 — schema → GBNF grammar.
    interp.register_native_ext(
        "schema_to_gbnf",
        ext("schema_to_gbnf", 1, Some(1), s31_schema_to_gbnf),
    );
}

fn parse_button(s: &str) -> cu::MouseButton {
    match s {
        "right" => cu::MouseButton::Right,
        "middle" => cu::MouseButton::Middle,
        _ => cu::MouseButton::Left,
    }
}

fn s31_screenshot(_args: &[Value]) -> Result<Value, String> {
    let shot = COMPUTER_DRIVER
        .with(|c| c.borrow_mut().screenshot())
        .map_err(|e| e.to_string())?;
    Ok(record_to_vec(vec![
        ("width", Value::Int(shot.width as i64)),
        ("height", Value::Int(shot.height as i64)),
        ("format", Value::String(Rc::new(shot.format))),
        ("bytes_len", Value::Int(shot.bytes.len() as i64)),
        ("tainted", Value::Bool(shot.tainted)),
    ]))
}

fn s31_click(args: &[Value]) -> Result<Value, String> {
    let x = i_arg(args, 0, "computer_click")? as i32;
    let y = i_arg(args, 1, "computer_click")? as i32;
    let button = s_arg(args, 2, "computer_click")?;
    let b = parse_button(button.as_str());
    COMPUTER_DRIVER
        .with(|c| c.borrow_mut().click(x, y, b))
        .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn s31_double_click(args: &[Value]) -> Result<Value, String> {
    let x = i_arg(args, 0, "computer_double_click")? as i32;
    let y = i_arg(args, 1, "computer_double_click")? as i32;
    COMPUTER_DRIVER
        .with(|c| c.borrow_mut().double_click(x, y))
        .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn s31_mouse_move(args: &[Value]) -> Result<Value, String> {
    let x = i_arg(args, 0, "computer_mouse_move")? as i32;
    let y = i_arg(args, 1, "computer_mouse_move")? as i32;
    COMPUTER_DRIVER
        .with(|c| c.borrow_mut().mouse_move(x, y))
        .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn s31_drag(args: &[Value]) -> Result<Value, String> {
    let fx = i_arg(args, 0, "computer_drag")? as i32;
    let fy = i_arg(args, 1, "computer_drag")? as i32;
    let tx = i_arg(args, 2, "computer_drag")? as i32;
    let ty = i_arg(args, 3, "computer_drag")? as i32;
    let button = s_arg(args, 4, "computer_drag")?;
    let b = parse_button(button.as_str());
    COMPUTER_DRIVER
        .with(|c| c.borrow_mut().drag((fx, fy), (tx, ty), b))
        .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn s31_scroll(args: &[Value]) -> Result<Value, String> {
    let dx = i_arg(args, 0, "computer_scroll")? as i32;
    let dy = i_arg(args, 1, "computer_scroll")? as i32;
    COMPUTER_DRIVER
        .with(|c| c.borrow_mut().scroll(dx, dy))
        .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn s31_type(args: &[Value]) -> Result<Value, String> {
    let text = s_arg(args, 0, "computer_type")?;
    COMPUTER_DRIVER
        .with(|c| c.borrow_mut().type_text(text.as_str()))
        .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn s31_key(args: &[Value]) -> Result<Value, String> {
    let name = s_arg(args, 0, "computer_key")?;
    COMPUTER_DRIVER
        .with(|c| c.borrow_mut().key(name.as_str()))
        .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn s31_wait(args: &[Value]) -> Result<Value, String> {
    let ms = i_arg(args, 0, "computer_wait")?.max(0) as u64;
    COMPUTER_DRIVER
        .with(|c| c.borrow_mut().wait(ms))
        .map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn s31_action_log(_args: &[Value]) -> Result<Value, String> {
    let log = COMPUTER_DRIVER.with(|c| {
        c.borrow()
            .action_log()
            .iter()
            .map(|a| serde_json::to_string(a).unwrap_or_default())
            .collect::<Vec<_>>()
    });
    Ok(list_value(
        log.into_iter().map(|s| Value::String(Rc::new(s))).collect(),
    ))
}

fn s31_schema_to_gbnf(
    interp: &mut Interpreter,
    args: &[Value],
    _span: axon_diag::Span,
) -> Result<Value, String> {
    // The host doesn't carry the parsed Program; the runtime stores
    // schemas in `schemas` (a name → fields map). The GBNF emitter
    // needs a `SchemaDecl`, so we reconstruct one from the runtime's
    // schema table.
    let _ = interp;
    let name = s_arg(args, 0, "schema_to_gbnf")?;
    let body = schema_to_gbnf_string(name.as_str())
        .ok_or_else(|| format!("schema_to_gbnf: no schema named `{}`", name.as_str()))?;
    Ok(Value::String(Rc::new(body)))
}

/// Look up a schema by name in the interpreter's schema table and
/// emit the GBNF grammar. The table holds `(field_name, ast_type)`
/// pairs — enough for the emitter.
fn schema_to_gbnf_string(name: &str) -> Option<String> {
    // We don't have direct access to the interpreter's schema table
    // from a `NativeExtFn` without plumbing — store schemas on a
    // process-wide thread-local mirror at program load time. For v0
    // the program registers schemas via `schema_register` (below).
    SCHEMA_MIRROR.with(|m| {
        let decl = m.borrow().get(name).cloned()?;
        Some(axon_tyck::gbnf::emit_for_schema(&decl))
    })
}

thread_local! {
    /// At program load the host registers every `schema` AST node
    /// here so `schema_to_gbnf` can find them at runtime. Kept in
    /// sync via `register_schemas`.
    static SCHEMA_MIRROR: RefCell<std::collections::HashMap<String, axon_ast::SchemaDecl>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Mirror every `Item::Schema` in `program` into the SCHEMA_MIRROR.
/// Called by the CLI after `load_program`.
pub fn register_schemas(program: &axon_ast::Program) {
    SCHEMA_MIRROR.with(|m| {
        let mut map = m.borrow_mut();
        map.clear();
        for item in &program.items {
            if let axon_ast::Item::Schema(s) = item {
                map.insert(s.name.name.clone(), s.clone());
            }
        }
    });
}

// ===========================================================================
// §32 — async I/O slice: `flow_parallel_asks`
//
// The first real piece of the async-runtime migration. Each ask in a batch
// is dispatched on `tokio::spawn_blocking` so the *waits* — which is where
// model-call latency lives — overlap on a thread pool. The interpreter
// itself stays single-threaded; only the work behind `provider.complete`
// (sync `ureq`, sync mock) crosses thread boundaries.
//
// Determinism is preserved by joining and recording in **input order**, not
// completion order. Replaying a recording produces byte-identical output
// regardless of which task finished first during the original run.
//
// Acceptance: 3 mock asks each sleeping 200 ms run in < 400 ms wall time
// (vs ~600 ms for the serial `flow_parallel` from Stage 28).
// ===========================================================================

/// Singleton tokio runtime used to dispatch parallel model I/O. Held for
/// the lifetime of the process. `OnceLock` (not `thread_local`) because
/// the runtime must outlive `block_on` and its workers run on detached
/// threads — a thread-local would tear the runtime down at first thread
/// exit and leak the workers.
fn parallel_runtime() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("axon-parallel")
            // Bounded worker pool — large enough to make 8-way parallelism
            // free, small enough that a runaway `flow_parallel_asks` can't
            // exhaust file descriptors / RAM.
            .worker_threads(num_workers())
            .build()
            .expect("tokio runtime")
    })
}

fn num_workers() -> usize {
    // The model I/O is blocking — `spawn_blocking` runs on a separate
    // blocking pool whose default is 512 threads. Four worker threads on
    // the core executor are plenty to drive coordination tasks.
    4
}

fn install_stage32(interp: &Interpreter) {
    interp.register_native_ext(
        "flow_parallel_asks",
        ext("flow_parallel_asks", 1, Some(1), flow_parallel_asks_impl),
    );
    // Testing-only: a mock model whose `complete` sleeps `ms` before
    // returning `text`. Used by the wall-time acceptance test to prove
    // that `flow_parallel_asks` actually overlaps the *waits* (which is
    // where real-world model latency lives). Not registered with the
    // type checker as a typed function — it's a builtin, callable but
    // not exposed in the language docs.
    interp.register_native(
        "mock_model_slow",
        n("mock_model_slow", 2, Some(2), s32_mock_model_slow),
    );
}

fn s32_mock_model_slow(args: &[Value]) -> Result<Value, String> {
    let text = s_arg(args, 0, "mock_model_slow")?;
    let ms = i_arg(args, 1, "mock_model_slow")?.max(0) as u64;
    let provider = SleepingProvider {
        text: text.as_str().to_owned(),
        delay_ms: ms,
        name: format!("mock-slow({ms}ms)"),
    };
    Ok(Value::Model(std::sync::Arc::new(provider)))
}

/// Deterministic mock that sleeps `delay_ms` before returning `text`. We
/// keep it inside the CLI crate (not `axon-models`) so the public mock
/// surface stays small — this exists only to make the acceptance test
/// honest about overlapping waits.
struct SleepingProvider {
    text: String,
    delay_ms: u64,
    name: String,
}

impl axon_models::ModelProvider for SleepingProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn complete(
        &self,
        _req: &axon_models::ChatRequest,
    ) -> Result<axon_models::ChatResponse, axon_models::ProviderError> {
        std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
        Ok(axon_models::ChatResponse {
            content: self.text.clone(),
            blocks: vec![axon_models::ContentBlock::Text(self.text.clone())],
            structured: None,
            tool_calls: Vec::new(),
            usage: axon_models::TokenUsage::default(),
            stop_reason: axon_models::StopReason::EndTurn,
        })
    }
}

/// `flow_parallel_asks(asks: List<{ target, user, system?, max_tokens? }>) -> List<String>`
///
/// `target` is the model handle (we can't call the field `model` because
/// `model` is a reserved keyword at the language level). Runs every ask
/// concurrently on the tokio thread pool, then returns the response texts
/// in **input order** — so a caller writing
///
///     let [a, b, c] = flow_parallel_asks([{target: m, user: q1}, ...])
///
/// can pattern-match by position the same way they would after three
/// serial `ask`s.
///
/// Errors:
///   - Top-level error if `LLM` (and `Net` for real providers) isn't in scope.
///   - Top-level error if a budget was already breached.
///   - Per-slot errors are surfaced as the string `"<error: ...>"` in the
///     output list rather than aborting the whole batch — this matches the
///     existing `flow_parallel` semantics for branch failures.
fn flow_parallel_asks_impl(
    interp: &mut Interpreter,
    args: &[Value],
    span: axon_diag::Span,
) -> Result<Value, String> {
    let items = match &args[0] {
        Value::List(l) => l.borrow().clone(),
        other => {
            return Err(format!(
                "flow_parallel_asks: expected a List of ask records, got `{}`",
                other.type_name()
            ));
        }
    };
    if items.is_empty() {
        return Ok(list_value(Vec::new()));
    }
    if items.len() > 64 {
        return Err(format!(
            "flow_parallel_asks: batch size {} exceeds the safety ceiling (64). \
             Split the batch or raise the ceiling explicitly.",
            items.len()
        ));
    }

    // Capability gate: LLM is always required. Net is only required if any
    // provider is going to actually hit the network — but we don't know
    // that without inspecting providers, so require it like a normal ask.
    if !interp.caps_have("LLM") {
        return Err(
            "flow_parallel_asks: capability `LLM` is not in scope (caller's `uses {{...}}` row)"
                .into(),
        );
    }
    if !interp.caps_have("Net") {
        return Err(
            "flow_parallel_asks: capability `Net` is not in scope (caller's `uses {{...}}` row)"
                .into(),
        );
    }

    // Parse each item into (provider, request) — fail fast on shape errors
    // before doing any I/O.
    let mut batch: Vec<(std::sync::Arc<dyn axon_models::ModelProvider>, axon_models::ChatRequest)> =
        Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let (provider, request) = parse_parallel_ask(item, i)?;
        batch.push((provider, request));
    }

    // Replay short-circuit: pop N events from the recording in input order.
    // The providers are NOT touched — same as how `ask` behaves under
    // replay.
    if interp.replay_active() {
        let mut out: Vec<Value> = Vec::with_capacity(batch.len());
        for _ in 0..batch.len() {
            let resp = interp
                .pop_replay_model_call(span)
                .map_err(|e| eval_signal_msg(&e))?;
            out.push(Value::String(Rc::new(resp.content)));
        }
        return Ok(list_value(out));
    }

    // Budget precheck: refuse the batch if a previous call already put us
    // over. Mirrors the precheck inside `call_provider`.
    interp.precheck_budget(span).map_err(|e| eval_signal_msg(&e))?;

    // Dispatch: `spawn_blocking` each provider.complete on the tokio
    // blocking pool. The interpreter is NOT shared with those tasks —
    // only the Arc<dyn ModelProvider> and an owned ChatRequest move
    // across the thread boundary. Sound because the trait is Send+Sync.
    let rt = parallel_runtime();
    let mut handles: Vec<tokio::task::JoinHandle<Result<axon_models::ChatResponse, axon_models::ProviderError>>> =
        Vec::with_capacity(batch.len());
    for (provider, request) in &batch {
        let p = provider.clone();
        let r = request.clone();
        handles.push(rt.spawn_blocking(move || p.complete(&r)));
    }
    // Join in input order — NOT completion order. This is what makes
    // replay byte-identical to a serial run.
    let results: Vec<Result<axon_models::ChatResponse, String>> = rt.block_on(async move {
        let mut out = Vec::with_capacity(handles.len());
        for h in handles {
            let r = match h.await {
                Ok(Ok(resp)) => Ok(resp),
                Ok(Err(e)) => Err(e.to_string()),
                Err(join_err) => Err(format!("task panicked: {join_err}")),
            };
            out.push(r);
        }
        out
    });

    // Now that we have all responses in input order, walk them once:
    //   1. record (in order) — replay sees the same sequence a serial run would
    //   2. debit budgets (in order) — same as serial
    //   3. project to the user-facing String list
    let mut out: Vec<Value> = Vec::with_capacity(results.len());
    for ((provider, _req), res) in batch.iter().zip(results.into_iter()) {
        match res {
            Ok(resp) => {
                interp.record_model_call(provider.name(), resp.clone());
                interp.debit_budget_for(&resp);
                out.push(Value::String(Rc::new(resp.content)));
            }
            Err(msg) => {
                out.push(Value::String(Rc::new(format!(
                    "<error: model `{}`: {msg}>",
                    provider.name()
                ))));
            }
        }
    }
    Ok(list_value(out))
}

/// Pull `(model, ChatRequest)` out of one record entry. The record shape
/// mirrors `ask`'s message form: `{ model, user, system?, max_tokens? }`.
fn parse_parallel_ask(
    v: &Value,
    idx: usize,
) -> Result<(std::sync::Arc<dyn axon_models::ModelProvider>, axon_models::ChatRequest), String> {
    let fields = match v {
        Value::Record(r) => r.borrow().clone(),
        other => {
            return Err(format!(
                "flow_parallel_asks: item {idx} must be a Record, got `{}`",
                other.type_name()
            ));
        }
    };
    let mut provider: Option<std::sync::Arc<dyn axon_models::ModelProvider>> = None;
    let mut user: Option<String> = None;
    let mut system: Option<String> = None;
    let mut max_tokens: u32 = 1024;
    for (k, val) in &fields {
        match k.as_str() {
            "target" => match val {
                Value::Model(p) => provider = Some(p.clone()),
                other => {
                    return Err(format!(
                        "flow_parallel_asks: item {idx}.target must be a Model, got `{}`",
                        other.type_name()
                    ));
                }
            },
            "user" => match val {
                Value::String(s) => user = Some(s.as_str().to_owned()),
                other => {
                    return Err(format!(
                        "flow_parallel_asks: item {idx}.user must be a String, got `{}`",
                        other.type_name()
                    ));
                }
            },
            "system" => match val {
                Value::String(s) => system = Some(s.as_str().to_owned()),
                Value::Nil => {}
                other => {
                    return Err(format!(
                        "flow_parallel_asks: item {idx}.system must be a String, got `{}`",
                        other.type_name()
                    ));
                }
            },
            "max_tokens" => match val {
                Value::Int(n) if *n > 0 => max_tokens = *n as u32,
                Value::Nil => {}
                other => {
                    return Err(format!(
                        "flow_parallel_asks: item {idx}.max_tokens must be a positive Int, got `{}`",
                        other.type_name()
                    ));
                }
            },
            _ => {
                // Unknown fields are ignored — keeps the record forward-
                // compatible with future ask options without breaking the
                // current parser.
            }
        }
    }
    let provider = provider.ok_or_else(|| {
        format!("flow_parallel_asks: item {idx} missing required field `target`")
    })?;
    let user = user.ok_or_else(|| {
        format!("flow_parallel_asks: item {idx} missing required field `user`")
    })?;
    let req = axon_models::ChatRequest {
        model: String::new(),
        system,
        messages: vec![axon_models::Message::user_text(user)],
        max_tokens,
        temperature: None,
        stop_sequences: Vec::new(),
        output_schema: None,
        output_schema_name: None,
        tools: Vec::new(),
    };
    Ok((provider, req))
}

