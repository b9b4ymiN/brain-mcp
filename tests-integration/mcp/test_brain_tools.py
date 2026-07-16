"""Phase C Task C3 — brain_* tools end-to-end via stdio MCP."""

import json
import pytest


@pytest.mark.asyncio
async def test_brain_status_returns_healthy(mcp_env):
    """brain_status returns ledger info via real stdio MCP."""
    result = await mcp_env.call("brain_status", {})
    data = json.loads(result)
    assert data["status"] == "healthy"
    assert "ledger_head" in data
    assert "active_claims" in data


@pytest.mark.asyncio
async def test_brain_capture_confirm_search_round_trip(mcp_env):
    """Full capture → confirm → search cycle via real stdio MCP."""
    # 1. Capture (propose)
    cap_result = await mcp_env.call("brain_capture", {
        "operation_id": "e2e-cap-1",
        "utterance": "I prefer tabs over spaces",
        "subject": "editor-pref",
        "predicate": "preference",
        "value": "tabs",
        "domain": "projects",
    })
    cap_data = json.loads(cap_result)
    assert cap_data["status"] == "proposed"

    # 2. Confirm
    confirm_result = await mcp_env.call("brain_confirm", {
        "operation_id": "e2e-confirm-1",
        "proposal_operation_id": "e2e-cap-1",
    })
    confirm_data = json.loads(confirm_result)
    assert confirm_data["status"] == "confirmed"
    assert "claim_id" in confirm_data

    # 3. Search — the confirmed claim should appear
    search_result = await mcp_env.call("brain_search", {
        "query": "editor",
    })
    search_data = json.loads(search_result)
    assert search_data["count"] >= 1
    assert any(r["subject"] == "editor-pref" for r in search_data["results"])

    # 4. Get — read by subject
    get_result = await mcp_env.call("brain_get", {
        "subject": "editor-pref",
    })
    get_data = json.loads(get_result)
    assert get_data["count"] >= 1
    assert get_data["claims"][0]["value"] == "tabs"
