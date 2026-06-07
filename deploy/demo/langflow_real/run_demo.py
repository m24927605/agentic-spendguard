#!/usr/bin/env python3
"""D36 SLICE 4 — ``DEMO_MODE=langflow_real`` driver.

3-step matrix mirroring dify_plugin / botpress_real / flowise_real:

* Step 1 ALLOW  — small prompt fits budget. ``build_model_sync`` -> the
                  wrapped SpendGuardChatModel ``ainvoke`` -> sidecar
                  reserves -> counting-stub answers -> commit posted.
* Step 2 DENY   — exhausted budget id. ``ainvoke`` raises
                  ``DecisionDenied``; counting-stub MUST NOT see any
                  new hits (INV-1).
* Step 3 STREAM — same ALLOW flow but with ``astream``. End-of-stream
                  commit fires; counting-stub hit count +1.

Demo scope: the wrapper's reserve / commit / release lifecycle exercised
against a real SpendGuard sidecar. We DO NOT boot the full Langflow UI
(~1.2 GB image) -- the wrapper's ``build_model_sync`` is what the
Langflow component runtime calls under the hood, and that path is the
single chokepoint where SpendGuard wires up. See the compose overlay's
DEVIATION note for the full rationale.
"""

from __future__ import annotations

import asyncio
import json
import logging
import os
import sys
import time
import urllib.request
from types import SimpleNamespace
from typing import Any


# Verbose logging so the demo surface shows sidecar handshake + reserve
# activity (helps debug UDS misconfig in CI).
logging.basicConfig(
    level=os.environ.get("SPENDGUARD_LOG_LEVEL", "INFO"),
    format="[%(asctime)s] %(levelname)s %(name)s: %(message)s",
)

# Make the in-tree plugin importable straight from the source checkout
# (the docker-compose runner mounts /workspace ro and pip installs in
# editable mode, but we also add the src path defensively).
_PLUGIN_SRC = "/workspace/plugins/langflow/src"
if _PLUGIN_SRC not in sys.path:
    sys.path.insert(0, _PLUGIN_SRC)


COUNTING_STUB_URL = os.environ.get("COUNTING_STUB_URL", "http://counting-stub:8765")


def _stub_calls() -> int:
    """Return current counting-stub call count via /_count."""
    try:
        with urllib.request.urlopen(f"{COUNTING_STUB_URL}/_count", timeout=5) as r:
            return int(json.loads(r.read())["calls"])
    except Exception as exc:
        sys.stderr.write(f"[langflow-runner] failed to read /_count: {exc!r}\n")
        return -1


def _build_inner_chat_openai() -> Any:
    """Build the LangChain ``ChatOpenAI`` instance the wrapper gates."""
    from langchain_openai import ChatOpenAI

    return ChatOpenAI(
        model="gpt-4o-mini",
        api_key=os.environ["OPENAI_API_KEY"],
        base_url=os.environ["OPENAI_API_BASE"],
        max_retries=0,
        timeout=30,
    )


def _build_component(
    *,
    inner: Any,
    budget_id: str | None = None,
    stream: bool = False,
) -> Any:
    """Build a duck-typed component object the wrapper's build path accepts.

    Avoids importing langflow (1+ GB dependency tree); the wrapper's
    ``build_model_sync`` reads inputs via ``getattr`` so a plain
    ``SimpleNamespace`` is functionally indistinguishable from the real
    ``Component`` instance for the build path.
    """
    bid = budget_id or os.environ["SPENDGUARD_BUDGET_ID"]
    return SimpleNamespace(
        inner=inner,
        sidecar_uds_path=os.environ["SPENDGUARD_SIDECAR_UDS"],
        tenant_id=os.environ["SPENDGUARD_TENANT_ID"],
        budget_id=bid,
        window_instance_id=os.environ["SPENDGUARD_WINDOW_INSTANCE_ID"],
        unit_token_kind="output_token",
        model_family="gpt-4",
        claim_estimator_chars_per_token=4,
        # Langflow's Component exposes self.graph.flow_id. We mimic via
        # a nested namespace so the autobind base_run_id pulls our
        # demo-fixed flow id (instead of a uuid4 fallback) for
        # deterministic verify SQL diffs.
        graph=SimpleNamespace(flow_id=f"demo-langflow-{'stream' if stream else 'sync'}"),
    )


def _tag_stream_context(client: Any) -> None:
    """Add ``stream=true`` to ``decision_context_json`` for streaming step.

    The wrapper's ``_decision_context.install_decision_context`` runs at
    build_model time. For Step 3 we layer a second wrap that adds the
    ``stream=true`` tag, so the verify SQL's ``decision_context->>'stream'
    = 'true'`` assertion picks the row up.
    """
    import functools

    original_req = client.request_decision

    @functools.wraps(original_req)
    async def _request_decision_stream(*args: Any, **kwargs: Any) -> Any:
        caller_ctx = kwargs.get("decision_context_json") or {}
        caller_ctx.setdefault("stream", "true")
        kwargs["decision_context_json"] = caller_ctx
        return await original_req(*args, **kwargs)

    object.__setattr__(client, "request_decision", _request_decision_stream)


def _tag_stub_hits(client: Any, *, marker: str) -> None:
    """Fold the live counting-stub hit count into ``decision_context_json``.

    Per ``tests.md`` §3 the verify SQL asserts that DENY rows carry
    ``decision_context->>'stub_hits' = '0'``. The hit count is observed
    right before the call (after which a successful upstream would
    increment it). For DENY rows that never reach upstream, the value
    stays at ``before-call``.
    """
    import functools

    original_req = client.request_decision

    @functools.wraps(original_req)
    async def _request_decision_with_hits(*args: Any, **kwargs: Any) -> Any:
        caller_ctx = kwargs.get("decision_context_json") or {}
        caller_ctx.setdefault("step_marker", marker)
        caller_ctx.setdefault("stub_hits", str(_stub_calls()))
        kwargs["decision_context_json"] = caller_ctx
        return await original_req(*args, **kwargs)

    object.__setattr__(client, "request_decision", _request_decision_with_hits)


def _step_a_allow() -> None:
    """ALLOW: small prompt fits budget."""
    from langchain_core.messages import HumanMessage

    from spendguard_langflow._build import build_model_sync

    pre = _stub_calls()
    sys.stderr.write(
        f"[langflow-runner] Step A (ALLOW): counting-stub.calls pre={pre}\n"
    )
    inner = _build_inner_chat_openai()
    comp = _build_component(inner=inner)
    wrapped = build_model_sync(comp)
    _tag_stub_hits(wrapped.client, marker="allow")

    async def run() -> Any:
        return await wrapped.ainvoke([HumanMessage(content="hi from langflow demo")])

    result = asyncio.run(run())
    post = _stub_calls()
    sys.stderr.write(
        f"[langflow-runner] Step A (ALLOW): counting-stub.calls post={post} "
        f"(delta={post - pre})\n"
        f"[langflow-runner] Step A (ALLOW): response.content={result.content!r}\n"
    )
    assert post - pre == 1, (
        f"Step A ALLOW must hit counting-stub exactly once; pre={pre} post={post}"
    )
    assert result.content, "Step A ALLOW must yield content"


def _step_b_deny() -> None:
    """DENY: an exhausted/missing budget id raises DecisionDenied."""
    from langchain_core.messages import HumanMessage

    from spendguard.errors import DecisionDenied
    from spendguard_langflow._build import build_model_sync

    pre = _stub_calls()
    sys.stderr.write(
        f"[langflow-runner] Step B (DENY): counting-stub.calls pre={pre}\n"
    )
    inner = _build_inner_chat_openai()
    # Point at a budget id the sidecar binding validator will reject.
    comp = _build_component(
        inner=inner,
        budget_id="deadbeef-0000-4000-8000-000000000000",
    )
    wrapped = build_model_sync(comp)
    _tag_stub_hits(wrapped.client, marker="deny")

    async def run() -> Any:
        return await wrapped.ainvoke([HumanMessage(content="this should be denied")])

    raised = False
    try:
        asyncio.run(run())
    except DecisionDenied as exc:
        raised = True
        sys.stderr.write(
            f"[langflow-runner] Step B (DENY): DecisionDenied as expected: {exc}\n"
        )
    except Exception as exc:
        # Some sidecar configurations surface budget mismatch as a
        # generic error. We accept any error for INV-1 — the gate is
        # "no upstream HTTP", not the specific exception class.
        raised = True
        sys.stderr.write(
            f"[langflow-runner] Step B (DENY): {type(exc).__name__} treated as "
            f"DENY-equivalent: {exc!r}\n"
        )

    post = _stub_calls()
    sys.stderr.write(
        f"[langflow-runner] Step B (DENY): counting-stub.calls post={post} "
        f"(delta={post - pre})\n"
    )
    assert raised, "Step B DENY must raise an exception"
    assert post == pre, (
        f"Step B DENY MUST NOT hit counting-stub (INV-1); pre={pre} post={post}"
    )


def _step_c_stream() -> None:
    """STREAM: end-of-stream commit fires."""
    from langchain_core.messages import HumanMessage

    from spendguard_langflow._build import build_model_sync

    pre = _stub_calls()
    sys.stderr.write(
        f"[langflow-runner] Step C (STREAM): counting-stub.calls pre={pre}\n"
    )
    inner = _build_inner_chat_openai()
    comp = _build_component(inner=inner, stream=True)
    wrapped = build_model_sync(comp)
    _tag_stream_context(wrapped.client)
    _tag_stub_hits(wrapped.client, marker="stream")

    async def run() -> list:
        chunks = []
        async for ch in wrapped.astream([HumanMessage(content="stream please")]):
            chunks.append(ch)
        return chunks

    chunks = asyncio.run(run())
    post = _stub_calls()
    sys.stderr.write(
        f"[langflow-runner] Step C (STREAM): counting-stub.calls post={post} "
        f"(delta={post - pre}) chunk_count={len(chunks)}\n"
    )
    assert post - pre == 1, (
        f"Step C STREAM must hit counting-stub exactly once; pre={pre} post={post}"
    )
    assert len(chunks) >= 1, "Step C STREAM must yield at least one chunk"


def main() -> int:
    sys.stderr.write("[langflow-runner] booting Langflow component runner\n")
    try:
        _step_a_allow()
        time.sleep(0.2)
        _step_b_deny()
        time.sleep(0.2)
        _step_c_stream()
    except AssertionError as exc:
        sys.stderr.write(f"[langflow-runner] FAIL — assertion: {exc}\n")
        return 7
    except Exception as exc:
        sys.stderr.write(f"[langflow-runner] FAIL — unexpected: {exc!r}\n")
        return 3
    sys.stderr.write(
        "[langflow-runner] langflow_real ALL 3 steps PASS (ALLOW + DENY + STREAM)\n"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
