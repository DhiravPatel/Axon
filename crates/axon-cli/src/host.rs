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
