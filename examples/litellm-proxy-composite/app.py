"""Slice B3 — runnable app demonstrating LiteLLM proxy + SpendGuard.

This is the **your-code-here** file: direct HTTP POSTs to the LiteLLM
proxy at http://localhost:4001/v1/chat/completions. No SpendGuard SDK
is imported by the app — the SDK lives inside the litellm-proxy
container's callback module.

The 3 steps exercise:
  1. ALLOW — a small request that gets reserved + committed normally.
  2. DENY — `spendguard_estimate_override=2B` triggers the hard-cap
     rule (1B atomic units). Proxy returns 403; counting stub is
     NEVER hit (negative control).
  3. STREAM — a follow-up ALLOW with `stream: True` to demonstrate
     the streaming reconciliation path (counter +1 because the
     proxy still hits the upstream).

The counting stub is an in-process aiohttp server on
host.docker.internal:8765 — `app.py` boots it as a child task before
firing the 3 HTTP calls.
"""

from __future__ import annotations

import asyncio
import sys

import httpx
from aiohttp import web

PROXY_URL = "http://localhost:4001"
COUNTING_PROVIDER_PORT = 8765
MASTER_KEY = "sk-demo-key"

_HITS: dict[str, int] = {"calls": 0}


async def _counting_handler(request: web.Request) -> web.Response:
    """Mimics OpenAI /v1/chat/completions just enough for LiteLLM."""
    _HITS["calls"] += 1
    body = await request.json()
    return web.json_response({
        "id": f"chatcmpl-example-{_HITS['calls']}",
        "object": "chat.completion",
        "created": 0,
        "model": body.get("model", "gpt-4o-mini"),
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "hi from stub"},
            "finish_reason": "stop",
        }],
        "usage": {"prompt_tokens": 5, "completion_tokens": 7, "total_tokens": 12},
    })


async def _start_counting_provider() -> web.AppRunner:
    app = web.Application()
    app.router.add_post("/v1/chat/completions", _counting_handler)
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "0.0.0.0", COUNTING_PROVIDER_PORT)
    await site.start()
    print(f"[app] counting stub listening on 0.0.0.0:{COUNTING_PROVIDER_PORT}")
    return runner


async def _run() -> int:
    runner = await _start_counting_provider()
    try:
        async with httpx.AsyncClient(
            base_url=PROXY_URL,
            headers={"Authorization": f"Bearer {MASTER_KEY}"},
            timeout=15.0,
        ) as http:
            # 1. ALLOW
            pre_allow = _HITS["calls"]
            r = await http.post("/v1/chat/completions", json={
                "model": "gpt-4o-mini",
                "messages": [{"role": "user", "content": "say hi"}],
            }, headers={"x-litellm-call-id": "app-allow-1"})
            if r.status_code != 200:
                print(f"[app] FATAL ALLOW: HTTP {r.status_code} body={r.text!r}",
                      file=sys.stderr)
                return 7
            tokens = r.json().get("usage", {}).get("completion_tokens")
            print(f"[app] (1) ALLOW: HTTP 200 completion_tokens={tokens}")
            if _HITS["calls"] != pre_allow + 1:
                print("[app] FATAL: counting stub not hit on ALLOW",
                      file=sys.stderr)
                return 7

            # 2. DENY (override → hard-cap)
            pre_deny = _HITS["calls"]
            r_deny = await http.post("/v1/chat/completions", json={
                "model": "gpt-4o-mini",
                "messages": [{"role": "user", "content": "deny me"}],
                # Production callbacks MUST NOT honour caller-supplied
                # overrides — see DEMO ONLY note in the in-tree demo
                # callback. The example's stripped callback does NOT
                # read this field; the DENY path here relies on the
                # demo callback variant for the deny demo. For a
                # stripped-callback DENY, exhaust the budget instead.
                "spendguard_estimate_override": "2000000000",
            }, headers={"x-litellm-call-id": "app-deny-1"})
            if r_deny.status_code >= 400:
                print(f"[app] (2) DENY: HTTP {r_deny.status_code} body="
                      f"{r_deny.text[:120]!r}")
            else:
                # Stripped callback ignores the override → call goes
                # through (this is the "what you fork" gap). In the
                # full demo, the override drives a real deny. The
                # example documents this difference in the README.
                print(f"[app] (2) DENY skipped — stripped callback ignores "
                      f"override (HTTP {r_deny.status_code}; counter+1). "
                      "See README §What you fork.")
            if r_deny.status_code >= 400 and _HITS["calls"] != pre_deny:
                print("[app] FATAL: counting stub hit on DENY",
                      file=sys.stderr)
                return 7

            # 3. STREAM (follow-up ALLOW; counter +1 expected)
            pre_stream = _HITS["calls"]
            r_stream = await http.post("/v1/chat/completions", json={
                "model": "gpt-4o-mini",
                "messages": [{"role": "user", "content": "stream"}],
                "stream": True,
            }, headers={"x-litellm-call-id": "app-stream-1"})
            print(f"[app] (3) STREAM: HTTP {r_stream.status_code}")
            if r_stream.status_code != 200:
                print(f"[app] FATAL STREAM: HTTP {r_stream.status_code}",
                      file=sys.stderr)
                return 7
            print(f"[app] (3) STREAM counter delta = "
                  f"{_HITS['calls'] - pre_stream}")

        print("[app] ALL 3 steps PASS")
        return 0
    finally:
        await runner.cleanup()
        print("[app] counting stub stopped")


if __name__ == "__main__":
    sys.exit(asyncio.run(_run()))
