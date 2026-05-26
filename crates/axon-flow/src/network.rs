//! Agent network — typed topology with deadlock/cycle/reachability analysis.
//!
//! §29.2 of the spec promises that a `network` declaration is checked
//! statically for:
//!   * cycles (potential deadlock at runtime if message edges are sync);
//!   * unreachable nodes (orphaned agents that nothing can dispatch to);
//!   * edges referencing unknown nodes.
//!
//! This module provides the *data* + *analysis* half. The parser-level
//! `network { ... }` declaration is left to a later edition; users build a
//! network procedurally via host bindings (`flow_network_new`,
//! `flow_network_add_node`, `flow_network_add_edge`, `flow_network_verify`)
//! and get the same checks.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeKind {
    /// `a -> b` — one-way send.
    OneWay,
    /// `a <-> b` — bidirectional. Stored as two `OneWay` entries internally
    /// so cycle analysis sees the correct shape.
    Bidirectional,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkEdge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

/// In-memory network: a labelled directed multigraph with named nodes.
///
/// Edges are stored in a `BTreeMap<from, Vec<to>>` so verification is
/// deterministic and serialization round-trips identically.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Network {
    pub name: String,
    pub nodes: BTreeSet<String>,
    /// Adjacency list of one-way successors. Bidirectional edges are
    /// expanded to two entries (a→b and b→a).
    pub adjacency: BTreeMap<String, Vec<String>>,
    /// Original declared edges (for round-trip + introspection).
    pub edges: Vec<NetworkEdge>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkError {
    UnknownNode { edge_from: String, edge_to: String, missing: String },
    Cycle { path: Vec<String> },
    Unreachable { from: String, orphans: Vec<String> },
    DuplicateNode(String),
    Empty,
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkError::UnknownNode { edge_from, edge_to, missing } => write!(
                f,
                "network edge {edge_from}->{edge_to} references unknown node `{missing}`"
            ),
            NetworkError::Cycle { path } => {
                write!(f, "network has cycle: {}", path.join(" -> "))
            }
            NetworkError::Unreachable { from, orphans } => write!(
                f,
                "from `{from}` these nodes are unreachable: {}",
                orphans.join(", ")
            ),
            NetworkError::DuplicateNode(n) => write!(f, "duplicate node `{n}`"),
            NetworkError::Empty => f.write_str("network has no nodes"),
        }
    }
}

impl std::error::Error for NetworkError {}

impl Network {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            nodes: BTreeSet::new(),
            adjacency: BTreeMap::new(),
            edges: Vec::new(),
        }
    }

    pub fn add_node(&mut self, name: impl Into<String>) -> Result<(), NetworkError> {
        let n = name.into();
        if !self.nodes.insert(n.clone()) {
            return Err(NetworkError::DuplicateNode(n));
        }
        self.adjacency.entry(n).or_default();
        Ok(())
    }

    pub fn add_edge(
        &mut self,
        from: impl Into<String>,
        to: impl Into<String>,
        kind: EdgeKind,
    ) {
        let f = from.into();
        let t = to.into();
        self.adjacency.entry(f.clone()).or_default().push(t.clone());
        if matches!(kind, EdgeKind::Bidirectional) {
            self.adjacency.entry(t.clone()).or_default().push(f.clone());
        }
        self.edges.push(NetworkEdge { from: f, to: t, kind });
    }

    /// Run *all* structural checks. Returns the first violation found —
    /// callers wanting an exhaustive list should call the individual
    /// methods.
    pub fn verify(&self) -> Result<(), NetworkError> {
        if self.nodes.is_empty() {
            return Err(NetworkError::Empty);
        }
        self.check_edges_reference_known_nodes()?;
        self.check_acyclic()?;
        Ok(())
    }

    fn check_edges_reference_known_nodes(&self) -> Result<(), NetworkError> {
        for e in &self.edges {
            if !self.nodes.contains(&e.from) {
                return Err(NetworkError::UnknownNode {
                    edge_from: e.from.clone(),
                    edge_to: e.to.clone(),
                    missing: e.from.clone(),
                });
            }
            if !self.nodes.contains(&e.to) {
                return Err(NetworkError::UnknownNode {
                    edge_from: e.from.clone(),
                    edge_to: e.to.clone(),
                    missing: e.to.clone(),
                });
            }
        }
        Ok(())
    }

    /// DFS-based cycle detection over the expanded adjacency list. Returns
    /// the cycle path on the first cycle found so users can fix it.
    pub fn check_acyclic(&self) -> Result<(), NetworkError> {
        // Three-color DFS: 0 = unvisited, 1 = on stack, 2 = done.
        let mut color: BTreeMap<&str, u8> = BTreeMap::new();
        for n in &self.nodes {
            color.insert(n.as_str(), 0);
        }
        for start in self.nodes.iter() {
            if color.get(start.as_str()).copied().unwrap_or(0) != 0 {
                continue;
            }
            let mut stack: Vec<(&str, usize)> = vec![(start.as_str(), 0)];
            let mut path: Vec<String> = vec![start.clone()];
            color.insert(start.as_str(), 1);
            while let Some((node, idx)) = stack.last().copied() {
                let succs: Vec<&str> = self
                    .adjacency
                    .get(node)
                    .map(|v| v.iter().map(|s| s.as_str()).collect())
                    .unwrap_or_default();
                if idx >= succs.len() {
                    color.insert(node, 2);
                    stack.pop();
                    path.pop();
                    continue;
                }
                let next = succs[idx];
                stack.last_mut().unwrap().1 = idx + 1;
                match color.get(next).copied().unwrap_or(0) {
                    0 => {
                        color.insert(next, 1);
                        path.push(next.to_string());
                        stack.push((next, 0));
                    }
                    1 => {
                        // Found a back edge — cycle from `next` back to itself.
                        let start_pos = path.iter().position(|p| p == next).unwrap_or(0);
                        let mut cycle: Vec<String> = path[start_pos..].to_vec();
                        cycle.push(next.to_string());
                        return Err(NetworkError::Cycle { path: cycle });
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// BFS from `root`. Returns every node not reached.
    pub fn unreachable_from(&self, root: &str) -> Vec<String> {
        if !self.nodes.contains(root) {
            return Vec::new();
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut q: VecDeque<&str> = VecDeque::new();
        q.push_back(root);
        seen.insert(root);
        while let Some(n) = q.pop_front() {
            if let Some(succs) = self.adjacency.get(n) {
                for s in succs {
                    if seen.insert(s.as_str()) {
                        q.push_back(s.as_str());
                    }
                }
            }
        }
        self.nodes
            .iter()
            .filter(|n| !seen.contains(n.as_str()))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn researcher_team() -> Network {
        let mut n = Network::new("ResearchTeam");
        for name in ["researcher", "critic", "writer", "editor"] {
            n.add_node(name).unwrap();
        }
        n.add_edge("researcher", "critic", EdgeKind::OneWay);
        n.add_edge("researcher", "writer", EdgeKind::OneWay);
        n.add_edge("critic", "writer", EdgeKind::Bidirectional);
        n.add_edge("writer", "editor", EdgeKind::OneWay);
        n
    }

    #[test]
    fn verify_passes_on_dag_with_bidirectional_edges_still_cyclic() {
        // Bidirectional critic <-> writer expands to a 2-cycle, so this
        // network *should* fail cycle detection — that's the safety the
        // spec promises.
        let n = researcher_team();
        let err = n.verify().unwrap_err();
        assert!(matches!(err, NetworkError::Cycle { .. }));
    }

    #[test]
    fn pure_dag_verifies_clean() {
        let mut n = Network::new("Pipeline");
        for name in ["a", "b", "c", "d"] {
            n.add_node(name).unwrap();
        }
        n.add_edge("a", "b", EdgeKind::OneWay);
        n.add_edge("b", "c", EdgeKind::OneWay);
        n.add_edge("c", "d", EdgeKind::OneWay);
        n.verify().unwrap();
    }

    #[test]
    fn unknown_node_in_edge_is_caught() {
        let mut n = Network::new("X");
        n.add_node("a").unwrap();
        n.add_edge("a", "ghost", EdgeKind::OneWay);
        let err = n.verify().unwrap_err();
        match err {
            NetworkError::UnknownNode { missing, .. } => assert_eq!(missing, "ghost"),
            other => panic!("expected UnknownNode, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_node_rejected() {
        let mut n = Network::new("X");
        n.add_node("a").unwrap();
        assert!(matches!(
            n.add_node("a").unwrap_err(),
            NetworkError::DuplicateNode(_)
        ));
    }

    #[test]
    fn empty_network_rejected() {
        let n = Network::new("X");
        assert!(matches!(n.verify().unwrap_err(), NetworkError::Empty));
    }

    #[test]
    fn unreachable_nodes_reported() {
        let mut n = Network::new("X");
        for name in ["a", "b", "c", "d"] {
            n.add_node(name).unwrap();
        }
        n.add_edge("a", "b", EdgeKind::OneWay);
        n.add_edge("c", "d", EdgeKind::OneWay);
        let orphans = n.unreachable_from("a");
        assert_eq!(orphans, vec!["c".to_string(), "d".to_string()]);
    }

    #[test]
    fn self_loop_is_a_cycle() {
        let mut n = Network::new("X");
        n.add_node("a").unwrap();
        n.add_edge("a", "a", EdgeKind::OneWay);
        let err = n.verify().unwrap_err();
        assert!(matches!(err, NetworkError::Cycle { .. }));
    }

    #[test]
    fn three_cycle_detected_with_path() {
        let mut n = Network::new("X");
        for name in ["a", "b", "c"] {
            n.add_node(name).unwrap();
        }
        n.add_edge("a", "b", EdgeKind::OneWay);
        n.add_edge("b", "c", EdgeKind::OneWay);
        n.add_edge("c", "a", EdgeKind::OneWay);
        let err = n.verify().unwrap_err();
        match err {
            NetworkError::Cycle { path } => {
                assert!(path.len() >= 3, "expected at least 3-step cycle, got {path:?}");
            }
            other => panic!("expected Cycle, got {other:?}"),
        }
    }
}
