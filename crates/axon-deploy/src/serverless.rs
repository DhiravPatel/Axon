//! Serverless deployment targets (§41).
//!
//! `#[lambda]`, `#[gcp_function]`, `#[cf_worker]` are *packaging hints*
//! — they don't change the agent's runtime semantics, they change how
//! `axon deploy` shapes the output bundle. The spec promises that
//! adding the attribute to an `agent` declaration is sufficient to
//! produce a serverless-compatible artifact.
//!
//! v0 ships:
//!
//!   * The typed [`ServerlessTarget`] enum, used by both the manifest
//!     loader and the deploy renderer.
//!   * [`ServerlessTrampoline`] — a small JSON-in / JSON-out adapter
//!     contract the deploy bundle declares so the chosen platform's
//!     entrypoint (Lambda's `handler`, GCP's HTTP function, CF
//!     Worker's `fetch`) can hand a request to the Axon handler and
//!     receive a response.
//!   * `render_dockerfile` / `render_lambda_yaml` / `render_cf_toml`
//!     emit ready-to-deploy scaffolding alongside the `.axskill`
//!     archive `axon deploy` already writes.
//!
//! The output is *manifest scaffolding*: real platform-specific
//! plumbing (Lambda runtime layer, GCP API endpoint, CF deploy token)
//! is the operator's job. We give them the shape; they wire the rest.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerlessTarget {
    /// AWS Lambda. Renders a `template.yaml` (SAM) snippet.
    Lambda,
    /// Google Cloud Functions / Cloud Run.
    GcpFunction,
    /// Cloudflare Workers.
    CfWorker,
}

impl ServerlessTarget {
    pub fn from_attribute(name: &str) -> Option<Self> {
        match name {
            "lambda" => Some(ServerlessTarget::Lambda),
            "gcp_function" => Some(ServerlessTarget::GcpFunction),
            "cf_worker" => Some(ServerlessTarget::CfWorker),
            _ => None,
        }
    }

    pub fn attribute_name(self) -> &'static str {
        match self {
            ServerlessTarget::Lambda => "lambda",
            ServerlessTarget::GcpFunction => "gcp_function",
            ServerlessTarget::CfWorker => "cf_worker",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerlessTrampoline {
    pub target: ServerlessTarget,
    /// Name of the Axon handler the trampoline calls.
    pub handler: String,
    /// Memory limit in MB (Lambda/GCP convention).
    #[serde(default = "default_memory_mb")]
    pub memory_mb: u32,
    /// Wall-clock timeout in seconds.
    #[serde(default = "default_timeout_s")]
    pub timeout_s: u32,
    /// Environment variables to expose at runtime.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

fn default_memory_mb() -> u32 {
    256
}
fn default_timeout_s() -> u32 {
    30
}

impl ServerlessTrampoline {
    pub fn new(target: ServerlessTarget, handler: impl Into<String>) -> Self {
        Self {
            target,
            handler: handler.into(),
            memory_mb: default_memory_mb(),
            timeout_s: default_timeout_s(),
            env: std::collections::BTreeMap::new(),
        }
    }
}

/// Render an AWS SAM template fragment for `tramp`.
pub fn render_lambda_yaml(tramp: &ServerlessTrampoline, skill_name: &str) -> String {
    debug_assert!(matches!(tramp.target, ServerlessTarget::Lambda));
    let mut env_block = String::new();
    if !tramp.env.is_empty() {
        env_block.push_str("        Variables:\n");
        for (k, v) in &tramp.env {
            env_block.push_str(&format!("          {k}: {}\n", quote_yaml(v)));
        }
    }
    format!(
        "AWSTemplateFormatVersion: '2010-09-09'\nTransform: AWS::Serverless-2016-10-31\nResources:\n  {skill}Fn:\n    Type: AWS::Serverless::Function\n    Properties:\n      Handler: bootstrap\n      Runtime: provided.al2023\n      Architectures: [arm64]\n      MemorySize: {mem}\n      Timeout: {to}\n      CodeUri: ./{skill}.axskill\n      Environment:\n{env_block}\n      Events:\n        ApiEvent:\n          Type: HttpApi\n          Properties:\n            Path: /invoke\n            Method: POST\nMetadata:\n  AxonHandler: {handler}\n",
        skill = skill_name,
        mem = tramp.memory_mb,
        to = tramp.timeout_s,
        env_block = if env_block.is_empty() { "        Variables: {}\n".into() } else { env_block },
        handler = tramp.handler,
    )
}

/// Render a GCP Functions `function.yaml`.
pub fn render_gcp_function_yaml(tramp: &ServerlessTrampoline, skill_name: &str) -> String {
    debug_assert!(matches!(tramp.target, ServerlessTarget::GcpFunction));
    let mut env_lines = String::new();
    for (k, v) in &tramp.env {
        env_lines.push_str(&format!("  {k}: {}\n", quote_yaml(v)));
    }
    format!(
        "name: {skill}\nentryPoint: {handler}\nruntime: rust-bin\navailableMemoryMb: {mem}\ntimeoutSeconds: {to}\nsourceArchiveUrl: ./{skill}.axskill\nenvironmentVariables:\n{env_lines}",
        skill = skill_name,
        handler = tramp.handler,
        mem = tramp.memory_mb,
        to = tramp.timeout_s,
        env_lines = if env_lines.is_empty() { "  {}\n".into() } else { env_lines },
    )
}

/// Render a Cloudflare Workers `wrangler.toml`.
pub fn render_cf_worker_toml(tramp: &ServerlessTrampoline, skill_name: &str) -> String {
    debug_assert!(matches!(tramp.target, ServerlessTarget::CfWorker));
    let mut vars = String::new();
    if !tramp.env.is_empty() {
        vars.push_str("[vars]\n");
        for (k, v) in &tramp.env {
            vars.push_str(&format!("{k} = {}\n", quote_toml(v)));
        }
    }
    format!(
        "name = \"{skill}\"\nmain = \"./worker.js\"\ncompatibility_date = \"2025-01-01\"\n[build]\ncommand = \"axon deploy . -o dist --target=cf_worker\"\n[axon]\nhandler = \"{handler}\"\ntimeout_seconds = {to}\n\n{vars}",
        skill = skill_name,
        handler = tramp.handler,
        to = tramp.timeout_s,
        vars = vars,
    )
}

fn quote_yaml(s: &str) -> String {
    if s.contains(':') || s.contains('#') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}
fn quote_toml(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_attribute_maps_known_names() {
        assert_eq!(
            ServerlessTarget::from_attribute("lambda"),
            Some(ServerlessTarget::Lambda)
        );
        assert_eq!(
            ServerlessTarget::from_attribute("gcp_function"),
            Some(ServerlessTarget::GcpFunction)
        );
        assert_eq!(
            ServerlessTarget::from_attribute("cf_worker"),
            Some(ServerlessTarget::CfWorker)
        );
        assert_eq!(ServerlessTarget::from_attribute("not-a-target"), None);
    }

    #[test]
    fn lambda_yaml_mentions_axskill_and_handler() {
        let mut t = ServerlessTrampoline::new(ServerlessTarget::Lambda, "main");
        t.env.insert("LOG_LEVEL".into(), "info".into());
        let y = render_lambda_yaml(&t, "research");
        assert!(y.contains("research.axskill"));
        assert!(y.contains("AxonHandler: main"));
        assert!(y.contains("LOG_LEVEL"));
    }

    #[test]
    fn gcp_yaml_uses_skill_name() {
        let t = ServerlessTrampoline::new(ServerlessTarget::GcpFunction, "h");
        let y = render_gcp_function_yaml(&t, "support");
        assert!(y.contains("name: support"));
        assert!(y.contains("entryPoint: h"));
    }

    #[test]
    fn cf_toml_emits_wrangler_block() {
        let mut t = ServerlessTrampoline::new(ServerlessTarget::CfWorker, "edge");
        t.env.insert("API_BASE".into(), "https://api.example.com".into());
        let y = render_cf_worker_toml(&t, "edge-bot");
        assert!(y.contains("name = \"edge-bot\""));
        assert!(y.contains("handler = \"edge\""));
        assert!(y.contains("API_BASE"));
    }

    #[test]
    fn yaml_quote_escapes_colons() {
        let q = quote_yaml("a:b");
        assert!(q.starts_with('"') && q.ends_with('"'));
    }

    #[test]
    fn trampoline_round_trips_through_json() {
        let mut t = ServerlessTrampoline::new(ServerlessTarget::Lambda, "h");
        t.memory_mb = 512;
        t.timeout_s = 60;
        t.env.insert("X".into(), "y".into());
        let j = serde_json::to_string(&t).unwrap();
        let back: ServerlessTrampoline = serde_json::from_str(&j).unwrap();
        assert_eq!(back, t);
    }
}
