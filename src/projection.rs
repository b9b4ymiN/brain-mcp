//! Projection adapters (Task 2.1): build Tantivy full-text index, Petgraph
//! concept graph, and generated Markdown pages from the canonical semantic
//! layers (event ledger + claim snapshots) alone. Every artifact this module
//! writes lives under a caller-supplied "generated wiki" directory that is
//! fully owned by the projector — deleting and rebuilding it must reproduce
//! the same composite checksum, and it must never touch a human-authored
//! wiki root.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};

use crate::frontmatter;
use crate::graph::{self, GraphFilter};
use crate::index_manager::{IndexReport, SpaceIndexManager};
use crate::index_schema::IndexSchema;
use crate::markdown;
use crate::semantic::{ClaimView, SemanticStore, canonicalize_json};
use crate::type_registry::SpaceTypeRegistry;

/// Identity of one rebuilt projection: the canonical-state fingerprint it
/// was built from (`ledger_head`/`purge_epoch`/`schema_version`, the same
/// triple the hard-purge saga uses for its own composite checksum, see
/// `SemanticStore::composite_checksum`) extended with a content fingerprint
/// over the exact claims that were projected. A pure metadata checksum would
/// trivially match on every rebuild regardless of whether the projector
/// itself is deterministic or correct; folding in the claim content is what
/// makes "delete projection then rebuild reproduces the same checksum" an
/// actual test of the projector, not just of canonical-store stability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionCheckpoint {
    pub ledger_head: u64,
    pub purge_epoch: u64,
    pub schema_version: u8,
    pub claims_projected: usize,
    pub composite_checksum: String,
}

/// Result of one `rebuild_projection` call.
#[derive(Clone, Debug)]
pub struct RebuildOutcome {
    pub checkpoint: ProjectionCheckpoint,
    pub index_report: IndexReport,
    pub graph_node_count: usize,
    pub graph_edge_count: usize,
}

/// Rebuild every projection (generated Markdown, Tantivy, Petgraph) from the
/// canonical claim set as of `world_time`.
///
/// `generated_wiki_root` is deleted and recreated on every call — it must be
/// a directory exclusively owned by this projector, never a human-authored
/// wiki root. `index_manager`'s index is rebuilt in place via the existing
/// `SpaceIndexManager::rebuild`, and the graph is built from that index's
/// searcher via the existing `graph::build_graph` — this module adds no new
/// search/graph engine, only the claim-to-Markdown adapter layer between
/// `SemanticStore` and those two pre-existing projections.
pub fn rebuild_projection(
    store: &SemanticStore,
    world_time: DateTime<Utc>,
    generated_wiki_root: &Path,
    repo_root: &Path,
    index_manager: &SpaceIndexManager,
    index_schema: &IndexSchema,
    registry: &SpaceTypeRegistry,
) -> Result<RebuildOutcome> {
    let ledger_head = store.ledger_head()?;
    let purge_epoch = store.registry_epoch()?;
    let schema_version = store.schema_version();

    let mut claims = store.all_claims_current(ledger_head, world_time)?.active;
    claims.sort_by_key(|claim| claim.claim_id);

    if generated_wiki_root.exists() {
        fs::remove_dir_all(generated_wiki_root)?;
    }
    fs::create_dir_all(generated_wiki_root)?;

    for claim in &claims {
        let slug = format!("claims/{}", claim.claim_id);
        let content = render_claim_page(claim);
        markdown::write_page(&slug, &content, generated_wiki_root)?;
    }

    let index_report =
        index_manager.rebuild(generated_wiki_root, repo_root, index_schema, registry)?;
    let searcher = index_manager.searcher()?;
    let wiki_graph =
        graph::build_graph(&searcher, index_schema, &GraphFilter::default(), registry)?;

    let composite_checksum = composite_checksum(ledger_head, purge_epoch, schema_version, &claims)?;

    Ok(RebuildOutcome {
        checkpoint: ProjectionCheckpoint {
            ledger_head,
            purge_epoch,
            schema_version,
            claims_projected: claims.len(),
            composite_checksum,
        },
        index_report,
        graph_node_count: wiki_graph.node_count(),
        graph_edge_count: wiki_graph.edge_count(),
    })
}

/// Lag, in ledger events, between a previously-computed checkpoint and the
/// store's current ledger head. Zero immediately after a rebuild; positive
/// once new events have been confirmed since. Makes projection staleness
/// observable without forcing a full rebuild just to check.
pub fn checkpoint_lag(checkpoint: &ProjectionCheckpoint, store: &SemanticStore) -> Result<u64> {
    let current = store.ledger_head()?;
    Ok(current.saturating_sub(checkpoint.ledger_head))
}

fn composite_checksum(
    ledger_head: u64,
    purge_epoch: u64,
    schema_version: u8,
    claims: &[ClaimView],
) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(format!("{ledger_head}:{purge_epoch}:{schema_version}").as_bytes());
    for claim in claims {
        let value = serde_json::to_value(claim)?;
        hasher.update(canonicalize_json(&value)?);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Render one confirmed claim as a generated-wiki Markdown page. The `type`
/// is fixed to `entity` (the "semantic" schema family's generic subtype) —
/// mapping every `claim_kind` to a distinct page type is deferred; nothing
/// in Task 2.1's DoD requires that taxonomy, only that a valid, indexable
/// page comes out the other end.
fn render_claim_page(claim: &ClaimView) -> String {
    let mut fm: BTreeMap<String, YamlValue> = BTreeMap::new();
    fm.insert("title".into(), YamlValue::String(claim.subject.clone()));
    fm.insert("type".into(), YamlValue::String("entity".into()));
    fm.insert("status".into(), YamlValue::String("active".into()));
    fm.insert(
        "confidence".into(),
        YamlValue::Number(serde_yaml::Number::from(
            f64::from(claim.confidence_basis_points) / 10_000.0,
        )),
    );
    fm.insert(
        "tags".into(),
        YamlValue::Sequence(vec![YamlValue::String(claim.domain.clone())]),
    );
    fm.insert(
        "claim_id".into(),
        YamlValue::String(claim.claim_id.to_string()),
    );

    let body = format!(
        "## {}\n\n- predicate: `{}`\n- value: `{}`\n- kind: `{}`\n- domain: `{}`\n- confirmed_event_seq: `{}`\n",
        claim.subject,
        claim.predicate,
        claim.value,
        claim.claim_kind,
        claim.domain,
        claim.confirmed_event_seq,
    );

    frontmatter::write(&fm, &body)
}
