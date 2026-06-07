# SpendGuard - Dify Model Provider Plugin

Dify Model Provider Plugin that gates every Dify LLM call (chat-apps,
agents, workflow steps, RAG retrieval steps that use an LLM) through a
SpendGuard sidecar before forwarding to the chosen upstream provider
(OpenAI, Anthropic, Gemini, Bedrock).

## Architecture

```
Dify core ─ ModelManager ─ plugin RPC bus
                              │
                              ▼
            spendguard-dify-plugin daemon container
              SpendGuardLLM._invoke(...)
                1. Build BudgetBinding from credentials
                2. _DifyReservation.reserve()       (UDS + mTLS)
                3. Forward to upstream provider     (OpenAI etc.)
                4. _DifyReservation.commit_success() with real usage
                5. On error: _DifyReservation.release_failure()
```

The plugin daemon runs in its own container per Dify v1 contract; Dify
core never sees the upstream provider directly — clean MITM point, no
header rewriting required in Dify core.

## v1 scope

- OpenAI upstream is wired in SLICE 4.
- Anthropic upstream + ``get_num_tokens`` via sidecar `count_tokens`
  land in SLICE 5.
- Streaming via end-of-stream commit lands in SLICE 6.
- `validate_credentials` performs a 1-token reserve+release roundtrip
  against the sidecar to prove SpendGuard wiring at install time
  (SLICE 3).

## Environment

The plugin daemon container reads:

| Var | Required | Purpose |
|-----|----------|---------|
| `SPENDGUARD_SIDECAR_UDS` | yes | UDS path to the sidecar (overridden per-call by `credentials.spendguard_sidecar_address`) |
| `SPENDGUARD_TENANT_ID` | yes | tenant identifier asserted at handshake |
| `SPENDGUARD_DIFY_FAIL_OPEN` | no | dev escape: `1` to allow calls through on sidecar errors (NOT for production) |

## Install (self-hosted Dify, sideload)

```bash
# inside the plugin source tree
python -m dify_plugin.cli plugin package -o spendguard.difypkg

# upload via Dify admin UI → Marketplace → Install Local Plugin
```

## Install (Dify Cloud, marketplace)

```bash
# operator side
dify plugin install spendguard
```

(Marketplace push automation lands in SLICE 8.)

## Configuration

Once installed, the operator opens the Dify model-provider list, selects
"SpendGuard", and supplies:

- Upstream Provider (OpenAI, Anthropic, Gemini, Bedrock)
- Upstream API Key
- (optional) Upstream Base URL override
- SpendGuard Budget ID
- SpendGuard Window Instance ID

`validate_credentials` runs a `RequestDecision` → release roundtrip
against the sidecar so install fails closed if SpendGuard wiring is
broken.

## Development

```bash
cd plugins/dify/spendguard
python3.12 -m venv .venv
source .venv/bin/activate
pip install -e ".[test,anthropic]"
pytest tests/ -v
ruff check .
```

## Reference

See `docs/specs/coverage/D10_dify_plugin/` for the spec set:

- `design.md` - architecture, key decisions, slice plan
- `implementation.md` - module layout, code skeleton
- `acceptance.md` - hard gates G1..G13, invariants INV-1..INV-9
- `review-standards.md` - reviewer checklists per slice

Adapter contract lineage: this plugin mirrors the reserve/commit/release
lifecycle of `sdk/python/src/spendguard/integrations/litellm.py`
(LiteLLM proxy CustomLogger), translated to the Dify SDK signature.
Composition over inheritance: `_DifyReservation` owns the SpendGuard
lifecycle, `SpendGuardLLM` only adapts the Dify SDK contract.
