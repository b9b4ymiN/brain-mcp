"""Layer 2 (filesystem watcher) end-to-end smoke test.

The watcher (src/watch.rs) is the safety net that catches file changes
bypassing the MCP write path. This test:

1. Starts the server with `--watch` against a temp wiki.
2. Writes a page directly to disk (bypassing MCP — simulating `git pull`
   or a host-side editor).
3. Polls `wiki_search` until the page appears (within debounce + a generous
   timeout), proving the watcher noticed the external write and refreshed
   the index.

Gated behind `--watch` because the standard test fixtures start `serve`
without it (see conftest.py:51).
"""
import asyncio
import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest
from mcp.client.stdio import stdio_client

from mcp import ClientSession, StdioServerParameters

BIN = os.environ.get("LLM_WIKI_BIN", "llm-wiki")
RESEARCH_FIXTURE = Path(__file__).parent.parent.parent / "tests" / "fixtures" / "wikis" / "research"


def _init_wiki(src: Path, dest: Path) -> None:
    shutil.copytree(src, dest)
    subprocess.run(["git", "-C", str(dest), "init", "-q"], check=True)
    subprocess.run(["git", "-C", str(dest), "add", "."], check=True)
    subprocess.run(
        [
            "git", "-C", str(dest),
            "-c", "user.name=test", "-c", "user.email=test@test.com",
            "commit", "-qm", "init",
        ],
        check=True,
    )


@pytest.mark.asyncio
async def test_watcher_picks_up_external_file_write(tmp_path):
    """Layer 2 contract: a file written outside MCP must become searchable."""
    # Build a fresh wiki under tmp_path and a config that points at it.
    repo_root = tmp_path / "brain"
    _init_wiki(RESEARCH_FIXTURE, repo_root)
    config_path = tmp_path / "config.toml"
    config_path.write_text(
        "[global]\n"
        'default_wiki = "research"\n\n'
        "[[wikis]]\n"
        'name = "research"\n'
        f'path = "{repo_root.as_posix()}"\n\n'
        "[watch]\n"
        "debounce_ms = 100\n"
    )

    server = StdioServerParameters(
        command=BIN,
        args=["--config", str(config_path), "serve", "--watch"],
    )

    ready: asyncio.Future = asyncio.get_event_loop().create_future()
    stop: asyncio.Event = asyncio.Event()
    session_holder: list = []
    exc_holder: list = []

    async def _run():
        try:
            async with stdio_client(server) as (read, write), ClientSession(read, write) as session:
                await session.initialize()
                session_holder.append(session)
                ready.set_result(None)
                await stop.wait()
        except Exception as e:
            if not ready.done():
                ready.set_exception(e)
            else:
                exc_holder.append(e)

    task = asyncio.ensure_future(_run())
    try:
        await ready

        sentinel = "ZQXWatcherLayer2Sentinel"
        page_path = repo_root / "wiki" / "concepts" / "watcher-sentinel.md"
        page_path.parent.mkdir(parents=True, exist_ok=True)
        page_path.write_text(
            "---\n"
            f"title: Watcher Sentinel\n"
            "type: concept\n"
            "status: active\n"
            "---\n\n"
            f"{sentinel} body.\n"
        )

        session = session_holder[0]

        # Poll for up to 10s. Debounce is 100ms; allow generous slack for
        # inotify latency on the bind mount / dev fs.
        deadline = asyncio.get_event_loop().time() + 10.0
        found = False
        last_results = None
        while asyncio.get_event_loop().time() < deadline:
            result = await session.call_tool("wiki_search", {"query": sentinel, "wiki": "research"})
            text = result.content[0].text if result.content else "{}"
            try:
                data = json.loads(text)
            except json.JSONDecodeError:
                data = {"results": []}
            last_results = data
            slugs = [r.get("slug", "") for r in data.get("results", [])]
            if any("watcher-sentinel" in s for s in slugs):
                found = True
                break
            await asyncio.sleep(0.25)

        assert found, (
            "watcher did not pick up external file write within 10s — "
            f"last search results: {last_results}"
        )
    finally:
        stop.set()
        try:
            await asyncio.wait_for(task, timeout=5)
        except TimeoutError:
            task.cancel()
        if exc_holder:
            raise exc_holder[0]


@pytest.mark.asyncio
async def test_watcher_poll_mode_via_env_override(tmp_path, monkeypatch):
    """Env var LLM_WIKI_WATCH_BACKEND=poll forces the PollWatcher backend.

    Auto-detection on a local filesystem would normally pick Native; the env
    override forces Poll regardless. A host-side write must then be picked
    up within poll_interval_ms (500ms here) + debounce (100ms) + slack.
    """
    repo_root = tmp_path / "brain"
    _init_wiki(RESEARCH_FIXTURE, repo_root)
    config_path = tmp_path / "config.toml"
    config_path.write_text(
        "[global]\n"
        'default_wiki = "research"\n\n'
        "[[wikis]]\n"
        'name = "research"\n'
        f'path = "{repo_root.as_posix()}"\n\n'
        "[watch]\n"
        "debounce_ms = 100\n"
    )

    # Force poll backend via env var with a short poll interval so the test
    # does not have to wait the 30s default.
    monkeypatch.setenv("LLM_WIKI_WATCH_BACKEND", "poll")
    monkeypatch.setenv("LLM_WIKI_WATCH_POLL_MS", "500")

    server = StdioServerParameters(
        command=BIN,
        args=["--config", str(config_path), "serve", "--watch"],
        env=dict(os.environ),
    )

    ready: asyncio.Future = asyncio.get_event_loop().create_future()
    stop: asyncio.Event = asyncio.Event()
    session_holder: list = []
    exc_holder: list = []

    async def _run():
        try:
            async with stdio_client(server) as (read, write), ClientSession(
                read, write
            ) as session:
                await session.initialize()
                session_holder.append(session)
                ready.set_result(None)
                await stop.wait()
        except Exception as e:
            if not ready.done():
                ready.set_exception(e)
            else:
                exc_holder.append(e)

    task = asyncio.ensure_future(_run())
    try:
        await ready

        sentinel = "ZQXEnvOverridePollSentinel"
        page_path = repo_root / "wiki" / "concepts" / "env-override-sentinel.md"
        page_path.parent.mkdir(parents=True, exist_ok=True)
        page_path.write_text(
            "---\n"
            f"title: Env Override Sentinel\n"
            "type: concept\n"
            "status: active\n"
            "---\n\n"
            f"{sentinel} body.\n"
        )

        session = session_holder[0]
        # Poll for up to 10s. Poll interval is 500ms + debounce 100ms + slack.
        deadline = asyncio.get_event_loop().time() + 10.0
        found = False
        last_results = None
        while asyncio.get_event_loop().time() < deadline:
            result = await session.call_tool(
                "wiki_search", {"query": sentinel, "wiki": "research"}
            )
            text = result.content[0].text if result.content else "{}"
            try:
                data = json.loads(text)
            except json.JSONDecodeError:
                data = {"results": []}
            last_results = data
            slugs = [r.get("slug", "") for r in data.get("results", [])]
            if any("env-override-sentinel" in s for s in slugs):
                found = True
                break
            await asyncio.sleep(0.5)

        assert found, (
            "env-override poll backend did not pick up external write within "
            f"10s — last results: {last_results}"
        )
    finally:
        stop.set()
        try:
            await asyncio.wait_for(task, timeout=5)
        except TimeoutError:
            task.cancel()
        if exc_holder:
            raise exc_holder[0]
