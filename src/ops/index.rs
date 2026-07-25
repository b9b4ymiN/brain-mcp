use anyhow::Result;

use crate::engine::{EngineState, WikiEngine};
use crate::index_manager;

/// Tear down and rebuild the tantivy index for the named wiki.
pub fn index_rebuild(manager: &WikiEngine, wiki_name: &str) -> Result<index_manager::IndexReport> {
    manager.rebuild_index(wiki_name)
}

/// Return the health and staleness status of the named wiki's index.
pub fn index_status(engine: &EngineState, wiki_name: &str) -> Result<index_manager::IndexStatus> {
    let space = engine.space(wiki_name)?;
    space.index_manager.status(&space.repo_root)
}

/// Incrementally update the index from git changes since the last indexed commit.
///
/// Layer 1 hook called by MCP handlers (`wiki_content_commit`, `wiki_ingest`)
/// immediately after their git commit succeeds. The underlying
/// `WikiEngine::refresh_index` → `index_manager.update` walks both committed
/// changes (vs `last_commit`) and uncommitted working-tree changes, so the
/// call is safe to make after any write or commit. Idempotent: when nothing
/// has changed since the last indexed state, `update()` returns a zero report.
pub fn index_after_commit(
    manager: &WikiEngine,
    wiki_name: &str,
) -> Result<index_manager::UpdateReport> {
    manager.refresh_index(wiki_name)
}
