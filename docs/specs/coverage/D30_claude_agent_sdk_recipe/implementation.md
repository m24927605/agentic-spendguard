# D30 — Anthropic claude-agent-sdk Egress-Proxy Install Recipe — `implementation.md`

> Status: Doc + smoke spec; lands before any slice.
> Sibling docs: `design.md`, `tests.md`, `acceptance.md`, `review-standards.md`.
> Audience: Technical Writer implementer; reviewer for file-touch sanity.

---

## 1. Overview

This document is the per-slice implementation plan for D30. It maps `design.md` §4 (slicing) to concrete file paths, script skeletons, and demo wiring. Two slices: Slice 1 ships the doc page + Python smoke + demo mode + verify SQL; Slice 2 ships the TS smoke + CI workflow + the doc cross-link to the TS example. No new SpendGuard code anywhere — D30 sits entirely above existing surfaces (`services/egress_proxy/src/routing.rs`, the sidecar audit chain, D02's CA install).

## 2. File layout (post-Slice 2)

```
docs/site/docs/integrations/
  claude-agent-sdk.md                                       # NEW Slice 1

examples/claude-agent-sdk-egress/
  README.md                                                 # NEW Slice 1
  python/
    pyproject.toml                                          # NEW Slice 1
    smoke.py                                                # NEW Slice 1
    verify_audit.py                                         # NEW Slice 1 (shared)
  typescript/
    package.json                                            # NEW Slice 2
    smoke.mjs                                               # NEW Slice 2
    README.md                                               # NEW Slice 2 (Node prereqs)

deploy/demo/
  Makefile                                                  # MODIFIED Slice 1 (new DEMO_MODE arm)
  verify_step_claude_agent_sdk_egress.sql                   # NEW Slice 1
  demo/run_demo.py                                          # MODIFIED Slice 1 (dispatcher arm)

.github/workflows/
  d30-claude-agent-sdk-smoke.yml                            # NEW Slice 2

docs/specs/coverage/D30_claude_agent_sdk_recipe/
  design.md
  implementation.md
  tests.md
  acceptance.md
  review-standards.md

docs/internal/slices/
  COV_<N>_d30_doc_and_python_smoke.md                       # NEW Slice 1
  COV_<N+1>_d30_typescript_smoke.md                         # NEW Slice 2
```

## 3. Doc page skeleton — `docs/site/docs/integrations/claude-agent-sdk.md`

Canonical structure for Slice 1. Square-bracket placeholders are author instructions; they must NOT appear in the shipped file.

```markdown
# Anthropic claude-agent-sdk

> SpendGuard gates Anthropic's `claude-agent-sdk` via the egress proxy + a
> trusted root CA — not through an SDK callback. **`PreToolUse` is
> tool-scope, not LLM-scope. The egress proxy is the only LLM-scope gate
> for this SDK.**

## Why the egress proxy

[~80 words: SDK subprocesses the `claude` CLI; LLM calls happen inside
 the CLI process; no SDK hook sees `messages → response`. Pattern 3 —
 forward HTTPS proxy + customer-installed root CA — is the only path.
 D02's `spendguard install` handles the CA install + `HTTPS_PROXY` set.]

## Prerequisites

- D02 install completed: `spendguard install` ran clean on this host.
- An Anthropic API key in `ANTHROPIC_API_KEY`.
- Python 3.11+ (for the Python SDK) or Node.js 20+ (for the TS SDK).
- A running SpendGuard egress proxy at `http://localhost:9000`.

## Recipe — Python

```bash
pip install claude-agent-sdk
# spendguard install already set HTTPS_PROXY + the per-shell rc snippet;
# verify it's in your current shell:
echo "$HTTPS_PROXY"   # → http://localhost:9000
```

```python
# examples/claude-agent-sdk-egress/python/smoke.py
import anyio
from claude_agent_sdk import query, ClaudeAgentOptions

async def main():
    async for msg in query(
        prompt="List two ways to reverse a string in Python in under 30 words.",
        options=ClaudeAgentOptions(model="claude-sonnet-4-5", max_turns=1),
    ):
        print(msg)

anyio.run(main)
```

Run it; check the SpendGuard dashboard or query `audit_outbox` to see
the matching `RESERVE_RESPONSE` + `COMMIT_OUTCOME` rows.

## Recipe — TypeScript

```bash
npm install @anthropic-ai/claude-agent-sdk
echo $HTTPS_PROXY            # → http://localhost:9000
echo $NODE_EXTRA_CA_CERTS    # → <path D02 installed>
```

```ts
// examples/claude-agent-sdk-egress/typescript/smoke.mjs
import { query } from "@anthropic-ai/claude-agent-sdk";

for await (const msg of query({
  prompt: "List two ways to reverse a string in Python in under 30 words.",
  options: { model: "claude-sonnet-4-5", maxTurns: 1 },
})) {
  console.log(msg);
}
```

## Verifying SpendGuard captured the call

[~120 words: link to dashboard; `psql -c "SELECT … FROM audit_outbox …"`
 example; what `provider`, `model`, `request_id`, and committed token
 columns to look for; reference verify SQL by path.]

## What `PreToolUse` is — and is not

[~120 words: explain that `PreToolUse` fires on tool invocations (Bash,
 Edit, Read); a budget cap registered there only fires when the agent
 hits a tool, not when the LLM is queried. Customers wanting LLM-scope
 gating MUST use the egress proxy. Cross-link to the strategy memo
 Pattern 3 row for claude-agent-sdk.]

## Troubleshooting

[Bullet list: certificate-verify errors → re-run `spendguard install`;
 `HTTPS_PROXY` set but calls still hit anthropic directly → check
 `NODE_EXTRA_CA_CERTS` on Node; per-shell rc not loaded → restart shell.]

## Next steps

- [D02 install](./install/) — base install if you have not run it.
- [Cover the rest of your stack](./drop-in/) — Pattern 2 quick wins.
- [Open an audit row in the dashboard](./operations/dashboard/).
```

## 4. Smoke script — Python (`examples/claude-agent-sdk-egress/python/smoke.py`)

Slice 1 ships an end-to-end script:

```python
"""SpendGuard egress smoke for claude-agent-sdk (Python).

Runs one canonical prompt through the SDK with HTTPS_PROXY pointing at
the SpendGuard egress proxy, then runs verify_audit.py to assert the
reserve + commit rows landed.
"""
import os
import sys
import anyio
from claude_agent_sdk import query, ClaudeAgentOptions
from verify_audit import assert_reserve_and_commit  # noqa: E402

PROMPT = "List two ways to reverse a string in Python in under 30 words."

async def _run() -> str:
    if not os.environ.get("ANTHROPIC_API_KEY"):
        print("FATAL: ANTHROPIC_API_KEY required", file=sys.stderr)
        sys.exit(8)
    if not os.environ.get("HTTPS_PROXY"):
        print("FATAL: HTTPS_PROXY required (run `spendguard install`)", file=sys.stderr)
        sys.exit(9)

    request_id = os.environ.get("SPENDGUARD_REQUEST_ID")  # optional pin
    text_chunks = []
    async for msg in query(
        prompt=PROMPT,
        options=ClaudeAgentOptions(model="claude-sonnet-4-5", max_turns=1),
    ):
        text_chunks.append(repr(msg))
    return "\n".join(text_chunks)

def main() -> int:
    out = anyio.run(_run)
    print(f"[smoke] SDK output ({len(out)} bytes captured)")
    print("[smoke] verifying audit chain...")
    assert_reserve_and_commit(provider="anthropic", model_prefix="claude-")
    print("[smoke] PASS")
    return 0

if __name__ == "__main__":
    sys.exit(main())
```

`verify_audit.py` (shared by Python + TS smoke; TS invokes via subprocess) reads `DATABASE_URL` (defaulted to the demo Postgres URL inside compose) and runs `verify_step_claude_agent_sdk_egress.sql` via `psycopg`. Exits 0 on PASS, non-zero with the failed assertion line on FAIL.

## 5. Smoke script — TypeScript (`examples/claude-agent-sdk-egress/typescript/smoke.mjs`)

Slice 2 mirror of §4 in raw ESM Node — see canonical TS snippet in §3 above. After the SDK loop, the script `child_process.spawnSync` calls `python ../python/verify_audit.py` to run the same SQL assertion. No second copy of the verification logic.

## 6. Demo wiring — `deploy/demo/Makefile` arm

```makefile
else ifeq ($(DEMO_MODE),agent_real_claude_agent_sdk_egress)
	@echo "[demo] DEMO_MODE=agent_real_claude_agent_sdk_egress →"
	@echo "[demo]   Pattern 3 verification for claude-agent-sdk (Python)."
	@echo "[demo]   Same stack as proxy mode; demo container runs the"
	@echo "[demo]   ACTUAL claude-agent-sdk against HTTPS_PROXY=egress-proxy."
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar egress-proxy
```

…then in the dispatch block:

```makefile
else ifeq ($(DEMO_MODE),agent_real_claude_agent_sdk_egress)
	@echo "[demo] running claude-agent-sdk smoke against the proxy..."
	@test -n "$$ANTHROPIC_API_KEY" || (echo "[demo] FATAL: ANTHROPIC_API_KEY required" >&2; exit 8)
	$(COMPOSE) build demo
	$(COMPOSE) run --rm --no-deps \
	    --env SPENDGUARD_DEMO_MODE=agent_real_claude_agent_sdk_egress \
	    --env ANTHROPIC_API_KEY=$$ANTHROPIC_API_KEY \
	    --env HTTPS_PROXY=http://egress-proxy:9000 \
	    --env NODE_EXTRA_CA_CERTS=/etc/ssl/spendguard/ca.crt \
	    demo
	@echo "[demo] verifying audit chain..."
	@$(MAKE) demo-verify-claude-agent-sdk-egress
	@echo "[demo] PASS — claude-agent-sdk recipe verified via Pattern 3"
```

`demo-verify-claude-agent-sdk-egress` is a new target that wraps `psql -f verify_step_claude_agent_sdk_egress.sql`.

## 7. Demo dispatcher (`deploy/demo/demo/run_demo.py`)

Slice 1 adds an arm to the dispatcher (around line 820, alongside `agent_real_openai_agents_proxy`):

```python
if DEMO_MODE == "agent_real_claude_agent_sdk_egress":
    return await run_claude_agent_sdk_egress_mode()
```

Plus a new `run_claude_agent_sdk_egress_mode()` that is structurally a 1:1 mirror of `run_openai_agents_proxy_mode()` (lines 822-870 in the existing file) with the SDK swapped to `claude_agent_sdk`. The same exit-code semantics (8 = missing key, 9 = SDK error, 0 = OK).

## 8. Verify SQL — `deploy/demo/verify_step_claude_agent_sdk_egress.sql`

Asserts the three rows from `design.md` §D-5 inside a `DO $$ … $$` block; raises `EXCEPTION` on any miss. Reuses the column names + ENUM values from `verify_step8.sql` so the schema bar is identical.

## 9. CI workflow — `.github/workflows/d30-claude-agent-sdk-smoke.yml`

Slice 2 adds a workflow that runs the TS smoke on push to slice branches:

| Job | Steps | Hard gate |
|---|---|---|
| `python-smoke` | Start compose stack with `DEMO_MODE=agent_real_claude_agent_sdk_egress`; assert exit 0 | Yes |
| `typescript-smoke` | Start compose stack `DEMO_MODE=proxy`; `cd examples/claude-agent-sdk-egress/typescript && npm ci && node smoke.mjs` | Yes |
| `doc-link-check` | `lychee docs/site/docs/integrations/claude-agent-sdk.md` | Yes |

Path filter: `examples/claude-agent-sdk-egress/**`, `docs/site/docs/integrations/claude-agent-sdk.md`, `deploy/demo/verify_step_claude_agent_sdk_egress.sql`, `.github/workflows/d30-claude-agent-sdk-smoke.yml`.

## 10. Reviewer-facing reading order

1. `design.md` §1 (scope) and §3 (decisions).
2. This doc §3 (page skeleton) — confirms shape.
3. The actual page diff in `docs/site/docs/integrations/claude-agent-sdk.md`.
4. `tests.md` §3 (smoke + verify SQL).
5. `acceptance.md` §2 (ship gates).
