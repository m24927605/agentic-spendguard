---
description: >-
  Run Agentic SpendGuard budget control behind the LiteLLM proxy so every
  /v1/chat/completions call is reserved pre-call and committed post-call
  through the SpendGuard ledger — without changing your agent or app code.
---

# LiteLLM integration

> LiteLLM's proxy is the most common way teams put a unified
> `/v1/chat/completions` shape in front of multi-provider model
> traffic. SpendGuard's LiteLLM CustomLogger callback gates every
> request through the SpendGuard ledger: pre-call reserve, post-call
> commit, end-of-stream reconciliation, retry release. Operators
> get hard-cap denies, real-spend audit trails, and per-team
> attribution — without changing their agent code.

## Why you'd want this

- **One-line gate for proxy traffic.** Register a Python module in
  `litellm_settings.callbacks` and every model call routes through
  SpendGuard's reserve → commit lifecycle.
- **Real reconciliation.** End-of-stream reconciler reads
  `response.usage.completion_tokens` and commits the true cost,
  not the worst-case estimator.
- **Multi-tenant via virtual keys.** Resolver inspects
  LiteLLM's `user_api_key_dict.team_id` and dispatches to the
  right `BudgetBinding` per team.
- **Fail-closed by default.** Sidecar unreachable or DEGRADE →
  call denied. Env override (`SPENDGUARD_LITELLM_FAIL_OPEN=1`)
  exists for dev only.

---

## Three paths to add SpendGuard

Pick by how your existing LiteLLM setup is structured.

### Path A — LiteLLM proxy + SpendGuard callback *(recommended for proxy users)*

Your team already runs `litellm --config proxy_config.yaml` and
points apps at `OPENAI_BASE_URL=http://your-proxy:4000/v1`. Add
SpendGuard by registering one module in the proxy config.

1. Install: `pip install 'spendguard-sdk[litellm]'` (pulls
   `litellm[proxy]` transitively).
2. Drop the operator template at
   `docs/specs/litellm-integration/PROXY_RECIPE.md` next to your
   proxy config (or copy the version that ships with the SDK).
3. Add to `proxy_config.yaml`:
   ```yaml
   litellm_settings:
     callbacks: ["spendguard_litellm_proxy_callback.handler_instance"]
   ```
4. Set `PYTHONPATH` to include the directory of the callback
   module, plus the SpendGuard env vars (sidecar UDS, tenant_id,
   pricing-table versions).
5. Launch: `python -m litellm.proxy.proxy_cli --config proxy_config.yaml`.

Everything downstream (ALLOW + DENY + STREAM + multi-team) works
without app changes. See PROXY_RECIPE.md for the full template.

### Path B — Direct `litellm.acompletion()` callers

You're calling `litellm.acompletion(...)` from Python directly,
not through the proxy. In v1, **this surface is not gated by the
SpendGuard callback** (LiteLLM's logging dispatcher swallows
exceptions from sync pre-call hooks, and `async_pre_call_hook` is
proxy-only — verified in ADR-005).

Route async direct callers through Path C below (egress proxy);
sync `litellm.completion()` callers also use Path C. The proxy
callback path (Path A) is the only surface where the SDK
integration's gating is reliable.

### Path C — Shape A: SpendGuard egress proxy *(no LiteLLM proxy required)*

For setups that don't run the LiteLLM proxy (or for sync
`litellm.completion()` callers), point LiteLLM at SpendGuard's
egress proxy and let it gate at the HTTP boundary:

```python
import litellm
litellm.api_base = "http://spendguard-egress-proxy:9000/v1"
litellm.completion(model="gpt-4o-mini", messages=[...])
```

The egress proxy does its own reserve→commit lifecycle against
the same SpendGuard sidecar, with no LiteLLM-side wiring.

---

## Prerequisites (one-time setup)

1. **SpendGuard sidecar running** (see Quickstart). The callback
   talks to it via UDS at `SPENDGUARD_SIDECAR_UDS`.
2. **Operator-supplied resolver/estimator/reconciler.** The
   integration is unopinionated — you wire them in the operator
   callback module (PROXY_RECIPE.md §2.2–2.4).
3. **LiteLLM ≥ 1.50** (the version that ships
   `litellm[proxy]` with the `async_pre_call_hook` shape and
   stable `litellm.integrations.custom_logger.CustomLogger`).

---

## Validation

Quickest path: run the demo.

```bash
make demo-up DEMO_MODE=litellm_real    # 4 steps: ALLOW + DENY + STREAM + MULTI-TEAM
make demo-up DEMO_MODE=litellm_deny    # 3 fail-closed sub-steps
```

`litellm_real` exits 0 with the success line `litellm_real ALL 4
steps PASS (ALLOW + DENY + STREAM + MULTI-TEAM)` AND the SQL gate
`SLICE6/9 LEDGER OK: reserve=N commit_estimated=N` AND
`SLICE6 CANONICAL OK: decision>=1 outcome>=1` AND `SLICE6 DENY OK:
denied_decision>=1`.

`litellm_deny` exits 0 with all 3 sub-steps showing
`counter pre=N post=N` (no increment) for each deny variant AND
`SLICE7 LEDGER OK: reserve>=3 commit_estimated>=3 denied_decision>=1`.

---

## Operational notes

- **TTL tuning.** `SPENDGUARD_LITELLM_TTL_SECONDS=300` (default).
  Tune up for very long streams. Reservations not committed by
  TTL get released by the TTL sweeper.
- **Fail-closed default.** Sidecar offline → call denied. Override
  for dev only via `SPENDGUARD_LITELLM_FAIL_OPEN=1`.
- **Audit join.** Proxy-mode calls write to
  `LiteLLM_SpendLogs`; SpendGuard writes to `canonical_events`.
  Join on `litellm_call_id` for cross-table forensics (DESIGN.md §8.3).

---

## Related

- [Quickstart](../quickstart.md) — full stack up in 5 minutes
- [Contract DSL reference](../contracts/yaml.md) — author allow/stop rules
- Other integrations: [Microsoft AGT](agt.md) · [Pydantic-AI](pydantic-ai.md) · [LangChain & LangGraph](langchain.md) · [OpenAI Agents SDK](openai-agents.md)
