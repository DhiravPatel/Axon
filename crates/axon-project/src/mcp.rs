//! MCP server declarations from `axon.toml` (§25.5).
//!
//! Programs that want to import tools from a Model Context Protocol
//! server declare them in their manifest:
//!
//! ```toml
//! [tools.github]
//! mcp = "https://api.example.com/mcp"        # remote MCP server
//!
//! [tools.local]
//! mcp_command = "node ./mcp-fs/index.js"     # subprocess MCP server
//!
//! [tools.calculator]
//! tools = [
//!     { name = "add", description = "Sum two numbers",
//!       input_schema = "{\"type\":\"object\"}" },
//! ]
//! ```
//!
//! At project load time the runtime resolves every `[tools.<name>]`
//! entry into a [`McpTool`] list. Programs see those as tool-shaped
//! callables under the `<name>.<tool>` namespace — identical to how
//! locally-declared `tool ...` items are bound. The MCP wire protocol
//! lives behind the [`McpClient`] trait so different transports
//! (HTTP/SSE, stdio JSON-RPC, websocket) can drop in without changing
//! call sites.
//!
//! v0 ships:
//!
//!   * Manifest parsing for `[tools.<name>]` already wired in
//!     [`crate::ToolDeclaration`].
//!   * `McpRegistry` holding the resolved tool table.
//!   * `McpClient` trait + a deterministic `StaticMcpClient` driver
//!     that returns the inline `tools = [...]` entries the operator
//!     declared. Remote / subprocess drivers slot into the same trait
//!     when the wire client lands.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ToolDeclaration;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpTool {
    /// Namespace from `[tools.<namespace>]`. Tool callable is bound as
    /// `<namespace>.<name>`.
    pub namespace: String,
    pub name: String,
    pub description: String,
    pub input_schema: String,
    /// Provider kind so traces can attribute calls. One of `inline`,
    /// `mcp_url`, `mcp_command`.
    pub provider: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpRegistry {
    pub tools: Vec<McpTool>,
    /// Tool namespaces whose underlying transport is *not yet
    /// available* (because the wire driver hasn't been wired). The
    /// host emits a one-time warning per namespace so users know why
    /// `<namespace>.<tool>` returns a stub.
    pub deferred_namespaces: Vec<String>,
}

impl McpRegistry {
    /// Resolve every `[tools.<name>]` declaration in a manifest. Inline
    /// `tools = [...]` arrays are always honored; `mcp` URLs and
    /// `mcp_command` subprocesses are recorded in `deferred_namespaces`
    /// until their transport drivers ship.
    pub fn from_manifest_tools(tools: &BTreeMap<String, ToolDeclaration>) -> Self {
        let mut out = McpRegistry::default();
        for (ns, decl) in tools.iter() {
            let provider = if !decl.mcp.is_empty() {
                out.deferred_namespaces.push(ns.clone());
                "mcp_url"
            } else if !decl.mcp_command.is_empty() {
                out.deferred_namespaces.push(ns.clone());
                "mcp_command"
            } else {
                "inline"
            };
            for t in &decl.tools {
                out.tools.push(McpTool {
                    namespace: ns.clone(),
                    name: t.name.clone(),
                    description: t.description.clone(),
                    input_schema: t.input_schema.clone(),
                    provider: provider.to_string(),
                });
            }
        }
        // Sort for deterministic order across runs.
        out.tools
            .sort_by(|a, b| (a.namespace.as_str(), a.name.as_str()).cmp(&(b.namespace.as_str(), b.name.as_str())));
        out.deferred_namespaces.sort();
        out.deferred_namespaces.dedup();
        out
    }

    /// Convenience: convert a `HashMap` (the manifest's runtime
    /// representation) to a sorted `BTreeMap` so resolution is
    /// deterministic.
    pub fn from_hashmap(tools: &std::collections::HashMap<String, ToolDeclaration>) -> Self {
        let mut sorted: BTreeMap<String, ToolDeclaration> = BTreeMap::new();
        for (k, v) in tools.iter() {
            sorted.insert(k.clone(), v.clone());
        }
        Self::from_manifest_tools(&sorted)
    }

    pub fn tools_in(&self, namespace: &str) -> Vec<&McpTool> {
        self.tools.iter().filter(|t| t.namespace == namespace).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }
}

/// Transport-abstracted MCP client. Real implementations: `HttpMcpClient`,
/// `StdioMcpClient`. The contract is the JSON-RPC `tools/list` and
/// `tools/call` methods from the MCP spec; the trait keeps the
/// caller-facing API minimal.
pub trait McpClient {
    fn list_tools(&self) -> Result<Vec<McpTool>, String>;
    fn call_tool(&self, name: &str, args_json: &str) -> Result<String, String>;
}

/// In-memory MCP driver that returns the registry's static tool list.
/// Useful for tests and for declaring tools entirely in `axon.toml`
/// without standing up a separate process. Calls to `call_tool` echo
/// the input back so the program can be exercised end-to-end.
pub struct StaticMcpClient {
    pub registry: McpRegistry,
    pub namespace: String,
}

impl McpClient for StaticMcpClient {
    fn list_tools(&self) -> Result<Vec<McpTool>, String> {
        Ok(self
            .registry
            .tools_in(&self.namespace)
            .into_iter()
            .cloned()
            .collect())
    }

    fn call_tool(&self, name: &str, args_json: &str) -> Result<String, String> {
        let found = self
            .registry
            .tools_in(&self.namespace)
            .into_iter()
            .find(|t| t.name == name);
        match found {
            Some(t) => Ok(format!(
                "{{\"ok\":true,\"namespace\":\"{}\",\"name\":\"{}\",\"echo\":{}}}",
                t.namespace, t.name, args_json
            )),
            None => Err(format!("MCP: no tool `{name}` in namespace `{}`", self.namespace)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InlineTool;

    fn fixture() -> BTreeMap<String, ToolDeclaration> {
        let mut t = BTreeMap::new();
        t.insert(
            "github".into(),
            ToolDeclaration {
                mcp: "https://example.com/mcp".into(),
                mcp_command: String::new(),
                tools: vec![],
            },
        );
        t.insert(
            "calculator".into(),
            ToolDeclaration {
                mcp: String::new(),
                mcp_command: String::new(),
                tools: vec![
                    InlineTool {
                        name: "add".into(),
                        description: "Sum two numbers".into(),
                        input_schema: "{\"type\":\"object\"}".into(),
                    },
                    InlineTool {
                        name: "sub".into(),
                        description: "Subtract two numbers".into(),
                        input_schema: "{\"type\":\"object\"}".into(),
                    },
                ],
            },
        );
        t.insert(
            "local-fs".into(),
            ToolDeclaration {
                mcp: String::new(),
                mcp_command: "node mcp-fs/index.js".into(),
                tools: vec![],
            },
        );
        t
    }

    #[test]
    fn inline_tools_are_registered_deterministically() {
        let reg = McpRegistry::from_manifest_tools(&fixture());
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.tools[0].namespace, "calculator");
        assert_eq!(reg.tools[0].name, "add");
        assert_eq!(reg.tools[1].name, "sub");
    }

    #[test]
    fn remote_and_subprocess_namespaces_deferred() {
        let reg = McpRegistry::from_manifest_tools(&fixture());
        assert!(reg.deferred_namespaces.contains(&"github".to_string()));
        assert!(reg.deferred_namespaces.contains(&"local-fs".to_string()));
        assert!(!reg.deferred_namespaces.contains(&"calculator".to_string()));
    }

    #[test]
    fn static_client_returns_namespace_subset() {
        let reg = McpRegistry::from_manifest_tools(&fixture());
        let c = StaticMcpClient {
            registry: reg,
            namespace: "calculator".into(),
        };
        let tools = c.list_tools().unwrap();
        assert_eq!(tools.len(), 2);
        assert!(tools.iter().all(|t| t.namespace == "calculator"));
    }

    #[test]
    fn static_client_call_unknown_tool_errors() {
        let reg = McpRegistry::from_manifest_tools(&fixture());
        let c = StaticMcpClient {
            registry: reg,
            namespace: "calculator".into(),
        };
        let err = c.call_tool("nope", "{}").unwrap_err();
        assert!(err.contains("no tool `nope`"));
    }

    #[test]
    fn static_client_call_echoes_args() {
        let reg = McpRegistry::from_manifest_tools(&fixture());
        let c = StaticMcpClient {
            registry: reg,
            namespace: "calculator".into(),
        };
        let r = c.call_tool("add", "{\"a\":1}").unwrap();
        assert!(r.contains("\"name\":\"add\""));
        assert!(r.contains("\"a\":1"));
    }

    #[test]
    fn empty_tools_table_yields_empty_registry() {
        let reg = McpRegistry::from_manifest_tools(&BTreeMap::new());
        assert!(reg.is_empty());
        assert!(reg.deferred_namespaces.is_empty());
    }
}
