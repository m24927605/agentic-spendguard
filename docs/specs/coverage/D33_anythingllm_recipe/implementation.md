# D33 — AnythingLLM Custom Base URL Recipe — `implementation.md`

> Status: Doc-first spec; per-slice file-level plan.
> Sibling docs: `design.md` (scope and decisions), `tests.md`, `acceptance.md`, `review-standards.md`.
> Audience: Technical Writer implementer; R1-R5 reviewer for layout / file-touch sanity.

---

## 1. Overview

D33 ships a single recipe page plus a minimal Docker smoke. The page lives at `/docs/drop-in/anythingllm/` on the Starlight site (replacing the D03 SLICE 1 stub). The smoke lives alongside the existing `proxy_smoke.sh` in `deploy/demo/`, called from a new `DEMO_MODE=anythingllm_real` target in `deploy/demo/Makefile`. The recipe page links to the smoke as the "verify end-to-end" call-out.

No new SpendGuard service / contract / migration. No `package.json` / `Cargo.toml` change. The only new runtime dependency is the pinned `mintplexlabs/anythingllm:1.8.4` image, added to the demo compose only when the `anythingllm_real` mode boots.

---

## 2. File layout (post-Slice 1)

```
docs/site-v2/
  src/content/docs/docs/drop-in/
    anythingllm.md                                            # REPLACES D03 SLICE 1 stub
docs/specs/coverage/D33_anythingllm_recipe/
  design.md                                                   # this spec set
  implementation.md                                           # this doc
  tests.md
  acceptance.md
  review-standards.md
  citations/
    anythingllm-custom-openai-base-url.pdf                    # snapshot of upstream docs (Slice 1)
  screenshots/                                                # Slice 2 only (optional)
    01-llm-preference-before.png
    02-generic-openai-config.png
deploy/demo/
  Makefile                                                    # MODIFIED Slice 1 (adds anythingllm_real)
  compose.yaml                                                # MODIFIED Slice 1 (adds anythingllm + anythingllm-smoke services)
  anythingllm_smoke.sh                                        # NEW Slice 1
  verify_step_anythingllm_real.sql                            # NEW Slice 1
docs/slices/
  COV_<seq>_d33_recipe_and_smoke.md                           # NEW Slice 1 slice doc
  COV_<seq+1>_d33_screenshots.md                              # NEW Slice 2 slice doc (optional)
```

---

## 3. Page skeleton (target output of Slice 1)

Canonical structure for `docs/site-v2/src/content/docs/docs/drop-in/anythingllm.md`. Bracketed `[ … ]` blocks are author instructions and must not appear in the shipped file.

```markdown
---
title: "AnythingLLM + SpendGuard (drop-in via Generic OpenAI provider)"
description: >-
  Point AnythingLLM's Generic OpenAI provider at a running SpendGuard egress
  proxy. Every chat is reserved before the call and committed after; no
  AnythingLLM code change, no fork, no plugin install.
---

> [Hero — one sentence: AnythingLLM ships with a Generic OpenAI provider
>  that accepts a custom base URL; SpendGuard exposes an OpenAI-compatible
>  endpoint at `/v1/chat/completions`; setting one field puts SpendGuard
>  in the request path. Five-minute setup, two screenshots, one smoke.]

## Prerequisites

- AnythingLLM running (Docker, Desktop, or self-hosted).
- A running SpendGuard egress proxy reachable from AnythingLLM. The
  fastest path is `make demo-up DEMO_MODE=proxy` from a clone of the
  SpendGuard repo; production deployments use the
  [Helm chart](../../deployment/helm/).
- One real OpenAI / Anthropic / Bedrock / Vertex / Azure OpenAI key
  (SpendGuard forwards to the real upstream).

## Step 1: open the LLM Preference panel

> [Screenshot 1 placeholder — Slice 2 fills in. Until then, the page
>  ships with the screenshot omitted but the navigation step described
>  in prose.]

In AnythingLLM's left rail, click **Settings → LLM Preference**. The
panel shows a grid of provider tiles.

## Step 2: pick the Generic OpenAI provider

Scroll to **Generic OpenAI** (not "OpenAI" — that one targets
`api.openai.com` directly and cannot be retargeted). Click it.

## Step 3: fill in the four fields

| Field | Value | Notes |
|---|---|---|
| **Base URL** | `http://localhost:9000/v1` | Or your SpendGuard egress proxy URL. Trailing `/v1` required — AnythingLLM appends `/chat/completions`. |
| **API Key** | `sk-anythingllm-spendguard` | Any non-empty string. SpendGuard ignores this and forwards `Authorization` from its own egress-proxy config to the real upstream. |
| **Chat Model Name** | `gpt-4o-mini` | The model identifier the real upstream expects. SpendGuard does not rewrite this. |
| **Token context window** | `128000` | AnythingLLM-side rate-limit hint only; SpendGuard's contract decides reservation size. |

Click **Save Settings**.

## Step 4: send a test chat

Open any Workspace and send a message. AnythingLLM POSTs to
SpendGuard's `/v1/chat/completions`; SpendGuard calls the contract,
reserves the predicted tokens, forwards to OpenAI, and on response
commits the actual usage.

## Verify end-to-end

```bash
# From a clone of github.com/m24927605/agentic-spendguard:
export OPENAI_API_KEY=sk-...
make demo-up DEMO_MODE=anythingllm_real
```

The smoke boots SpendGuard + AnythingLLM, configures the provider
through AnythingLLM's `/api/v1/system/update-env` endpoint, sends one
chat, and asserts the audit chain in Postgres shows one `reserve` row
and one `commit_estimated` row for the call. Successful run prints
`[anythingllm-smoke] OK: reserve+commit verified`.

## Deployment notes

### Docker

The Docker image is `mintplexlabs/anythingllm:1.8.4`. Point its
`STORAGE_DIR` at a host volume so settings persist across restarts.

### Desktop

AnythingLLM Desktop is GUI-only. Follow steps 1-4 above; the
"Verify end-to-end" smoke does not apply (Desktop has no API). To
confirm SpendGuard sees the call, open the SpendGuard dashboard
(`http://localhost:8081`) and watch the audit feed during your next
chat.

### AnythingLLM Cloud

AnythingLLM Cloud cannot point at a `localhost` SpendGuard. Deploy
SpendGuard with [Helm](../../deployment/helm/) and use the public
Kubernetes Service URL as the Base URL.

## Gotchas

- **Trailing `/v1` is mandatory.** AnythingLLM appends
  `/chat/completions` to your Base URL. Omitting `/v1` gives
  `http://localhost:9000/chat/completions` which the egress proxy
  does not match.
- **Generic OpenAI, not OpenAI.** The `OpenAI` tile in the provider
  grid does not expose a Base URL field. The `Generic OpenAI` tile is
  the only one that does.
- **AnythingLLM's `OpenAiKey` is sent verbatim.** SpendGuard's egress
  proxy ignores the inbound `Authorization` header and substitutes the
  upstream credential from its own config. The AnythingLLM-side key
  exists only to satisfy AnythingLLM's "field non-empty" validation.
- **Streaming works.** AnythingLLM uses Server-Sent Events for
  streaming responses; the SpendGuard egress proxy passes SSE through
  unchanged and commits on the terminating `[DONE]` event.

## What next

- [Set a real budget](../../deployment/helm/)
- [Open the SpendGuard dashboard](../../operations/dashboard/)
- [Cover the rest of your stack (Pattern 3 install)](../../install/)
- Back to the [drop-in landing](../)

**Maintainer docs:** [AnythingLLM — Custom OpenAI Base URL](https://docs.anythingllm.com/llm-configuration/custom-openai-base-url)
```

---

## 4. Demo wiring (Slice 1)

### 4.1 `deploy/demo/Makefile` — add `anythingllm_real` mode

After the existing `agent_real_openai_agents_proxy` block, insert:

```makefile
else ifeq ($(DEMO_MODE),anythingllm_real)
	@echo "[demo] DEMO_MODE=anythingllm_real → postgres + ledger + canonical-ingest + sidecar + egress-proxy + AnythingLLM."
	@echo "[demo] Boots mintplexlabs/anythingllm:1.8.4, configures Generic OpenAI provider"
	@echo "[demo]   pointing at egress-proxy, sends one chat, asserts reserve+commit."
	@test -n "$$OPENAI_API_KEY" || (echo "[demo] FATAL: OPENAI_API_KEY required for DEMO_MODE=anythingllm_real" >&2; exit 8)
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar egress-proxy anythingllm
```

And in the post-bring-up smoke dispatcher:

```makefile
else ifeq ($(DEMO_MODE),anythingllm_real)
	@echo "[demo] running AnythingLLM smoke..."
	$(COMPOSE) run --rm anythingllm-smoke
```

### 4.2 `deploy/demo/compose.yaml` — two new services

```yaml
  anythingllm:
    image: mintplexlabs/anythingllm:1.8.4
    profiles: ["anythingllm_real"]
    environment:
      STORAGE_DIR: /app/server/storage
      SERVER_PORT: "3001"
    ports:
      - "127.0.0.1:3001:3001"
    volumes:
      - anythingllm-storage:/app/server/storage
    depends_on:
      egress-proxy:
        condition: service_healthy
    healthcheck:
      test: ["CMD-SHELL", "wget -qO- http://localhost:3001/api/ping | grep -q online"]
      interval: 5s
      timeout: 3s
      retries: 20

  anythingllm-smoke:
    build:
      context: .
      dockerfile: anythingllm_smoke.Dockerfile
    profiles: ["anythingllm_real"]
    environment:
      ANYTHINGLLM_URL: http://anythingllm:3001
      PROXY_URL: http://egress-proxy:9000
      POSTGRES_URL: postgres://spendguard:spendguard@postgres:5432/spendguard
      OPENAI_API_KEY: ${OPENAI_API_KEY}
    depends_on:
      anythingllm:
        condition: service_healthy
    entrypoint: ["/bin/bash", "/smoke/anythingllm_smoke.sh"]
    volumes:
      - ./anythingllm_smoke.sh:/smoke/anythingllm_smoke.sh:ro
      - ./verify_step_anythingllm_real.sql:/smoke/verify.sql:ro

volumes:
  anythingllm-storage:
```

(The `anythingllm_smoke.Dockerfile` is a four-line `FROM alpine:3.20`
with `curl`, `jq`, `postgresql-client` installed.)

### 4.3 `deploy/demo/anythingllm_smoke.sh`

Modelled on `proxy_smoke.sh`. Five steps:

```bash
#!/bin/bash
# DEMO_MODE=anythingllm_real smoke — AnythingLLM → SpendGuard → OpenAI round-trip.
set -euo pipefail

log() { echo "[anythingllm-smoke] $*" >&2; }
fail() { log "FAIL: $*"; exit 1; }

# Step 0: AnythingLLM /api/ping
curl -sS --max-time 5 "${ANYTHINGLLM_URL}/api/ping" | grep -q online \
    || fail "AnythingLLM not ready"

# Step 1: bootstrap (first-run only) — AnythingLLM 1.8+ requires an
# initial /api/setup-account call before /api/v1 routes accept POSTs.
log "step 1: bootstrap account..."
curl -sS --max-time 10 -X POST "${ANYTHINGLLM_URL}/api/setup-account" \
    -H 'Content-Type: application/json' \
    -d '{"username":"smoke","password":"smoke-pw-1234"}' >/dev/null || true

# Step 2: configure Generic OpenAI provider via /api/v1/system/update-env
log "step 2: configure provider → ${PROXY_URL}/v1..."
curl -sS --max-time 10 -X POST "${ANYTHINGLLM_URL}/api/v1/system/update-env" \
    -H 'Content-Type: application/json' \
    -d "{
        \"LLMProvider\": \"generic-openai\",
        \"GenericOpenAiBasePath\": \"${PROXY_URL}/v1\",
        \"GenericOpenAiKey\": \"sk-anythingllm-spendguard\",
        \"GenericOpenAiModelPref\": \"gpt-4o-mini\",
        \"GenericOpenAiTokenLimit\": 128000
    }" | jq -e '.newValues' >/dev/null || fail "update-env failed"

# Step 3: create a workspace
log "step 3: create workspace..."
WS=$(curl -sS --max-time 10 -X POST "${ANYTHINGLLM_URL}/api/v1/workspace/new" \
    -H 'Content-Type: application/json' \
    -d '{"name":"smoke-ws"}' | jq -r '.workspace.slug')
[ -n "$WS" ] && [ "$WS" != "null" ] || fail "workspace not created"
log "  workspace=${WS}"

# Step 4: send one chat
log "step 4: chat round-trip via SpendGuard..."
RESP=$(curl -sS --max-time 30 -X POST "${ANYTHINGLLM_URL}/api/v1/workspace/${WS}/chat" \
    -H 'Content-Type: application/json' \
    -d '{"message":"Say hi in two words.","mode":"chat"}')
echo "$RESP" | jq -e '.textResponse | length > 0' >/dev/null \
    || fail "no chat response: $RESP"
log "  chat OK"

# Step 5: verify audit chain
log "step 5: verify reserve+commit in ledger..."
psql "${POSTGRES_URL}" -v ON_ERROR_STOP=1 -f /smoke/verify.sql \
    || fail "verify SQL failed"

log "OK: reserve+commit verified"
```

### 4.4 `deploy/demo/verify_step_anythingllm_real.sql`

Modelled on `verify_step_litellm_real.sql`. Asserts:

```sql
\set ON_ERROR_STOP on
SELECT operation_kind, COUNT(*) AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN ('reserve','commit_estimated')
   AND event_time > now() - interval '10 minutes'
 GROUP BY operation_kind
 ORDER BY operation_kind \gset
-- Fail-closed: require both rows
DO $$
BEGIN
  IF (SELECT COUNT(*) FROM ledger_transactions
        WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
          AND operation_kind = 'reserve'
          AND event_time > now() - interval '10 minutes') < 1 THEN
    RAISE EXCEPTION 'no reserve row';
  END IF;
  IF (SELECT COUNT(*) FROM ledger_transactions
        WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
          AND operation_kind = 'commit_estimated'
          AND event_time > now() - interval '10 minutes') < 1 THEN
    RAISE EXCEPTION 'no commit_estimated row';
  END IF;
END $$;
```

---

## 5. Per-slice file plan

### 5.1 Slice 1 — Recipe page + smoke (`COV_<seq>_d33_recipe_and_smoke`)

**Adds:** `anythingllm.md` (replaces D03 stub), `anythingllm_smoke.sh`, `verify_step_anythingllm_real.sql`, `citations/anythingllm-custom-openai-base-url.pdf`, slice doc.

**Modifies:** `deploy/demo/Makefile` (adds `anythingllm_real` mode), `deploy/demo/compose.yaml` (adds `anythingllm` + `anythingllm-smoke` services + volume). D03 row 10 `Verified` column promoted `Spec → Live` in the same slice.

**Build / verification (per `tests.md`):** Astro build green; `DEMO_MODE=anythingllm_real make demo-up` exits 0; smoke prints `OK: reserve+commit verified`; upstream docs link-check passes.

### 5.2 Slice 2 — Screenshots + UX polish (optional)

**Adds:** Two PNGs under `screenshots/`. Inline references inserted into the page where the Slice-1 prose said "Screenshot N placeholder".

**Modifies:** `anythingllm.md` — replace placeholder prose with embedded screenshot syntax + caption.

**Verification:** Visual diff between Slice 1 baseline and Slice 2 matches the documented change set.

Descope rule: Slice 2 is skipped automatically if Slice 1 ships under 400 LOC AND the R1 reviewer flags Slice 2 as low marginal value (page is already self-sufficient from prose).
