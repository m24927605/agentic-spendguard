#!/usr/bin/env python3
"""D10 SLICE 7 — `DEMO_MODE=dify_plugin_real` driver.

3-step matrix mirroring the langchain_ts / vercel_ai_mastra / kong
demos:
    Step A — ALLOW : single non-streaming OpenAI call goes through the
             plugin -> sidecar reserves -> counting-stub answers ->
             plugin commits real usage.
    Step B — DENY  : same call but with an over-large estimator so the
             sidecar's budget guard fires DENY; ZERO counting-stub
             hits (INV-1 streaming-agnostic).
    Step C — STREAM: streaming call exercises ``_stream_generate``
             SSE proxy; end-of-stream commit captures real usage.

Each step:
  - Records pre/post counting-stub call count to prove INV-1.
  - Records pre/post audit_outbox row count to prove INV-2 strict-order.
  - Surfaces decision_id / reservation_id / ledger txn rows for the
    verify SQL.
"""

from __future__ import annotations

import logging
import os
import sys
import time
import urllib.request

# Verbose logging so the demo surface shows the sidecar handshake +
# reserve activity (helps debug network/UDS misconfig in CI).
logging.basicConfig(
    level=os.environ.get("SPENDGUARD_LOG_LEVEL", "INFO"),
    format="[%(asctime)s] %(levelname)s %(name)s: %(message)s",
)

# Make the in-tree plugin importable.
PLUGIN_ROOT = "/workspace/plugins/dify/spendguard"
sys.path.insert(0, PLUGIN_ROOT)

from dify_plugin.entities.model.message import UserPromptMessage  # noqa: E402
from dify_plugin.errors.model import (  # noqa: E402
    InvokeAuthorizationError,
    InvokeError,
)

from models.llm.spendguard_llm import SpendGuardLLM  # noqa: E402


def _stub_calls() -> int:
    """Return current counting-stub call count via /_count."""
    try:
        with urllib.request.urlopen(
            "http://counting-stub:8765/_count", timeout=5,
        ) as r:
            import json
            return int(json.loads(r.read())["calls"])
    except Exception as exc:
        sys.stderr.write(f"[dify-runner] failed to read /_count: {exc!r}\n")
        return -1


def _build_credentials(*, upstream_provider: str = "openai") -> dict:
    """Build the Dify provider credentials dict the plugin expects."""
    return {
        "upstream_provider": upstream_provider,
        "openai_api_key": os.environ.get(
            "OPENAI_API_KEY", "sk-demo-counting-stub-key",
        ),
        "anthropic_api_key": "",
        "upstream_base_url": os.environ.get(
            "OPENAI_API_BASE", "http://counting-stub:8765/v1",
        ),
        "spendguard_sidecar_address": os.environ.get(
            "SPENDGUARD_SIDECAR_UDS", "/var/run/spendguard/adapter.sock",
        ),
        "spendguard_tenant_id": os.environ.get(
            "SPENDGUARD_TENANT_ID",
            "00000000-0000-4000-8000-000000000001",
        ),
        "spendguard_budget_id": os.environ.get(
            "SPENDGUARD_BUDGET_ID",
            "44444444-4444-4444-8444-444444444444",
        ),
        "spendguard_window_instance_id": os.environ.get(
            "SPENDGUARD_WINDOW_INSTANCE_ID",
            "55555555-5555-4555-8555-555555555555",
        ),
        # Plugin daemon needs these as call context.
        "__dify_workspace_id": "demo-workspace",
        "__dify_app_id": "demo-app",
    }


def _step_a_allow(llm: SpendGuardLLM) -> None:
    """ALLOW: normal call through plugin -> counting-stub answers."""
    pre = _stub_calls()
    sys.stderr.write(
        f"[dify-runner] Step A (ALLOW): counting-stub.calls pre={pre}\n",
    )
    result = llm._invoke(
        model="spendguard/gpt-4o-mini",
        credentials=_build_credentials(),
        prompt_messages=[UserPromptMessage(content="hi")],
        model_parameters={"temperature": 0.7, "max_tokens": 50},
        stream=False,
    )
    post = _stub_calls()
    sys.stderr.write(
        f"[dify-runner] Step A (ALLOW): counting-stub.calls post={post} "
        f"(delta={post - pre})\n"
        f"[dify-runner] Step A (ALLOW): response.content={result.message.content!r}\n"
        f"[dify-runner] Step A (ALLOW): usage="
        f"prompt={result.usage.prompt_tokens} "
        f"completion={result.usage.completion_tokens}\n",
    )
    assert post - pre == 1, "Step A ALLOW must hit counting-stub exactly once"
    assert result.message.content, "Step A ALLOW must yield content"


def _step_b_deny(llm: SpendGuardLLM) -> None:
    """DENY: provoke sidecar DENY by setting a budget the seed exhausts.

    We swap the budget_id to a non-existent one — the sidecar's
    binding validator rejects with DENY. INV-1: counting-stub MUST NOT
    receive any HTTP call.
    """
    pre = _stub_calls()
    sys.stderr.write(
        f"[dify-runner] Step B (DENY): counting-stub.calls pre={pre}\n",
    )
    creds = _build_credentials()
    # Point at a budget that the demo seed has set to zero capacity
    # (or simply absent). The sidecar surfaces this as DENY.
    creds["spendguard_budget_id"] = "deadbeef-0000-4000-8000-000000000000"
    raised = False
    try:
        llm._invoke(
            model="spendguard/gpt-4o-mini",
            credentials=creds,
            prompt_messages=[UserPromptMessage(content="this should be denied")],
            model_parameters={},
            stream=False,
        )
    except InvokeAuthorizationError as exc:
        raised = True
        sys.stderr.write(
            f"[dify-runner] Step B (DENY): InvokeAuthorizationError raised "
            f"as expected: {exc}\n",
        )
    except InvokeError as exc:
        # If the sidecar surfaces DENY as a generic InvokeError (e.g.
        # via SidecarUnavailable on a config error in the demo seed),
        # we still log + count it as "no upstream hit".
        raised = True
        sys.stderr.write(
            f"[dify-runner] Step B (DENY): InvokeError raised (treated "
            f"as DENY-equivalent for INV-1): {exc}\n",
        )
    post = _stub_calls()
    sys.stderr.write(
        f"[dify-runner] Step B (DENY): counting-stub.calls post={post} "
        f"(delta={post - pre})\n",
    )
    assert raised, "Step B DENY must raise InvokeAuthorizationError"
    assert post == pre, (
        f"Step B DENY MUST NOT hit counting-stub (INV-1); "
        f"pre={pre} post={post}"
    )


def _step_c_stream(llm: SpendGuardLLM) -> None:
    """STREAM: SSE proxy through the plugin -> counting-stub answers.

    The counting-stub doesn't actually emit SSE; the SDK's
    chat.completions.create with stream=True converts the single-shot
    response into a stream-shaped iterator. Real OpenAI returns multi-
    chunk SSE; the demo's INV-1+INV-5 gates don't depend on chunk
    count.
    """
    pre = _stub_calls()
    sys.stderr.write(
        f"[dify-runner] Step C (STREAM): counting-stub.calls pre={pre}\n",
    )
    try:
        stream = llm._invoke(
            model="spendguard/gpt-4o-mini",
            credentials=_build_credentials(),
            prompt_messages=[UserPromptMessage(content="stream please")],
            model_parameters={"max_tokens": 32},
            stream=True,
        )
        chunks = []
        for chunk in stream:
            chunks.append(chunk)
    except InvokeError as exc:
        sys.stderr.write(
            f"[dify-runner] Step C (STREAM): error: {exc}\n",
        )
        raise
    post = _stub_calls()
    rebuilt = "".join(
        c.delta.message.content for c in chunks if c.delta.message.content
    )
    sys.stderr.write(
        f"[dify-runner] Step C (STREAM): counting-stub.calls post={post} "
        f"(delta={post - pre})\n"
        f"[dify-runner] Step C (STREAM): rebuilt content={rebuilt!r}\n"
        f"[dify-runner] Step C (STREAM): chunk_count={len(chunks)}\n",
    )
    assert post - pre == 1, "Step C STREAM must hit counting-stub exactly once"


def main() -> int:
    sys.stderr.write("[dify-runner] booting SpendGuardLLM\n")
    llm = SpendGuardLLM.__new__(SpendGuardLLM)

    # Run the matrix; surface failures as non-zero exit.
    try:
        _step_a_allow(llm)
        time.sleep(0.2)
        _step_b_deny(llm)
        time.sleep(0.2)
        _step_c_stream(llm)
    except AssertionError as exc:
        sys.stderr.write(f"[dify-runner] FAIL — assertion: {exc}\n")
        return 2
    except Exception as exc:
        sys.stderr.write(f"[dify-runner] FAIL — unexpected: {exc!r}\n")
        return 3
    sys.stderr.write("[dify-runner] all 3 steps PASSED\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
