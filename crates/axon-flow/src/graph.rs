//! Workflow graph — explicit, inspectable DAG of typed steps (§29.6).
//!
//! A `WorkflowGraph` is a directed acyclic graph of named nodes, each of
//! which carries a payload (string label today; the host crate may swap in
//! `Value` callbacks). The library provides:
//!
//!   * structural checks (`add_node`, `add_edge`, `verify` — cycle &
//!     unknown-node detection mirrors [`crate::network::Network`]);
//!   * Kahn's-algorithm topological order
//!     (`topological_order` — used by the runtime to schedule nodes
//!     in legal execution order while letting independent nodes run
//!     concurrently);
//!   * deterministic JSON serialization for `axon trace --graph`.
//!
//! Like `Network`, the parser-level `graph { ... }` declaration is left
//! to a later edition; users build a graph procedurally via host bindings.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    pub name: String,
    /// Free-form label used by the runtime; the host crate stores a
    /// step-callable id here.
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowGraph {
    pub name: String,
    pub nodes: BTreeMap<String, GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    UnknownNode { edge_from: String, edge_to: String, missing: String },
    Cycle { path: Vec<String> },
    DuplicateNode(String),
    Empty,
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::UnknownNode { edge_from, edge_to, missing } => write!(
                f,
                "graph edge {edge_from}->{edge_to} references unknown node `{missing}`"
            ),
            GraphError::Cycle { path } => write!(f, "graph has cycle: {}", path.join(" -> ")),
            GraphError::DuplicateNode(n) => write!(f, "duplicate node `{n}`"),
            GraphError::Empty => f.write_str("graph has no nodes"),
        }
    }
}

impl std::error::Error for GraphError {}

impl WorkflowGraph {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            nodes: BTreeMap::new(),
            edges: Vec::new(),
        }
    }

    pub fn add_node(
        &mut self,
        name: impl Into<String>,
        label: impl Into<String>,
    ) -> Result<(), GraphError> {
        let n = name.into();
        if self.nodes.contains_key(&n) {
            return Err(GraphError::DuplicateNode(n));
        }
        self.nodes.insert(
            n.clone(),
            GraphNode {
                name: n,
                label: label.into(),
            },
        );
        Ok(())
    }

    pub fn add_edge(&mut self, from: impl Into<String>, to: impl Into<String>) {
        self.edges.push(GraphEdge {
            from: from.into(),
            to: to.into(),
        });
    }

    pub fn verify(&self) -> Result<(), GraphError> {
        if self.nodes.is_empty() {
            return Err(GraphError::Empty);
        }
        for e in &self.edges {
            if !self.nodes.contains_key(&e.from) {
                return Err(GraphError::UnknownNode {
                    edge_from: e.from.clone(),
                    edge_to: e.to.clone(),
                    missing: e.from.clone(),
                });
            }
            if !self.nodes.contains_key(&e.to) {
                return Err(GraphError::UnknownNode {
                    edge_from: e.from.clone(),
                    edge_to: e.to.clone(),
                    missing: e.to.clone(),
                });
            }
        }
        // Kahn's algorithm — runs in O(V+E). If we can't drain all nodes
        // there's a cycle; reconstruct one for the error.
        let _ = self.topological_order()?;
        Ok(())
    }

    /// Kahn's algorithm; nodes within an in-degree-zero layer are returned
    /// in alphabetical order for deterministic scheduling.
    pub fn topological_order(&self) -> Result<Vec<String>, GraphError> {
        let mut indeg: BTreeMap<&str, usize> = BTreeMap::new();
        for n in self.nodes.keys() {
            indeg.insert(n.as_str(), 0);
        }
        let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for n in self.nodes.keys() {
            adj.entry(n.as_str()).or_default();
        }
        for e in &self.edges {
            *indeg.entry(e.to.as_str()).or_insert(0) += 1;
            adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
        }
        let mut q: VecDeque<&str> = indeg
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(k, _)| *k)
            .collect();
        // Sort the queue so the result is deterministic.
        let mut sorted: Vec<&str> = q.drain(..).collect();
        sorted.sort();
        let mut q: VecDeque<&str> = sorted.into_iter().collect();

        let mut order: Vec<String> = Vec::with_capacity(self.nodes.len());
        while let Some(n) = q.pop_front() {
            order.push(n.to_string());
            let mut succs: Vec<&str> = adj.get(n).cloned().unwrap_or_default();
            succs.sort();
            succs.dedup();
            for s in succs {
                let d = indeg.entry(s).or_insert(0);
                *d = d.saturating_sub(1);
                if *d == 0 {
                    q.push_back(s);
                }
            }
        }
        if order.len() != self.nodes.len() {
            // Reconstruct a cycle path by DFS from any still-in-degree>0 node.
            let mut stuck: Vec<&str> = indeg
                .iter()
                .filter(|(_, &d)| d > 0)
                .map(|(k, _)| *k)
                .collect();
            stuck.sort();
            let path = self.find_cycle_from(stuck.first().copied().unwrap_or(""));
            return Err(GraphError::Cycle { path });
        }
        Ok(order)
    }

    fn find_cycle_from(&self, start: &str) -> Vec<String> {
        let mut on_stack: BTreeSet<&str> = BTreeSet::new();
        let mut path: Vec<&str> = Vec::new();
        let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for e in &self.edges {
            adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
        }
        fn dfs<'a>(
            node: &'a str,
            adj: &BTreeMap<&'a str, Vec<&'a str>>,
            on_stack: &mut BTreeSet<&'a str>,
            path: &mut Vec<&'a str>,
        ) -> Option<Vec<String>> {
            on_stack.insert(node);
            path.push(node);
            if let Some(succs) = adj.get(node) {
                for &s in succs {
                    if on_stack.contains(s) {
                        // Found cycle.
                        let pos = path.iter().position(|p| *p == s).unwrap_or(0);
                        let mut cycle: Vec<String> =
                            path[pos..].iter().map(|s| s.to_string()).collect();
                        cycle.push(s.to_string());
                        return Some(cycle);
                    }
                    if let Some(c) = dfs(s, adj, on_stack, path) {
                        return Some(c);
                    }
                }
            }
            on_stack.remove(node);
            path.pop();
            None
        }
        if start.is_empty() {
            return Vec::new();
        }
        dfs(start, &adj, &mut on_stack, &mut path).unwrap_or_default()
    }

    /// Roots: nodes with no incoming edges. These are the entry points.
    pub fn roots(&self) -> Vec<String> {
        let mut incoming: BTreeSet<&str> = BTreeSet::new();
        for e in &self.edges {
            incoming.insert(e.to.as_str());
        }
        let mut out: Vec<String> = self
            .nodes
            .keys()
            .filter(|n| !incoming.contains(n.as_str()))
            .cloned()
            .collect();
        out.sort();
        out
    }

    /// Leaves: nodes with no outgoing edges.
    pub fn leaves(&self) -> Vec<String> {
        let mut outgoing: BTreeSet<&str> = BTreeSet::new();
        for e in &self.edges {
            outgoing.insert(e.from.as_str());
        }
        let mut out: Vec<String> = self
            .nodes
            .keys()
            .filter(|n| !outgoing.contains(n.as_str()))
            .cloned()
            .collect();
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triage() -> WorkflowGraph {
        let mut g = WorkflowGraph::new("TriageFlow");
        g.add_node("classify", "fast").unwrap();
        g.add_node("retrieve", "kb").unwrap();
        g.add_node("draft", "brain").unwrap();
        g.add_node("review", "judge").unwrap();
        g.add_edge("classify", "draft");
        g.add_edge("retrieve", "draft");
        g.add_edge("draft", "review");
        g
    }

    #[test]
    fn topo_order_respects_dependencies() {
        let g = triage();
        g.verify().unwrap();
        let order = g.topological_order().unwrap();
        // draft must come after both classify and retrieve.
        let pos = |n: &str| order.iter().position(|s| s == n).unwrap();
        assert!(pos("classify") < pos("draft"));
        assert!(pos("retrieve") < pos("draft"));
        assert!(pos("draft") < pos("review"));
    }

    #[test]
    fn roots_and_leaves_identified() {
        let g = triage();
        assert_eq!(g.roots(), vec!["classify", "retrieve"]);
        assert_eq!(g.leaves(), vec!["review"]);
    }

    #[test]
    fn cycle_in_graph_is_rejected() {
        let mut g = WorkflowGraph::new("X");
        g.add_node("a", "").unwrap();
        g.add_node("b", "").unwrap();
        g.add_edge("a", "b");
        g.add_edge("b", "a");
        let err = g.verify().unwrap_err();
        assert!(matches!(err, GraphError::Cycle { .. }));
    }

    #[test]
    fn unknown_node_in_edge_caught() {
        let mut g = WorkflowGraph::new("X");
        g.add_node("a", "").unwrap();
        g.add_edge("a", "ghost");
        let err = g.verify().unwrap_err();
        match err {
            GraphError::UnknownNode { missing, .. } => assert_eq!(missing, "ghost"),
            other => panic!("expected UnknownNode, got {other:?}"),
        }
    }

    #[test]
    fn deterministic_topo_order_for_independent_nodes() {
        let mut g = WorkflowGraph::new("X");
        g.add_node("z", "").unwrap();
        g.add_node("a", "").unwrap();
        g.add_node("m", "").unwrap();
        let order = g.topological_order().unwrap();
        // No edges -> all are roots -> alphabetical.
        assert_eq!(order, vec!["a", "m", "z"]);
    }
}
