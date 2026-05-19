use std::fmt;

/// Errors surfaced by flow combinators.
///
/// `path` is a breadcrumb the combinator adds so callers can see *where*
/// in the pipeline the failure happened — e.g. `"sequential[2]"`,
/// `"parallel[branch=4]"`, `"refine[round=2]"`.
#[derive(Debug, Clone)]
pub struct FlowError {
    pub message: String,
    pub path: Vec<String>,
}

impl FlowError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            path: Vec::new(),
        }
    }

    pub fn with_step(mut self, step: impl Into<String>) -> Self {
        self.path.push(step.into());
        self
    }
}

impl fmt::Display for FlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            f.write_str(&self.message)
        } else {
            // Outer-most context first for readability.
            let trail: Vec<String> = self.path.iter().rev().cloned().collect();
            write!(f, "{}: {}", trail.join(" / "), self.message)
        }
    }
}

impl std::error::Error for FlowError {}
