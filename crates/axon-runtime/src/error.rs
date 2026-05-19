//! Runtime errors and control-flow signals.
//!
//! The evaluator returns `EvalResult<Value>` from every step. Non-error
//! exits (`return`, `break`, `continue`) bubble up the same way ordinary
//! errors do; specialized callers catch and translate them into normal
//! control flow.

use axon_diag::{Diagnostic, Severity, SourceFile, Span};

use crate::value::Value;

pub type EvalResult<T> = Result<T, EvalSignal>;

/// Anything that interrupts ordinary expression evaluation.
#[derive(Debug)]
pub enum EvalSignal {
    /// A normal `return [expr]` from a function.
    Return(Value),
    /// `break [label]`. The optional value is `Unit` for an unvalued break.
    Break {
        label: Option<String>,
    },
    /// `continue [label]`.
    Continue {
        label: Option<String>,
    },
    /// `yield expr` — used by stream blocks. The evaluator treats unhandled
    /// yields as a runtime error in v0 (stream support comes in a later stage).
    Yield(Value),
    /// Genuine runtime error: bad operands, division by zero, missing field,
    /// failed pattern, capability not implemented, etc.
    Error(RuntimeError),
}

#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub message: String,
    pub span: Span,
    pub trace: Vec<TraceFrame>,
}

#[derive(Debug, Clone)]
pub struct TraceFrame {
    pub site: Span,
    pub label: String,
}

impl EvalSignal {
    pub fn error(message: impl Into<String>, span: Span) -> Self {
        EvalSignal::Error(RuntimeError {
            message: message.into(),
            span,
            trace: Vec::new(),
        })
    }

    /// Used at sites that need to clone the error variant only (e.g. when
    /// recording a span on an early-return). Non-`Error` variants clone to
    /// themselves; `Return(v)` clones the value.
    pub fn clone_signal_for_error(&self) -> EvalSignal {
        match self {
            EvalSignal::Error(e) => EvalSignal::Error(e.clone()),
            EvalSignal::Return(v) => EvalSignal::Return(v.clone()),
            EvalSignal::Break { label } => EvalSignal::Break {
                label: label.clone(),
            },
            EvalSignal::Continue { label } => EvalSignal::Continue {
                label: label.clone(),
            },
            EvalSignal::Yield(v) => EvalSignal::Yield(v.clone()),
        }
    }
}

impl RuntimeError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            trace: Vec::new(),
        }
    }

    pub fn with_frame(mut self, frame: TraceFrame) -> Self {
        self.trace.push(frame);
        self
    }

    /// Render this runtime error using the diagnostics module so it shows
    /// up with the same caret-pointed format as compile-time errors.
    pub fn render(&self, file: &SourceFile, use_color: bool) -> String {
        let mut diag = Diagnostic {
            severity: Severity::Error,
            code: Some("R0001"),
            message: format!("runtime error: {}", self.message),
            primary: axon_diag::Label {
                span: self.span,
                message: None,
            },
            secondary: Vec::new(),
            notes: Vec::new(),
        };
        for frame in self.trace.iter().rev() {
            diag.notes.push(format!("called from {}", frame.label));
        }
        axon_diag::render(&diag, file, use_color)
    }
}
