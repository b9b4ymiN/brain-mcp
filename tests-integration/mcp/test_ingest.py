async def test_ingest_dry_run_pages_validated(mcp_env):
    data = await mcp_env.json(
        "wiki_ingest",
        {"path": "inbox/01-paper-switch-transformer.md", "dry_run": True},
    )
    assert data["pages_validated"] >= 0


async def test_ingest_dry_run_has_warnings_array(mcp_env):
    data = await mcp_env.json(
        "wiki_ingest",
        {"path": "inbox/01-paper-switch-transformer.md", "dry_run": True},
    )
    assert isinstance(data["warnings"], list)


async def test_ingest_dry_run_unchanged_count(mcp_env):
    data = await mcp_env.json(
        "wiki_ingest",
        {"path": "inbox/01-paper-switch-transformer.md", "dry_run": True},
    )
    assert data["unchanged_count"] >= 0


async def test_ingest_redact_dry_run(mcp_env):
    data = await mcp_env.json(
        "wiki_ingest",
        {"path": "inbox/03-note-with-secrets.md", "dry_run": True, "redact": True},
    )
    assert data["pages_validated"] >= 0


async def test_ingest_response_surfaces_web_sync(mutable_mcp_env):
    """Layer 1 ingest path: ingest must refresh the index + sync web content.

    `ops::ingest_with_redact` already calls `manager.refresh_index` after the
    auto-commit, and `handle_ingest` already calls `sync_web_content`. This
    test pins the contract so a future refactor cannot silently regress it:
    after ingest, the response must carry a `web_content_synced` key and the
    page must be findable via `wiki_search` with no manual rebuild.
    """
    # Create + write a unique page via MCP (so the file lands under wiki_root
    # with the canonical layout the rest of the system expects).
    slug = "concepts/ingest-web-sync-sentinel"
    await mutable_mcp_env.json(
        "wiki_content_new", {"uri": slug, "wiki": "research"}
    )
    body = (
        "---\n"
        "title: Ingest Web Sync Sentinel\n"
        "type: concept\n"
        "status: active\n"
        "---\n\n"
        "ZQWIngestSentinel body.\n"
    )
    await mutable_mcp_env.call(
        "wiki_content_write",
        {"uri": slug, "content": body, "wiki": "research"},
    )

    # Ingest the page (path relative to wiki_root — matches the slug layout).
    report = await mutable_mcp_env.json(
        "wiki_ingest", {"path": "concepts/ingest-web-sync-sentinel.md", "wiki": "research"}
    )

    # `web_content_synced` must be present in the response (None when Hugo is
    # not installed in the test env, but the key must exist).
    assert "web_content_synced" in report, (
        f"ingest response missing web_content_synced: {report}"
    )

    # And search must find the page — refresh_index ran inside ingest_with_redact.
    search = await mutable_mcp_env.json(
        "wiki_search", {"query": "ZQWIngestSentinel", "wiki": "research"}
    )
    slugs = [r.get("slug", "") for r in search.get("results", [])]
    assert slug in slugs, f"ingest did not make page searchable: {search}"
