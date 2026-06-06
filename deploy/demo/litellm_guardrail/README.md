# `DEMO_MODE=litellm_guardrail` (COV_D11 SLICE 6)

Demo bundle that proves the **NEW guardrail-registry path**
(`SpendGuardGuardrail` composing `_LoopBoundCallback` via the SLICE 5
factory) gates LiteLLM proxy calls **before** the upstream OpenAI HTTP
request leaves the proxy, with a hard-cap deny short-circuit.

This is distinct from the legacy callback demo (`DEMO_MODE=litellm_real`,
`deploy/demo/litellm_proxy/`), which exercises the
`litellm_settings.callbacks` `CustomLogger` surface. Both paths must
keep working (review-standards §6.8).

## Files

| Path | Purpose |
|------|---------|
| `proxy_config.yaml` | LiteLLM `guardrails:` registry entry pointing at `spendguard.integrations.litellm_guardrail.spendguard_guardrail_factory` |
| `spendguard_guardrail_resolver.py` | Triple-factory (`resolver` / `estimator` / `reconciler`) loaded via SLICE 4b `SPENDGUARD_RESOLVER_MODULE` path |
| `README.md` | This file |

## Bring-up

```bash
make demo-up DEMO_MODE=litellm_guardrail
```

The demo container (`spendguard-demo`):

1. Boots an in-process counting HTTP provider on `127.0.0.1:8765/v1`
   (OpenAI-shaped responses with non-zero `usage.completion_tokens`).
2. Spawns the LiteLLM proxy subprocess (`python -m litellm.proxy.proxy_cli`)
   with `--config proxy_config.yaml`. The proxy boot resolves the
   guardrail factory and binds the resolver triple.
3. Issues 3 HTTP calls:
   - ALLOW: small message within budget → HTTP 200, counter +1.
   - DENY: `spendguard_estimate_override=2000000000` blows past the
     seeded 1B hard-cap → HTTP 4xx, counter unchanged.
   - STREAM: `stream=True` within budget → HTTP 200, counter +1,
     end-of-stream commit reconciles real token usage.
4. Asserts ledger rows (`verify_step_litellm_guardrail.sql`) and
   canonical_events `spendguard.integration='litellm'` enrichment
   (Makefile cross-DB block) after the outbox forwarder drains.

## Gates

Each gate is fail-loud (exit 7 on failure):

- ALLOW counter increment (+1).
- DENY counter unchanged (negative control; INV-1 strict-order proof).
- STREAM counter increment (+1).
- Ledger: `reserve >= 2`, `commit_estimated >= 2`, `denied_decision >= 1`.
- canonical_events: `decision >= 2`, `outcome >= 1`, `integration='litellm'`.

## Success line

```
[demo] litellm_guardrail ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

The literal text is review-standards §6.7 LOCKED — automation
(e.g. CI grep) depends on the exact spelling.
