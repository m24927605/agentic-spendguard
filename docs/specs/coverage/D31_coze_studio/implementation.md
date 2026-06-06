# D31 — Coze Studio Model Provider — Implementation

**Reads:** [`design.md`](design.md), [`acceptance.md`](acceptance.md), [`review-standards.md`](review-standards.md).
**Touches:** New recipe tree under `examples/coze-studio/` + demo orchestration overlay + public docs page + README row. **No Rust changes. No proto changes. No DB schema changes. No new sidecar endpoint.**

## §1. Module layout

```
examples/coze-studio/                                  # NEW — recipe tree (Slice 1)
├── README.md                                          # operator-facing how-to
├── coze-workspace-config.yaml                         # OpenAI-provider config snippet for Coze workspace
├── headers-cheatsheet.md                              # X-SpendGuard-* header reference
├── docker-compose.coze.yaml                           # Slice 2 + 3 — Coze Studio stack overlay (pinned digest)
├── smoke.sh                                           # Slice 2 — curl-driven smoke test
└── client.py                                          # Slice 3 — demo driver entry (chat-flow exercise)

deploy/demo/                                           # Slice 3 wiring (additive)
├── Makefile                                           # +DEMO_MODE=coze_studio_real branch
├── coze_studio/
│   ├── compose.override.yaml                          # symlink or copy of examples/coze-studio/docker-compose.coze.yaml
│   ├── seed_workspace.sql                             # seeds the SpendGuard ledger budget + tenant for the workspace
│   └── README.md                                      # demo-mode-specific notes
└── verify_step_coze_studio_real.sql                   # SQL gate

docs/site/docs/integrations/                          # Slice 4
└── coze-studio.md                                     # NEW — public docs page

README.md                                              # Slice 4 — one row added to adapter integrations table
```

No edits anywhere under `services/`, `crates/`, `sdk/`, or existing demo modes' compose files. D31 is purely additive at the recipe + docs + demo-overlay layer.

## §2. Slice breakdown

### Slice 1 — Coze workspace config snippet (S)

**Files:** `examples/coze-studio/coze-workspace-config.yaml`, `examples/coze-studio/headers-cheatsheet.md`, `examples/coze-studio/README.md`.

`coze-workspace-config.yaml` is the config snippet operators paste into a Coze workspace's "Model Provider → OpenAI" custom-endpoint form. Shape (target schema is Coze Studio's v1 workspace provider config):

```yaml
# Coze Studio workspace → Model Provider → OpenAI (custom endpoint)
# Paste into the workspace YAML, or fill the equivalent web-form fields.
provider: openai
display_name: SpendGuard-Gated OpenAI
base_url: https://spendguard-sidecar.spendguard.svc.cluster.local:8443/v1/openai
# For self-host demo: https://localhost:8443/v1/openai
api_key: ${OPENAI_API_KEY}                  # passed through to SpendGuard → upstream OpenAI
custom_headers:
  X-SpendGuard-Tenant-Id: "<COZE_WORKSPACE_ID>"
  X-SpendGuard-Budget-Id: "<SPENDGUARD_BUDGET_ID>"
  X-SpendGuard-Window-Instance-Id: "<SPENDGUARD_WINDOW_INSTANCE_ID>"
tls:
  ca_cert_path: /etc/coze/spendguard-ca.pem  # SpendGuard sidecar's mTLS root
  client_cert_path: /etc/coze/coze-client.pem
  client_key_path: /etc/coze/coze-client.key
models:
  - id: gpt-4o-mini
  - id: gpt-4o
  - id: gpt-3.5-turbo
```

`headers-cheatsheet.md` documents the three `X-SpendGuard-*` headers, their required formats, and how to extract the right values from a Coze workspace's admin UI.

`README.md` walks an operator through: prereqs (Coze Studio ≥ 1.0, SpendGuard sidecar with HTTP companion enabled), workspace ID extraction, snippet paste location, smoke test command, troubleshooting (mTLS handshake fail → check CA; 502 on every call → check tenant header).

**Acceptance gate (slice-local):** snippet is YAML-valid (`python -c "import yaml; yaml.safe_load(open('examples/coze-studio/coze-workspace-config.yaml'))"`). README mentions all three `X-SpendGuard-*` headers verbatim.

### Slice 2 — HTTP companion smoke (S, depends on D09 SLICE 1)

**Files:** `examples/coze-studio/docker-compose.coze.yaml`, `examples/coze-studio/smoke.sh`.

`docker-compose.coze.yaml` boots a minimal stack: Coze Studio (image pinned by SHA256 digest), Postgres (Coze's metadata store), Redis (Coze's job queue), SpendGuard sidecar with `--http-companion-bind 0.0.0.0:8443` flag, and ledger DB.

```yaml
services:
  coze-studio:
    image: ghcr.io/coze-dev/coze-studio@sha256:<PINNED_DIGEST>
    depends_on: [coze-postgres, coze-redis]
    environment:
      - COZE_DB_HOST=coze-postgres
      - COZE_REDIS_HOST=coze-redis
  coze-postgres:
    image: postgres:16
  coze-redis:
    image: redis:7
  spendguard-sidecar:
    image: spendguard-sidecar:dev
    command:
      - "--http-companion-bind=0.0.0.0:8443"
      - "--http-companion-svid=/run/svid/cert.pem"
    ports: ["8443:8443"]
```

`smoke.sh` drives the smoke:

1. Boot the compose stack, wait for Coze + sidecar healthy.
2. POST a synthetic "test connection" call to the sidecar `/v1/openai/chat/completions` with the snippet's headers — mimics what Coze sends on the workspace "Test connection" button.
3. Assert HTTP 200, assert the response body has an OpenAI-shaped completion (real upstream OpenAI is hit; `OPENAI_API_KEY` required).
4. Query the ledger DB: assert a `PRE_LLM_CALL.RESERVE` row and a `LLM_CALL_POST.SUCCESS` row exist with matching `reservation_id` and `integration='coze_studio'` in `decision_context`.
5. Tear down.

**Slice-local acceptance:** `bash examples/coze-studio/smoke.sh` exits 0 on a clean checkout with `OPENAI_API_KEY` exported. Sidecar audit row visible.

### Slice 3 — Demo mode (M)

**Files:** `deploy/demo/Makefile`, `deploy/demo/coze_studio/compose.override.yaml`, `deploy/demo/coze_studio/seed_workspace.sql`, `deploy/demo/coze_studio/README.md`, `deploy/demo/verify_step_coze_studio_real.sql`, `examples/coze-studio/client.py`.

`Makefile` adds a branch:

```makefile
else ifeq ($(DEMO_MODE),coze_studio_real)
	@echo "[demo] DEMO_MODE=coze_studio_real → postgres + ledger + sidecar (HTTP companion) + Coze Studio + real OpenAI"
	@test -n "$$OPENAI_API_KEY" || (echo "[demo] FATAL: OPENAI_API_KEY required for DEMO_MODE=coze_studio_real" >&2; exit 8)
	$(COMPOSE) -f compose.yaml -f coze_studio/compose.override.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest tokenizer sidecar \
	    coze-postgres coze-redis coze-studio coze-seed
```

`compose.override.yaml` mirrors `examples/coze-studio/docker-compose.coze.yaml` but reuses the demo's shared Postgres + sidecar containers (a single Postgres serves both `spendguard_ledger` and `coze_db` databases). The override sets the sidecar's `--http-companion-bind` flag and exposes port 8443 on the demo network so Coze can reach it.

`seed_workspace.sql` (run by a one-shot `coze-seed` init container) seeds:
- One SpendGuard budget + window-instance for the demo workspace.
- One Coze workspace + project + chat-flow pointing at the SpendGuard provider snippet.

`client.py` is the demo driver step. It POSTs against Coze's chat-flow run API for the seeded workspace, exercising 3 phases:

1. **ALLOW**: small prompt fits the budget. Expect Coze HTTP 200, expect SpendGuard audit row with reserve + commit, expect real OpenAI hit (visible in audit's `provider_event_id`).
2. **DENY**: a synthetic large-prompt projection (or pre-exhausted budget) triggers DENY. Expect Coze to surface the 502 from the companion as a workflow error; expect SpendGuard DENY decision audited; **expect zero hits** at the upstream OpenAI sentinel (a counting wrapper sits in front of `api.openai.com` in the demo).
3. **STREAMING**: Coze chat-flow with streaming response. Expect 200 SSE chunks reach Coze, end-of-stream commit fires in SpendGuard audit chain.

`verify_step_coze_studio_real.sql` (SQL gate, mirrors `verify_step_litellm_real.sql` shape):

```sql
-- D31_COZE: at least 2 decisions tagged integration=coze_studio (1 ALLOW + 1 DENY).
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'coze_studio'
     AND created_at > now() - interval '5 minute';
  IF c < 2 THEN RAISE EXCEPTION 'D31_COZE_GATE: expected >=2, got %', c; END IF;
END; $$;

-- D31_COZE: at least one DENY.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'coze_studio'
     AND cloudevent_payload::jsonb->'data'->>'decision' = 'DENY';
  IF c < 1 THEN RAISE EXCEPTION 'D31_COZE_GATE: no DENY'; END IF;
END; $$;

-- D31_COZE: commit row present for ALLOW path.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM commits
   WHERE latest_state IN ('estimated', 'provider_reported')
     AND created_at > now() - interval '5 minute';
  IF c < 1 THEN RAISE EXCEPTION 'D31_COZE_GATE: no commit'; END IF;
END; $$;

-- D31_COZE: streaming step produced an end-of-stream commit row.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'coze_studio'
     AND decision_context->>'stream' = 'true';
  IF c < 1 THEN RAISE EXCEPTION 'D31_COZE_GATE: no streaming row'; END IF;
END; $$;

-- D31_COZE: upstream stub counter unchanged across DENY decisions.
DO $$ DECLARE bad INT; BEGIN
  SELECT COUNT(*) INTO bad FROM audit_outbox
   WHERE decision_context->>'integration' = 'coze_studio'
     AND cloudevent_payload::jsonb->'data'->>'decision' = 'DENY'
     AND (decision_context->>'stub_hits')::int > 0;
  IF bad > 0 THEN RAISE EXCEPTION 'D31_COZE_GATE: % DENY decisions saw upstream', bad; END IF;
END; $$;

-- D31_COZE: canonical_events received the coze events (forwarder ran).
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM canonical_events WHERE source_integration = 'coze_studio';
  IF c < 1 THEN RAISE EXCEPTION 'D31_COZE_GATE: canonical_events empty'; END IF;
END; $$;

-- D31_COZE: audit chain hash continuity intact.
SELECT spendguard_verify_chain('coze_studio_real') AS chain_intact;
```

### Slice 4 — Docs page (S)

**Files:** `docs/site/docs/integrations/coze-studio.md`, `README.md`.

The docs page covers:

- **Why SpendGuard for Coze Studio** — what's broken without it (no budget gate, no signed audit), what SpendGuard adds.
- **Install (self-hosted Coze)** — workspace YAML snippet + cert bootstrap + verify steps. Includes verbatim the snippet from `examples/coze-studio/coze-workspace-config.yaml`.
- **Decision matrix** — when to use D31 base-URL vs D02 egress-proxy. Coze-specific guidance: "If you only run Coze, D31 is simpler. If you also run terminal CLIs and other apps from the same pod/VM, D02 covers both with one install."
- **Limitations** — Coze Cloud not supported, Anthropic/Gemini/Bedrock slots not in v1, no mid-stream cap, no native Coze plugin SDK route (tracked as v1.1).
- **Troubleshooting** — common errors (mTLS chain, tenant header missing, Coze workspace ID extraction).

`README.md` adapter integrations table gains one row:

```
| Coze Studio | OpenAI-compatible base URL | See [examples/coze-studio](examples/coze-studio/README.md) |
```

## §3. Backwards compatibility

| Surface | Action |
|---------|--------|
| Existing `examples/` / `sdk/python/` integrations | Untouched. |
| `compose.yaml` for other demo modes | Unchanged. Coze services live in an overlay file, opt-in per `DEMO_MODE` branch. |
| Existing PyPI extras of `spendguard-sdk` | Unchanged. D31 is recipe-only, no Python package. |
| Existing DB schemas | Unchanged. Coze uses a separate database name (`coze_db`) on the shared Postgres instance. |
| Sidecar binary | Unchanged. HTTP companion already shipped via D09 SLICE 1. |

## §4. Failure modes (must be tested in Slice 3 demo)

| Mode | Expected | Where verified |
|------|----------|----------------|
| Sidecar DENY | Coze sees 502 from companion, Coze surfaces workflow error, no upstream OpenAI hit | demo driver step 2 + verify SQL `stub_hits` |
| Sidecar DEGRADE | Coze sees 503 from companion, Coze surfaces workflow error | manual injection in Slice 2 smoke (`make sidecar-degrade` toggle) |
| Tenant header missing | Companion returns 400, smoke + demo assert error surface | Slice 2 smoke negative case |
| Coze workspace YAML malformed | Coze rejects on Apply, no SpendGuard interaction | Slice 1 acceptance gate (yaml.safe_load) |
| Upstream OpenAI 5xx mid-call | Companion returns the upstream error, SpendGuard `release_failure` fires | Slice 2 smoke + demo driver step 1 retry case |
| Streaming usage missing in upstream final chunk | Companion's existing estimator fallback handles it; commit row uses estimator with WARN | Slice 3 streaming step + verify SQL |

## §5. LOC budget

| File | LOC |
|------|-----|
| `coze-workspace-config.yaml` | ~40 |
| `headers-cheatsheet.md` | ~80 |
| `examples/coze-studio/README.md` | ~150 |
| `docker-compose.coze.yaml` | ~80 |
| `smoke.sh` | ~120 |
| `deploy/demo/coze_studio/compose.override.yaml` | ~60 |
| `seed_workspace.sql` | ~80 |
| `verify_step_coze_studio_real.sql` | ~80 |
| `client.py` | ~180 |
| `docs/site/docs/integrations/coze-studio.md` | ~250 |
| **Total** | **~1120** |

## §6. Out of scope

Everything in design.md §5. Plus: no Coze plugin SDK code, no `services/sidecar/` edits, no new companion endpoint, no Coze Cloud automation, no Coze upstream PR, no Anthropic / Gemini / Bedrock provider slot. Plus: no changes to `sdk/python/src/spendguard/integrations/`.
