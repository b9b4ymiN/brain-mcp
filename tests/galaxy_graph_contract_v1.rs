//! Task 5.2 — Galaxy 3D graph contract (RED stage).
//!
//! Encodes GOAL-vNext §13 Task 5.2 DoD at the contract level (Rust data
//! model; no real Three.js/react-force-graph in-env). The server sends
//! bounded subgraphs; the renderer is a separate deployment concern.

use llm_wiki::galaxy::{
    EdgeKind, GalaxyEdge, GalaxyGraph, GalaxyNode, GraphLod, GraphRendererKind, ZoomLevel,
};
use serde_json::json;

fn node(id: &str, kind: &str) -> GalaxyNode {
    GalaxyNode {
        id: id.to_owned(),
        label: id.to_owned(),
        kind: kind.to_owned(),
        domain: "stocks".to_owned(),
    }
}

fn edge(src: &str, dst: &str, kind: EdgeKind) -> GalaxyEdge {
    GalaxyEdge {
        source: src.to_owned(),
        target: dst.to_owned(),
        kind,
    }
}

// =============================================================================
// DoD: bounded subgraph — server never sends the whole brain
// =============================================================================

/// `GalaxyGraph` is bounded: it carries at most `max_nodes` nodes. The server
/// sends an ego-neighborhood or a LOD-capped cluster, never the entire graph.
/// §9.2 "server ส่ง bounded subgraph/ego network ไม่โหลดทั้งสมองพร้อมกัน".
#[test]
fn galaxy_graph_is_bounded() {
    let graph = GalaxyGraph::new(300);
    assert_eq!(graph.max_nodes(), 300);
    // Adding 310 nodes keeps only the first 300 (bounded).
    let mut g = graph;
    for i in 0..310 {
        g.add_node(node(&format!("n{i}"), "entity"));
    }
    assert_eq!(g.node_count(), 300, "graph must cap at max_nodes");
}

// =============================================================================
// DoD: LOD (level of detail) — semantic zoom
// =============================================================================

/// `ZoomLevel` drives the LOD: far = community supernodes (≤300), mid =
/// visible nodes (≤2000), close = ego-neighborhood with provenance.
/// §9.2.
#[test]
fn zoom_level_drives_lod() {
    assert_eq!(
        ZoomLevel::Far.lod(),
        GraphLod::CommunitySupernodes,
        "far zoom = community supernodes ≤300"
    );
    assert_eq!(
        ZoomLevel::Mid.lod(),
        GraphLod::VisibleNodes,
        "mid zoom = visible nodes ≤2000"
    );
    assert_eq!(
        ZoomLevel::Close.lod(),
        GraphLod::EgoNeighborhood,
        "close zoom = ego neighborhood with provenance"
    );
}

/// Each LOD level has a node cap matching §9.2's thresholds.
#[test]
fn lod_node_caps_match_spec() {
    assert_eq!(GraphLod::CommunitySupernodes.node_cap(), 300);
    assert_eq!(GraphLod::VisibleNodes.node_cap(), 2000);
    assert!(GraphLod::EgoNeighborhood.node_cap() <= 2000);
}

// =============================================================================
// DoD: renderer fallback (no-WebGL, reduced-motion, 2D/list)
// =============================================================================

/// The graph schema is separate from the renderer. Multiple renderers can
/// consume the same `GalaxyGraph`. §9.2 "graph schema แยกจาก renderer".
#[test]
fn renderer_kinds_are_swappable() {
    let _ = GraphRendererKind::ForceGraph3d;
    let _ = GraphRendererKind::ForceGraph2d;
    let _ = GraphRendererKind::List;
    // No WebGL → 2D or list fallback. Compile-time proof the variants exist.
}

/// A no-WebGL client falls back to the 2D or list renderer. The graph data is
/// the same; only the rendering changes.
#[test]
fn no_webgl_falls_back_to_2d_or_list() {
    let renderers = GraphRendererKind::fallback_chain();
    assert!(
        renderers.contains(&GraphRendererKind::ForceGraph2d)
            || renderers.contains(&GraphRendererKind::List),
        "fallback chain must include 2D or list"
    );
}

// =============================================================================
// DoD: cluster counts/aggregated edges match raw fixture 100%
// =============================================================================

/// The aggregated edge count between two community supernodes equals the sum
/// of raw edges between their member nodes. §9.2 "cluster counts/aggregated
/// edges ตรงกับ raw fixture 100%".
#[test]
fn aggregated_edge_count_matches_raw() {
    let mut g = GalaxyGraph::new(100);
    g.add_node(node("a1", "entity"));
    g.add_node(node("a2", "entity"));
    g.add_node(node("b1", "entity"));
    g.add_edge(edge("a1", "b1", EdgeKind::Related));
    g.add_edge(edge("a2", "b1", EdgeKind::Related));
    // Two raw edges a*→b1. The aggregated count between community A and B is 2.
    let count = g.edge_count_between_sets(&["a1", "a2"], &["b1"]);
    assert_eq!(count, 2, "aggregated edge count must match raw");
}

// =============================================================================
// DoD: color is not the only signal
// =============================================================================

/// A `GalaxyNode` carries `kind` (shape/category) + `label` so color is not
/// the sole signal. §9.2 "สีไม่ใช่สัญญาณเดียว; ใช้ shape/label/status".
#[test]
fn node_has_shape_and_label_not_just_color() {
    let n = node("GULF", "entity");
    assert!(!n.label.is_empty());
    assert!(!n.kind.is_empty());
}

// keep json alive
#[test]
fn _json_check() {
    let _ = json!({"ok": true});
}
