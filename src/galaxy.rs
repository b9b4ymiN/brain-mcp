//! Galaxy 3D graph contract (Task 5.2).
//!
//! Data model for GOAL-vNext §13 Task 5.2 + §9.2 Galaxy 3D principles. The
//! server sends BOUNDED subgraphs (ego networks / LOD-capped clusters); the
//! renderer (Three.js / react-force-graph-3d / 2d / list) is a separate
//! deployment concern. This module is the schema the server serializes and
//! the client consumes.
//!
//! Key invariants (§9.2):
//! - The graph is bounded: never the whole brain at once.
//! - LOD (semantic zoom): far → community supernodes (≤300); mid → visible
//!   nodes (≤2000); close → ego neighborhood with provenance.
//! - Color is not the only signal: nodes carry `kind` (shape) + `label`.
//! - The schema is renderer-agnostic: a no-WebGL client falls back to 2D/list.

use serde::{Deserialize, Serialize};

// ── Nodes + edges ────────────────────────────────────────────────────────────

/// A graph node. Carries `kind` (shape/category) + `label` so color is not
/// the sole signal (§9.2). Domain groups nodes into galaxies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GalaxyNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub domain: String,
}

/// The kind of relationship an edge represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Related,
    Sources,
    Supersedes,
    Retracts,
}

/// A graph edge between two nodes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GalaxyEdge {
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
}

// ── Bounded graph ────────────────────────────────────────────────────────────

/// A bounded subgraph the server sends to the client. Caps at `max_nodes`;
/// adding beyond the cap drops the overflow (the server is responsible for
/// selecting the most relevant nodes — ego neighborhood or LOD cluster).
/// §9.2 "server ส่ง bounded subgraph ไม่โหลดทั้งสมองพร้อมกัน".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GalaxyGraph {
    max_nodes: usize,
    nodes: Vec<GalaxyNode>,
    edges: Vec<GalaxyEdge>,
}

impl GalaxyGraph {
    /// Create an empty graph with the given node cap.
    pub fn new(max_nodes: usize) -> Self {
        Self {
            max_nodes,
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// The node cap.
    pub fn max_nodes(&self) -> usize {
        self.max_nodes
    }

    /// Current node count.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Add a node, respecting the cap. Overflow nodes are silently dropped
    /// (the server pre-selects; this is a safety bound).
    pub fn add_node(&mut self, node: GalaxyNode) {
        if self.nodes.len() < self.max_nodes {
            self.nodes.push(node);
        }
    }

    /// Add an edge.
    pub fn add_edge(&mut self, edge: GalaxyEdge) {
        self.edges.push(edge);
    }

    /// Count edges where source ∈ `left_set` and target ∈ `right_set`.
    /// Used to verify aggregated edge counts match raw fixtures (§9.2: 100%).
    pub fn edge_count_between_sets(&self, left_set: &[&str], right_set: &[&str]) -> usize {
        self.edges
            .iter()
            .filter(|e| {
                left_set.contains(&e.source.as_str()) && right_set.contains(&e.target.as_str())
            })
            .count()
    }
}

// ── LOD (semantic zoom) ──────────────────────────────────────────────────────

/// Level of detail driven by zoom. §9.2 thresholds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphLod {
    /// Far zoom: community supernodes, ≤300.
    CommunitySupernodes,
    /// Mid zoom: visible nodes, ≤2000.
    VisibleNodes,
    /// Close zoom: ego neighborhood with provenance.
    EgoNeighborhood,
}

impl GraphLod {
    /// The node cap for this LOD level.
    pub fn node_cap(&self) -> usize {
        match self {
            GraphLod::CommunitySupernodes => 300,
            GraphLod::VisibleNodes => 2000,
            GraphLod::EgoNeighborhood => 2000, // ego is bounded by neighborhood depth, not a flat cap
        }
    }
}

/// The zoom level the client reports; drives LOD selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoomLevel {
    Far,
    Mid,
    Close,
}

impl ZoomLevel {
    /// Map a zoom level to its LOD.
    pub fn lod(&self) -> GraphLod {
        match self {
            ZoomLevel::Far => GraphLod::CommunitySupernodes,
            ZoomLevel::Mid => GraphLod::VisibleNodes,
            ZoomLevel::Close => GraphLod::EgoNeighborhood,
        }
    }
}

// ── Renderer fallback chain ──────────────────────────────────────────────────

/// The renderer kind. §9.2: the schema is separate from the renderer; a
/// no-WebGL client falls back to 2D or list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphRendererKind {
    /// Three.js / react-force-graph-3d (requires WebGL).
    ForceGraph3d,
    /// 2D fallback (react-force-graph-2d).
    ForceGraph2d,
    /// List/timeline fallback (no canvas).
    List,
}

impl GraphRendererKind {
    /// The fallback chain a client tries when the preferred renderer is
    /// unavailable (e.g. no WebGL). §9.2 "no-WebGL, reduced-motion, keyboard
    /// และ list/2D fallback".
    pub fn fallback_chain() -> Vec<GraphRendererKind> {
        vec![
            GraphRendererKind::ForceGraph3d,
            GraphRendererKind::ForceGraph2d,
            GraphRendererKind::List,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_caps_nodes() {
        let mut g = GalaxyGraph::new(2);
        g.add_node(GalaxyNode {
            id: "a".into(),
            label: "a".into(),
            kind: "entity".into(),
            domain: "d".into(),
        });
        g.add_node(GalaxyNode {
            id: "b".into(),
            label: "b".into(),
            kind: "entity".into(),
            domain: "d".into(),
        });
        g.add_node(GalaxyNode {
            id: "c".into(),
            label: "c".into(),
            kind: "entity".into(),
            domain: "d".into(),
        }); // overflow
        assert_eq!(g.node_count(), 2);
    }

    #[test]
    fn lod_caps() {
        assert_eq!(GraphLod::CommunitySupernodes.node_cap(), 300);
    }
}
