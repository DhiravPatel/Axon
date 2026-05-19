//! Disassembler for AxVM bytecode.
//!
//! Used by `axon disasm <file>` for debugging the compiler and by parity
//! tests to inspect generated chunks.

use std::fmt::Write;

use crate::ops::{Function, Op};
use crate::value::Value;

/// Disassemble all the functions in a compiled program.
pub fn disassemble_program(functions: &[std::rc::Rc<Function>]) -> String {
    let mut out = String::new();
    for (i, f) in functions.iter().enumerate() {
        writeln!(out, "// ── fn #{i}: {} ─────────", f.display_name()).unwrap();
        writeln!(
            out,
            "  arity={} locals={} upvalues={} effects={:?}",
            f.arity,
            f.locals_count,
            f.upvalues.len(),
            f.declared_effects
        )
        .unwrap();
        for (j, c) in f.constants.iter().enumerate() {
            writeln!(out, "  const[{j}] = {c}").unwrap();
        }
        for (ip, op) in f.bytecode.iter().enumerate() {
            writeln!(out, "  {ip:04} {}", format_op(op, f)).unwrap();
        }
        writeln!(out).unwrap();
    }
    out
}

fn format_op(op: &Op, f: &Function) -> String {
    match op {
        Op::LoadConst(idx) => format!("LoadConst {idx} ({})", show_const(f, *idx)),
        Op::LoadGlobal(idx) => format!("LoadGlobal {idx} ({})", show_const(f, *idx)),
        Op::StoreGlobal(idx) => format!("StoreGlobal {idx} ({})", show_const(f, *idx)),
        Op::GetField(idx) => format!("GetField {idx} ({})", show_const(f, *idx)),
        Op::SetField(idx) => format!("SetField {idx} ({})", show_const(f, *idx)),
        Op::MethodCall {
            method_idx,
            argc,
        } => format!(
            "MethodCall {method_idx} ({}) argc={argc}",
            show_const(f, *method_idx)
        ),
        Op::Unsupported(idx) => format!("Unsupported {idx} ({})", show_const(f, *idx)),
        Op::IsType(idx) => format!("IsType {idx} ({})", show_const(f, *idx)),
        other => format!("{other:?}"),
    }
}

fn show_const(f: &Function, idx: u32) -> String {
    f.constants
        .get(idx as usize)
        .map(|c| match c {
            Value::String(s) => format!("\"{s}\""),
            other => other.to_string(),
        })
        .unwrap_or_else(|| "<oor>".into())
}
