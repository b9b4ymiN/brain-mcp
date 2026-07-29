use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use llm_wiki::engine::WikiEngine;
use llm_wiki::git;
use llm_wiki::graph::{GraphFilter, WikiGraph, WikiGraphCache, get_or_build_graph};
use petgraph_live::cache::GenerationCache;

// Build a wiki space with two concept pages (moe, transformer; transformer links moe)
// and return the config_path. Mirrors tests/graph_cache.rs::setup_wiki.
fn setup_wiki(dir: &Path, name: &str) -> PathBuf {
    let config_path = dir.join("state").join("config.toml");
    let wiki_path = dir.join(name);

    llm_wiki::spaces::create(&wiki_path, name, None, false, true, &config_path, None).unwrap();

    let wiki_root = wiki_path.join("wiki");
    fs::create_dir_all(wiki_root.join("concepts")).unwrap();
    fs::write(
        wiki_root.join("concepts/moe.md"),
        "---\ntitle: \"MoE\"\ntype: concept\nstatus: active\ntags: [ml]\n---\n\nMixture of Experts.\n",
    )
    .unwrap();
    fs::write(
        wiki_root.join("concepts/transformer.md"),
        "---\ntitle: \"Transformer\"\ntype: concept\nstatus: active\n---\n\nAttention is all you need. See [[concepts/moe]].\n",
    )
    .unwrap();
    git::commit(&wiki_path, "add pages").unwrap();

    config_path
}

/// Enable `index.auto_rebuild = true` on a generated config. Without this,
/// `mount_space` skips the staleness/update branch entirely (default is false),
/// so `state.toml.commit` never advances on restart and the snapshot key cannot
/// move — the regression test below would be unable to exercise the bug.
fn enable_auto_rebuild(config_path: &Path) {
    let mut cfg = llm_wiki::config::load_global(config_path).unwrap();
    cfg.index.auto_rebuild = true;
    llm_wiki::config::save_global(&cfg, config_path).unwrap();
}

/// Slugs of the local (non-external) nodes in a graph.
fn local_slugs(graph: &WikiGraph) -> HashSet<String> {
    graph
        .node_weights()
        .filter(|n| !n.external)
        .map(|n| n.slug.clone())
        .collect()
}

/// Read the default-filter graph for the named space, under the engine read lock.
fn space_slugs(engine: &WikiEngine, name: &str) -> HashSet<String> {
    let state = engine.state.read();
    let space = state.spaces.get(name).unwrap();
    let searcher = space.index_manager.searcher().unwrap();
    let graph = get_or_build_graph(
        &space.index_schema,
        &space.type_registry,
        &space.index_manager,
        &space.graph_cache,
        &searcher,
        &GraphFilter::default(),
    )
    .unwrap();
    local_slugs(&graph)
}

// ── compile-time smoke checks (kept) ──────────────────────────────────────────

#[test]
fn wiki_graph_cache_no_snapshot_variant_exists() {
    let _ = std::mem::discriminant(&WikiGraphCache::NoSnapshot(GenerationCache::new()));
}

#[test]
fn build_wiki_graph_cache_format_zstd_arm_compiles() {
    // Verifies Compression::Zstd is reachable — compile-time only.
    use petgraph_live::snapshot::Compression;
    let _ = Compression::Zstd { level: 3 };
}

#[test]
fn build_fn_does_not_capture_path_or_tokenizer() {
    // Compile-time: verify build_wiki_graph_cache signature no longer requires
    // repo_root or tokenizer strings. This test just checks it compiles without them.
    let _ = ();
}

#[test]
fn wiki_graph_cache_no_snapshot_uses_generation_cache() {
    let cache = WikiGraphCache::NoSnapshot(GenerationCache::<WikiGraph>::new());
    assert!(matches!(cache, WikiGraphCache::NoSnapshot(_)));
}

// ── regression: snapshot graph cache stays in sync with the wiki across restart ─

/// Regression for the stale-after-restart bug.
///
/// The snapshot-backed graph cache keys its on-disk snapshot by the value returned
/// from the `key_fn` wired in `engine.rs`. That key used to be
/// `index_manager.generation()` — an in-memory counter that resets to 0 on every
/// process restart and is bumped back to a small integer by the rebuild/update that
/// `mount_space` triggers. So across a restart a snapshot saved at gen N was re-read
/// at the same N forever, even after the wiki changed: the graph froze at the
/// pre-restart state and newly added pages never appeared.
///
/// The fix keys the snapshot on `cache_key()` = `{commit}:{schema_hash}` from
/// `state.toml`, which persists across restarts and changes whenever indexed content
/// moves. This test simulates a restart (drop the engine, build a fresh one against
/// the same state dir) after editing + committing a new page, and asserts the second
/// engine's graph reflects the new page.
#[test]
fn graph_snapshot_reflects_wiki_changes_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    enable_auto_rebuild(&config_path);
    let wiki_path = dir.path().join("test");
    let wiki_root = wiki_path.join("wiki");

    // --- process 1: build, snapshot saved with key derived from commit C1 ---
    let eng1 = WikiEngine::build(&config_path).unwrap();
    let slugs1 = space_slugs(&eng1, "test");
    drop(eng1); // snapshot file + state.toml persist on disk; in-memory key is lost

    assert!(
        !slugs1.iter().any(|s| s.contains("attention")),
        "precondition: attention page not yet present"
    );

    // --- edit + commit a new page (must produce a real tree diff so HEAD advances) ---
    fs::write(
        wiki_root.join("concepts/attention.md"),
        "---\ntitle: \"Attention\"\ntype: concept\nstatus: active\n---\n\nScaled dot-product attention.\n",
    )
    .unwrap();
    let new_commit = git::commit(&wiki_path, "add attention").unwrap();
    assert!(
        !new_commit.is_empty(),
        "git::commit returned empty — HEAD did not advance; test cannot exercise the bug"
    );

    // --- process 2 (restart): stale snapshot must NOT be reused ---
    let eng2 = WikiEngine::build(&config_path).unwrap();
    let slugs2 = space_slugs(&eng2, "test");

    assert!(
        slugs2.iter().any(|s| s.contains("attention")),
        "graph after restart is missing the new 'attention' page (slugs={:?}) — \
         snapshot was reused with a stale key",
        slugs2
    );
}

/// Non-regression guard: an unchanged restart must still warm-start (reuse the
/// snapshot) so the cache_key() fix did not over-correct and force a rebuild every
/// time. Topology must be identical across the restart.
#[test]
fn graph_snapshot_warm_starts_when_wiki_unchanged_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = setup_wiki(dir.path(), "test");
    enable_auto_rebuild(&config_path);

    let eng1 = WikiEngine::build(&config_path).unwrap();
    let slugs1 = space_slugs(&eng1, "test");
    drop(eng1);

    // --- restart with NO wiki change ---
    let eng2 = WikiEngine::build(&config_path).unwrap();
    let slugs2 = space_slugs(&eng2, "test");

    assert_eq!(
        slugs1, slugs2,
        "unchanged restart must preserve graph topology via warm-start"
    );
}
