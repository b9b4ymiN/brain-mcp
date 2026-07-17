//! Galaxy 3D graph contract (Task 5.2) + materializer (Task E2.1).
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
//!
//! Task E2.1 adds the materializer: [`GalaxyGraph::from_claims`] derives a
//! bounded subgraph from a flat `[ClaimView]` snapshot, and
//! [`GalaxyGraph::ego_around`] builds a depth-bounded ego network around one
//! focus entity. The serializable wire shape is [`GalaxyPayload`] (produced by
//! [`GalaxyGraph::to_payload`]); the Console consumes it at
//! `GET /api/v1/galaxy`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::semantic::ClaimView;

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

    /// Read-only view of the materialized nodes (Task E2.1: lets the API
    /// serialize the graph without re-deriving it).
    pub fn nodes(&self) -> &[GalaxyNode] {
        &self.nodes
    }

    /// Mutable view of the materialized nodes. The API uses this to re-stamp
    /// node labels with canonical subjects after the ego builder has selected
    /// the surviving set.
    pub fn nodes_mut(&mut self) -> &mut [GalaxyNode] {
        &mut self.nodes
    }

    /// Read-only view of the materialized edges.
    pub fn edges(&self) -> &[GalaxyEdge] {
        &self.edges
    }

    /// Current edge count.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
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

// ── Wire payload + materializer (Task E2.1) ─────────────────────────────────

/// Serializable wire shape the Console consumes at `GET /api/v1/galaxy`.
///
/// `nodes`/`edges` are owned snapshots so the JSON body is self-contained;
/// `lod` echoes the level of detail the server picked so a client can choose
/// its renderer/labels accordingly (§9.2 semantic-zoom). `max_nodes` is the
/// enforced cap; `node_count` is the realized count (≤ `max_nodes`).
#[derive(Clone, Debug, Serialize)]
pub struct GalaxyPayload {
    pub lod: GraphLod,
    pub max_nodes: usize,
    pub node_count: usize,
    pub nodes: Vec<GalaxyNode>,
    pub edges: Vec<GalaxyEdge>,
}

impl GalaxyGraph {
    /// Render this graph to the wire payload at the given LOD. The LOD is
    /// stamped into the payload but does NOT re-trim the graph (the server is
    /// expected to have built the graph at the right cap via [`Self::new`] or
    /// [`Self::from_claims`]).
    pub fn to_payload(&self, lod: GraphLod) -> GalaxyPayload {
        GalaxyPayload {
            lod,
            max_nodes: self.max_nodes,
            node_count: self.nodes.len(),
            nodes: self.nodes.clone(),
            edges: self.edges.clone(),
        }
    }

    /// Materialize a bounded subgraph from a flat `[ClaimView]` snapshot
    /// (Task E2.1).
    ///
    /// # Node selection (deterministic, first-seen)
    /// - Claims with `entity_id = None` are skipped (they pre-date the entity
    ///   table and can't be graphed — none exist in production).
    /// - If `domain_filter` is provided, only claims with a matching `domain`
    ///   contribute.
    /// - One node per distinct `entity_id`, in first-seen order. The node
    ///   `label` is the most frequent `subject` for that entity (ties broken
    ///   by first-seen); `kind` is the most frequent `claim_kind`; `domain`
    ///   is the most frequent `domain`. These "most frequent" aggregations are
    ///   stable given the input order — the goal is a sensible label, not a
    ///   semantic resolution.
    /// - `lod_cap` caps the distinct-entity count. Overflow entities (and
    ///   their edges) are dropped; first-seen ordering is the selection
    ///   heuristic for E2.1 (relevance-ranking is deferred to a later task).
    ///
    /// # Edge inference (honest about the data model)
    /// `ClaimView` is flat: it carries `subject` + `predicate` + `value` +
    /// `entity_id` + `status`, but NO object-entity pointer and NO
    /// superseding-partner linkage (supersede replaces a claim on the SAME
    /// subject/entity — so it is intra-entity, useless as a cross-entity
    /// edge). We therefore derive cross-entity edges from two deterministic,
    /// defensible signals rather than fabricating them:
    ///
    /// 1. **Value-string reference.** When a claim's `value` (read as a string
    ///    when it is a JSON string) exactly matches another entity's
    ///    canonical `subject` label, emit a [`EdgeKind::Related`] edge
    ///    `subject_entity → referenced_entity`. This is the most meaningful
    ///    signal available: it catches `parent_of`, `same_as`, `rival_of`,
    ///    and `targets` style references whose value carries an entity name.
    /// 2. **Same-subject co-occurrence across domains.** Two distinct
    ///    entities (in different domains) whose claims share the exact same
    ///    `subject` string are linked with [`EdgeKind::Related`]. This is rare
    ///    by construction (entity aliases enforce 1 subject per domain), but
    ///    legitimate after cross-domain merges.
    ///
    /// Edges are de-duplicated and only emitted when BOTH endpoints land in
    /// the cap-bounded node set (so the parity invariant — payload edge count
    /// equals the predicted raw count — holds for whatever edges ARE emitted).
    ///
    /// Richer edge inference (explicit cross-entity references, typed
    /// relationships, supersede/retract partners) is deferred to a future
    /// task that either extends `ClaimView` with an object-entity pointer or
    /// adds a graph-specific store method that reads the link table directly.
    ///
    /// # Status-based edge inference (deferred)
    /// This signature takes no `now: DateTime<Utc>` parameter. Status-aware
    /// edge inference (e.g. treating superseded/retracted/expired claims
    /// differently when materializing a strictly-"current brain" view) is
    /// deferred. When that lands, a `now` parameter will be added back to
    /// this signature, the handler, and the tests.
    pub fn from_claims(
        claims: &[ClaimView],
        lod_cap: usize,
        domain_filter: Option<&str>,
    ) -> GalaxyGraph {
        Self::from_claims_with_subjects(claims, lod_cap, domain_filter, &HashMap::new())
    }

    /// Same as [`Self::from_claims`] but accepts a pre-built `entity_id →
    /// canonical subject` map. When non-empty, this map is preferred as the
    /// node label source AND the value-reference match target (canonical
    /// subject is a live view; the per-claim `subject` is the historical
    /// payload). Exposed publicly so the API can pass the store's
    /// `entity_canonical_subjects_owned()` snapshot in one call.
    pub fn from_claims_with_subjects(
        claims: &[ClaimView],
        lod_cap: usize,
        domain_filter: Option<&str>,
        canonical_subjects: &HashMap<Uuid, String>,
    ) -> GalaxyGraph {
        // Stage 1: filter + group claims by entity_id (first-seen order).
        let mut entity_order: Vec<Uuid> = Vec::new();
        let mut by_entity: HashMap<Uuid, Vec<&ClaimView>> = HashMap::new();
        for claim in claims {
            // Only graph claims that resolve to an entity. Skip `future`/`past`
            // rows only when the caller passed `now`-bounded active claims;
            // we don't filter on status here so callers can pass `active`
            // (the common case) or a wider slice. Domain filter is honored.
            if let Some(filter) = domain_filter
                && claim.domain != filter
            {
                continue;
            }
            let Some(entity_id) = claim.entity_id else {
                continue;
            };
            if !by_entity.contains_key(&entity_id) {
                entity_order.push(entity_id);
            }
            by_entity.entry(entity_id).or_default().push(claim);
        }

        // Stage 2: respect the LOD cap (deterministic first-seen selection).
        let cap = lod_cap.max(1);
        if entity_order.len() > cap {
            entity_order.truncate(cap);
        }
        let allowed: HashMap<Uuid, ()> = entity_order.iter().map(|id| (*id, ())).collect();

        // Stage 3: build nodes (label = most frequent subject, preferring the
        // canonical_subject map when provided; kind = most frequent claim_kind;
        // domain = most frequent domain).
        let mut graph = GalaxyGraph::new(cap);
        for entity_id in &entity_order {
            let entity_claims = by_entity.get(entity_id).expect("grouped above");
            let label = canonical_subjects
                .get(entity_id)
                .cloned()
                .or_else(|| most_frequent(entity_claims.iter().map(|c| c.subject.as_str())))
                .unwrap_or_else(|| entity_id.to_string());
            let kind = most_frequent(entity_claims.iter().map(|c| c.claim_kind.as_str()))
                .unwrap_or_else(|| "entity".to_string());
            let domain =
                most_frequent(entity_claims.iter().map(|c| c.domain.as_str())).unwrap_or_default();
            graph.add_node(GalaxyNode {
                id: entity_id.to_string(),
                label,
                kind,
                domain,
            });
        }

        // Stage 4: derive edges. Build a `subject_label → entity_id` index so
        // a value-string that names another entity resolves to its node id.
        let mut label_to_entity: HashMap<&str, &Uuid> = HashMap::new();
        for entity_id in &entity_order {
            let label = canonical_subjects
                .get(entity_id)
                .map(String::as_str)
                .or_else(|| {
                    by_entity
                        .get(entity_id)
                        .and_then(|cs| cs.first())
                        .map(|c| c.subject.as_str())
                });
            if let Some(label) = label {
                label_to_entity.insert(label, entity_id);
            }
        }

        let mut seen_edges: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        for (entity_id, entity_claims) in &by_entity {
            // Skip edges whose source fell outside the cap.
            if !allowed.contains_key(entity_id) {
                continue;
            }
            for claim in entity_claims {
                // (1) value-string reference: value is a JSON string naming
                // another entity's subject/canonical label.
                if let Some(s) = claim.value.as_str()
                    && let Some(target) = label_to_entity.get(s).copied()
                    && target != entity_id
                    && allowed.contains_key(target)
                {
                    push_dedup_edge(
                        &mut graph,
                        &mut seen_edges,
                        entity_id.to_string(),
                        target.to_string(),
                        EdgeKind::Related,
                    );
                }
            }
        }

        // (2) same-subject co-occurrence across distinct entities.
        let mut subject_to_entities: HashMap<&str, Vec<Uuid>> = HashMap::new();
        for (entity_id, entity_claims) in &by_entity {
            if !allowed.contains_key(entity_id) {
                continue;
            }
            for claim in entity_claims {
                subject_to_entities
                    .entry(claim.subject.as_str())
                    .or_default()
                    .push(*entity_id);
            }
        }
        for entities in subject_to_entities.values() {
            // dedup + sort so edge emission is deterministic regardless of
            // input order.
            let mut unique: Vec<Uuid> = entities.to_vec();
            unique.sort();
            unique.dedup();
            for i in 0..unique.len() {
                for j in (i + 1)..unique.len() {
                    if unique[i] == unique[j] {
                        continue;
                    }
                    if !allowed.contains_key(&unique[i]) || !allowed.contains_key(&unique[j]) {
                        continue;
                    }
                    push_dedup_edge(
                        &mut graph,
                        &mut seen_edges,
                        unique[i].to_string(),
                        unique[j].to_string(),
                        EdgeKind::Related,
                    );
                }
            }
        }

        graph
    }

    /// Build a bounded ego network around `focus_entity_id` (Task E2.1).
    ///
    /// Starts with the focus entity and expands outward by following the same
    /// subject-co-occurrence + value-reference heuristic used by
    /// [`Self::from_claims`]. `depth` is the hop count; it is hard-capped at
    /// [`EGO_MAX_DEPTH`] (2) for safety so a pathological dataset can't blow
    /// up the graph (§9.2: server sends bounded subgraphs).
    ///
    /// Returns an empty graph (cap = [`GraphLod::EgoNeighborhood`] but 0 nodes)
    /// if the focus entity has no claims in the slice.
    pub fn ego_around(claims: &[ClaimView], focus_entity_id: Uuid, depth: usize) -> GalaxyGraph {
        let cap = GraphLod::EgoNeighborhood.node_cap();
        let bounded_depth = depth.min(EGO_MAX_DEPTH);
        if bounded_depth == 0 {
            return Self::ego_singleton(claims, focus_entity_id);
        }

        // Resolve neighbors hop by hop.
        let mut visited: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
        visited.insert(focus_entity_id);
        let mut frontier: Vec<Uuid> = vec![focus_entity_id];
        for _ in 0..bounded_depth {
            let mut next_frontier: Vec<Uuid> = Vec::new();
            for source in &frontier {
                for neighbor in ego_neighbors(claims, *source) {
                    if visited.insert(neighbor) {
                        next_frontier.push(neighbor);
                    }
                }
            }
            frontier = next_frontier;
            if frontier.is_empty() {
                break;
            }
        }

        // Reuse `from_claims`-shaped logic, but constrain the allowed set to
        // the ego BFS result. We do this by filtering the claim slice to the
        // visited entities and then building the graph (the cap still applies
        // as a safety bound).
        let allowed: std::collections::HashSet<Uuid> = visited;
        let focused: Vec<ClaimView> = claims
            .iter()
            .filter(|c| c.entity_id.is_some_and(|id| allowed.contains(&id)))
            .cloned()
            .collect();
        if focused.is_empty() {
            // Empty ego graph: report the LOD cap honestly but with 0 nodes.
            return GalaxyGraph::new(cap);
        }
        // Build with the focused slice; pass empty canonical map (caller can
        // re-label later if needed). The cap keeps the payload bounded.
        Self::from_claims(&focused, cap, None)
    }

    /// Single-entity (depth 0) ego graph: just the focus node + its claims'
    /// aggregated label. Used when `depth == 0`.
    fn ego_singleton(claims: &[ClaimView], focus_entity_id: Uuid) -> GalaxyGraph {
        let cap = GraphLod::EgoNeighborhood.node_cap();
        let focused: Vec<ClaimView> = claims
            .iter()
            .filter(|c| c.entity_id == Some(focus_entity_id))
            .cloned()
            .collect();
        if focused.is_empty() {
            return GalaxyGraph::new(cap);
        }
        Self::from_claims(&focused, cap, None)
    }
}

/// Maximum hop count for [`GalaxyGraph::ego_around`]. Caps traversal so a
/// densely-connected dataset can't produce an unbounded graph (§9.2).
pub const EGO_MAX_DEPTH: usize = 2;

/// Pick the most frequent string in the iterator (ties broken by first-seen).
///
/// # Determinism
/// `HashMap` iteration order is randomized by Rust's `RandomState`, and
/// `Iterator::max_by_key` returns the LAST maximal element. A naive
/// `counts.into_iter().max_by_key(|(_, n)| *n)` would therefore make the
/// tie-breaker non-deterministic — producing different `label`/`kind`/`domain`
/// values run-to-run for entities whose counts are tied (frontend flicker,
/// flaky tests). To stay deterministic we track each string's first-seen index
/// alongside its count and break ties on first-seen (smaller index wins),
/// matching the `from_claims` doc contract ("ties broken by first-seen").
fn most_frequent<'a>(iter: impl Iterator<Item = &'a str>) -> Option<String> {
    // (count, first_seen_index)
    let mut counts: HashMap<&str, (usize, usize)> = HashMap::new();
    for (idx, s) in iter.enumerate() {
        let entry = counts.entry(s).or_insert((0, idx));
        entry.0 += 1;
    }
    // Highest count wins; on a tie, the SMALLER first-seen index wins
    // (first-seen). `max_by` with a count-descending-then-index-ascending
    // comparator yields exactly that.
    counts
        .into_iter()
        .max_by(|a, b| a.1.0.cmp(&b.1.0).then_with(|| b.1.1.cmp(&a.1.1)))
        .map(|(s, _)| s.to_owned())
}

/// Add an edge to `graph` only if `(source, target)` hasn't been emitted yet.
fn push_dedup_edge(
    graph: &mut GalaxyGraph,
    seen: &mut std::collections::HashSet<(String, String)>,
    source: String,
    target: String,
    kind: EdgeKind,
) {
    if source == target {
        return;
    }
    if !seen.insert((source.clone(), target.clone())) {
        return;
    }
    graph.add_edge(GalaxyEdge {
        source,
        target,
        kind,
    });
}

/// Resolve the immediate neighbors of `entity_id` under the E2.1 heuristic
/// (value-string references + same-subject co-occurrence). Used by
/// [`GalaxyGraph::ego_around`] for BFS expansion.
fn ego_neighbors(claims: &[ClaimView], entity_id: Uuid) -> Vec<Uuid> {
    // Build subject-label → entity_id index across all claims (the entity a
    // value-string refers to is any entity whose canonical or first-claim
    // subject equals the value).
    let mut subject_to_entities: HashMap<&str, Vec<Uuid>> = HashMap::new();
    let mut entity_first_subject: HashMap<Uuid, &str> = HashMap::new();
    for c in claims {
        let Some(id) = c.entity_id else { continue };
        entity_first_subject
            .entry(id)
            .or_insert_with(|| c.subject.as_str());
        subject_to_entities
            .entry(c.subject.as_str())
            .or_default()
            .push(id);
    }

    let mut out: Vec<Uuid> = Vec::new();
    let mut seen: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    seen.insert(entity_id);

    // Outgoing value-references from this entity's claims.
    for c in claims.iter().filter(|c| c.entity_id == Some(entity_id)) {
        if let Some(value) = c.value.as_str()
            && let Some(targets) = subject_to_entities.get(value)
        {
            for t in targets {
                if seen.insert(*t) {
                    out.push(*t);
                }
            }
        }
    }

    // Incoming value-references: other entities whose value names this entity.
    let my_subject = entity_first_subject.get(&entity_id).copied();
    if let Some(my_subject) = my_subject {
        for c in claims {
            if c.entity_id == Some(entity_id) {
                continue;
            }
            if c.value.as_str() == Some(my_subject)
                && let Some(id) = c.entity_id
                && seen.insert(id)
            {
                out.push(id);
            }
        }
    }

    // Same-subject co-occurrence.
    if let Some(my_subject) = my_subject
        && let Some(entities) = subject_to_entities.get(my_subject)
    {
        for t in entities {
            if seen.insert(*t) {
                out.push(*t);
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::{OriginClass, PrivacyLabel};
    use serde_json::json;

    /// Minimal `ClaimView` fixture for unit testing. Only the fields the
    /// materializer reads are parameterized; the rest are stable defaults.
    fn claim(
        entity_id: Uuid,
        subject: &str,
        predicate: &str,
        value: serde_json::Value,
        domain: &str,
        claim_kind: &str,
    ) -> ClaimView {
        ClaimView {
            claim_id: Uuid::new_v4(),
            proposal_id: Uuid::new_v4(),
            subject: subject.to_owned(),
            predicate: predicate.to_owned(),
            value,
            claim_kind: claim_kind.to_owned(),
            status: "confirmed".to_owned(),
            domain: domain.to_owned(),
            confidence_basis_points: 8_000,
            privacy_label: PrivacyLabel::LocalOnly,
            valid_from: None,
            valid_to: None,
            confirmed_event_seq: 1,
            provenance_kind: "user_assertion".to_owned(),
            origin: OriginClass::HumanAuthored,
            entity_id: Some(entity_id),
        }
    }

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

    // ── accessors ────────────────────────────────────────────────────────────

    #[test]
    fn nodes_and_edges_accessors_return_slices() {
        let mut g = GalaxyGraph::new(10);
        g.add_node(GalaxyNode {
            id: "a".into(),
            label: "a".into(),
            kind: "entity".into(),
            domain: "d".into(),
        });
        g.add_edge(GalaxyEdge {
            source: "a".into(),
            target: "a".into(),
            kind: EdgeKind::Related,
        });
        assert_eq!(g.nodes().len(), 1);
        assert_eq!(g.edges().len(), 1);
        assert_eq!(g.edge_count(), 1);
    }

    // ── from_claims: nodes ───────────────────────────────────────────────────

    #[test]
    fn from_claims_materializes_one_node_per_distinct_entity() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let claims = vec![
            claim(
                a,
                "GULF",
                "target_price",
                json!(58),
                "stocks",
                "external_fact",
            ),
            claim(a, "GULF", "rating", json!("buy"), "stocks", "external_fact"),
            claim(
                b,
                "PTT",
                "target_price",
                json!(70),
                "stocks",
                "external_fact",
            ),
        ];
        let g = GalaxyGraph::from_claims(&claims, 300, None);
        assert_eq!(g.node_count(), 2, "two distinct entities → two nodes");
        let labels: Vec<&str> = g.nodes().iter().map(|n| n.label.as_str()).collect();
        assert!(labels.contains(&"GULF"));
        assert!(labels.contains(&"PTT"));
    }

    #[test]
    fn from_claims_skips_claims_without_entity_id() {
        let a = Uuid::new_v4();
        let mut unbound = claim(a, "GULF", "p", json!(1), "stocks", "external_fact");
        unbound.entity_id = None;
        let claims = vec![unbound];
        let g = GalaxyGraph::from_claims(&claims, 300, None);
        assert_eq!(g.node_count(), 0);
    }

    #[test]
    fn from_claims_respects_lod_cap_in_first_seen_order() {
        let mut claims = Vec::new();
        let mut expected_first = String::new();
        for i in 0..5 {
            let id = Uuid::new_v4();
            let subject = format!("E{i}");
            if i == 0 {
                expected_first = id.to_string();
            }
            claims.push(claim(
                id,
                &subject,
                "p",
                json!(i),
                "stocks",
                "external_fact",
            ));
        }
        // Cap at 2 — only the first two entities (by first-seen) survive.
        let g = GalaxyGraph::from_claims(&claims, 2, None);
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.nodes()[0].id, expected_first, "first-seen entity kept");
    }

    #[test]
    fn from_claims_respects_domain_filter() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let claims = vec![
            claim(a, "GULF", "p", json!(1), "stocks", "external_fact"),
            claim(b, "PTT", "p", json!(2), "crypto", "external_fact"),
        ];
        let g = GalaxyGraph::from_claims(&claims, 300, Some("stocks"));
        assert_eq!(g.node_count(), 1);
        assert_eq!(g.nodes()[0].label, "GULF");
        assert_eq!(g.nodes()[0].domain, "stocks");
    }

    #[test]
    fn from_claims_picks_most_frequent_label_and_kind() {
        let a = Uuid::new_v4();
        let claims = vec![
            claim(a, "GULF", "p", json!(1), "stocks", "external_fact"),
            claim(a, "GULF", "p", json!(2), "stocks", "external_fact"),
            claim(a, "GULF-OLD", "p", json!(3), "stocks", "inference"),
        ];
        let g = GalaxyGraph::from_claims(&claims, 300, None);
        // GULF appears twice → label. external_fact appears twice → kind.
        assert_eq!(g.nodes()[0].label, "GULF");
        assert_eq!(g.nodes()[0].kind, "external_fact");
    }

    // ── from_claims: edges (parity-relevant) ─────────────────────────────────

    #[test]
    fn from_claims_emits_value_reference_edge() {
        // GULF claim's value names PTT's subject → one Related edge.
        let gulf = Uuid::new_v4();
        let ptt = Uuid::new_v4();
        let claims = vec![
            claim(
                gulf,
                "GULF",
                "rival_of",
                json!("PTT"),
                "stocks",
                "external_fact",
            ),
            claim(
                ptt,
                "PTT",
                "target_price",
                json!(70),
                "stocks",
                "external_fact",
            ),
        ];
        let g = GalaxyGraph::from_claims(&claims, 300, None);
        assert_eq!(
            g.edge_count(),
            1,
            "exactly one edge: GULF → PTT (value reference)"
        );
        assert_eq!(g.edges()[0].source, gulf.to_string());
        assert_eq!(g.edges()[0].target, ptt.to_string());
        assert_eq!(g.edges()[0].kind, EdgeKind::Related);
    }

    #[test]
    fn from_claims_emits_same_subject_co_occurrence_edge() {
        // Two entities in different domains share a subject → one edge.
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let claims = vec![
            claim(a, "SHARED", "p", json!(1), "stocks", "external_fact"),
            claim(b, "SHARED", "p", json!(2), "crypto", "external_fact"),
        ];
        let g = GalaxyGraph::from_claims(&claims, 300, None);
        assert_eq!(g.edge_count(), 1);
    }

    #[test]
    fn from_claims_dedups_edges() {
        // Two value-references from GULF to PTT → still one edge.
        let gulf = Uuid::new_v4();
        let ptt = Uuid::new_v4();
        let claims = vec![
            claim(
                gulf,
                "GULF",
                "rival_of",
                json!("PTT"),
                "stocks",
                "external_fact",
            ),
            claim(
                gulf,
                "GULF",
                "parent_of",
                json!("PTT"),
                "stocks",
                "external_fact",
            ),
            claim(ptt, "PTT", "p", json!(1), "stocks", "external_fact"),
        ];
        let g = GalaxyGraph::from_claims(&claims, 300, None);
        assert_eq!(g.edge_count(), 1, "deduplicated");
    }

    #[test]
    fn from_claims_drops_edges_with_endpoint_outside_cap() {
        // GULF references PTT, but cap=1 keeps only GULF → no edges.
        let gulf = Uuid::new_v4();
        let ptt = Uuid::new_v4();
        let claims = vec![
            claim(
                gulf,
                "GULF",
                "rival_of",
                json!("PTT"),
                "stocks",
                "external_fact",
            ),
            claim(ptt, "PTT", "p", json!(1), "stocks", "external_fact"),
        ];
        let g = GalaxyGraph::from_claims(&claims, 1, None);
        assert_eq!(g.node_count(), 1);
        assert_eq!(g.edge_count(), 0, "PTT outside cap → edge dropped");
    }

    // ── parity: aggregated edge count = raw fixture count ───────────────────

    /// §9.2 DoD: "cluster counts / aggregated edges ตรงกับ raw fixture 100%".
    /// With 3 value-references in the fixture, the materialized graph must
    /// carry exactly 3 edges — no fabrication, no loss.
    #[test]
    fn from_claims_parity_edge_count_matches_raw_fixture() {
        let gulf = Uuid::new_v4();
        let ptt = Uuid::new_v4();
        let advanc = Uuid::new_v4();
        let claims = vec![
            // 1) GULF → PTT (value reference)
            claim(
                gulf,
                "GULF",
                "rival_of",
                json!("PTT"),
                "stocks",
                "external_fact",
            ),
            // 2) PTT → ADVANC (value reference)
            claim(
                ptt,
                "PTT",
                "owns_stake_in",
                json!("ADVANC"),
                "stocks",
                "external_fact",
            ),
            // 3) ADVANC → GULF (value reference, closes the triangle)
            claim(
                advanc,
                "ADVANC",
                "supplier_to",
                json!("GULF"),
                "stocks",
                "external_fact",
            ),
        ];
        let g = GalaxyGraph::from_claims(&claims, 300, None);
        assert_eq!(g.node_count(), 3);
        // The fixture has exactly 3 distinct value-reference edges; the
        // materializer must produce exactly those 3 (no fabrication, no loss).
        assert_eq!(g.edge_count(), 3);
        // Aggregated between {GULF, ADVANC} (sources) and {PTT, GULF} (targets).
        let count = g.edge_count_between_sets(
            &[gulf.to_string().as_str(), advanc.to_string().as_str()],
            &[ptt.to_string().as_str(), gulf.to_string().as_str()],
        );
        assert_eq!(count, 2, "aggregated count matches raw");
    }

    // ── ego_around ───────────────────────────────────────────────────────────

    #[test]
    fn ego_around_empty_when_focus_has_no_claims() {
        let claims: Vec<ClaimView> = vec![];
        let g = GalaxyGraph::ego_around(&claims, Uuid::new_v4(), 1);
        assert_eq!(g.node_count(), 0);
        assert_eq!(g.max_nodes(), GraphLod::EgoNeighborhood.node_cap());
    }

    #[test]
    fn ego_around_returns_focus_and_direct_neighbors() {
        let gulf = Uuid::new_v4();
        let ptt = Uuid::new_v4();
        let claims = vec![
            claim(
                gulf,
                "GULF",
                "rival_of",
                json!("PTT"),
                "stocks",
                "external_fact",
            ),
            claim(ptt, "PTT", "p", json!(1), "stocks", "external_fact"),
        ];
        let g = GalaxyGraph::ego_around(&claims, gulf, 1);
        assert!(
            g.node_count() >= 2,
            "focus + at least one neighbor: got {}",
            g.node_count()
        );
        let ids: Vec<&str> = g.nodes().iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&gulf.to_string().as_str()), "focus in nodes");
    }

    #[test]
    fn ego_around_caps_depth_at_two() {
        // Build a chain A → B → C → D via value references; depth cap = 2
        // means from A we reach B (hop 1) and C (hop 2), but not D.
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let d = Uuid::new_v4();
        let claims = vec![
            claim(a, "A", "r", json!("B"), "stocks", "external_fact"),
            claim(b, "B", "r", json!("C"), "stocks", "external_fact"),
            claim(c, "C", "r", json!("D"), "stocks", "external_fact"),
            claim(d, "D", "p", json!(1), "stocks", "external_fact"),
        ];
        // Requested depth 99 is clamped to EGO_MAX_DEPTH (2).
        let g = GalaxyGraph::ego_around(&claims, a, 99);
        let ids: Vec<&str> = g.nodes().iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&a.to_string().as_str()));
        assert!(ids.contains(&b.to_string().as_str()));
        assert!(ids.contains(&c.to_string().as_str()));
        assert!(
            !ids.contains(&d.to_string().as_str()),
            "depth-2 cap excludes hop 3"
        );
    }

    // ── to_payload ───────────────────────────────────────────────────────────

    #[test]
    fn to_payload_carries_lod_max_nodes_and_counts() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let claims = vec![
            claim(a, "A", "r", json!("B"), "stocks", "external_fact"),
            claim(b, "B", "p", json!(1), "stocks", "external_fact"),
        ];
        let g = GalaxyGraph::from_claims(&claims, 300, None);
        let payload = g.to_payload(GraphLod::CommunitySupernodes);
        assert_eq!(payload.lod, GraphLod::CommunitySupernodes);
        assert_eq!(payload.max_nodes, 300);
        assert_eq!(payload.node_count, 2);
        assert_eq!(payload.nodes.len(), 2);
        assert_eq!(payload.edges.len(), 1);
    }

    #[test]
    fn to_payload_serializes_to_json() {
        let a = Uuid::new_v4();
        let claims = vec![claim(a, "A", "p", json!(1), "stocks", "external_fact")];
        let g = GalaxyGraph::from_claims(&claims, 300, None);
        let payload = g.to_payload(GraphLod::VisibleNodes);
        let json = serde_json::to_value(&payload).expect("serializes");
        assert_eq!(json["lod"], "visible_nodes");
        assert_eq!(json["node_count"], 1);
        assert_eq!(json["nodes"][0]["label"], "A");
    }

    // ── most_frequent: determinism (regression) ──────────────────────────────

    /// Direct unit test of the `most_frequent` helper: with equal counts for
    /// "B" and "A" and "B" seen first, "B" must win — deterministically,
    /// regardless of HashMap iteration order. Before the fix this returned
    /// whichever key HashMap happened to yield last, so the result flipped
    /// run-to-run (frontend flicker, flaky tests).
    #[test]
    fn most_frequent_breaks_ties_by_first_seen_deterministically() {
        // Tied counts; "B" appears first.
        assert_eq!(
            most_frequent(["B", "A", "B", "A"].into_iter()),
            Some("B".to_owned()),
            "first-seen (B) must win the tie"
        );
        // Tied counts; "A" appears first.
        assert_eq!(
            most_frequent(["A", "B", "A", "B"].into_iter()),
            Some("A".to_owned()),
            "first-seen (A) must win the tie"
        );
        // Clear winner ignores first-seen.
        assert_eq!(
            most_frequent(["A", "B", "B", "B"].into_iter()),
            Some("B".to_owned()),
            "majority wins over first-seen"
        );
        // Empty → None.
        assert_eq!(most_frequent(std::iter::empty::<&str>()), None);
        // Single element.
        assert_eq!(most_frequent(["only"].into_iter()), Some("only".to_owned()));
    }

    /// End-to-end regression: a `from_claims` fixture where the entity has
    /// exactly tied `subject` counts must resolve the node `label` to the
    /// first-seen subject, deterministically. The previous implementation was
    /// non-deterministic on this input.
    #[test]
    fn from_claims_label_tie_breaks_by_first_seen_deterministically() {
        let a = Uuid::new_v4();
        // Two "A" claims first, then two "B" claims — tied 2-2, "A" first-seen.
        let claims = vec![
            claim(a, "A", "p", json!(1), "stocks", "external_fact"),
            claim(a, "A", "p", json!(2), "stocks", "external_fact"),
            claim(a, "B", "p", json!(3), "stocks", "external_fact"),
            claim(a, "B", "p", json!(4), "stocks", "external_fact"),
        ];
        let g = GalaxyGraph::from_claims(&claims, 300, None);
        let node = &g.nodes()[0];
        assert_eq!(node.label, "A", "first-seen subject must win the tie");
    }
}
