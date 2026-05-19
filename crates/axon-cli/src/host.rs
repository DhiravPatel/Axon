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
