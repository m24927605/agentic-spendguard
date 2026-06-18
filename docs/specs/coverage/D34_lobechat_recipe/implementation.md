# D34 — LobeChat Custom Base URL Recipe — `implementation.md`

> Status: Doc-first spec; per-slice file-level plan.
> Sibling docs: `design.md` (scope and decisions), `tests.md`, `acceptance.md`, `review-standards.md`.
> Audience: Technical Writer implementer; R1-R5 reviewer for layout / file-touch sanity.

---

## 1. Overview

D34 ships a single recipe page plus a minimal Docker smoke. The page lives at `/docs/drop-in/lobechat/` on the Starlight site (replacing the D03 SLICE 1 stub). The smoke lives alongside the existing `proxy_smoke.sh` and `anythingllm_smoke.sh` in `deploy/demo/`, called from a new `DEMO_MODE=lobechat_real` target in `deploy/demo/Makefile`. The recipe page links to the smoke as the "verify end-to-end" call-out.

No new SpendGuard service / contract / migration. No `package.json` / `Cargo.toml` change. The only new runtime dependency is the pinned `lobehub/lobe-chat:1.40.0` image, added to the demo compose only when the `lobechat_real` mode boots.

---

## 2. File layout (post-Slice 1)

```
docs/site-v2/
  src/content/docs/docs/drop-in/
    lobechat.md                                               # REPLACES D03 SLICE 1 stub
docs/specs/coverage/D34_lobechat_recipe/
  design.md                                                   # this spec set
  implementation.md                                           # this doc
  tests.md
  acceptance.md
  review-standards.md
  citations/
    lobechat-environment-variables.pdf                        # snapshot of upstream docs (Slice 1)
  screenshots/                                                # Slice 2 only (optional)
    01-language-model-settings.png
deploy/demo/
  Makefile                                                    # MODIFIED Slice 1 (adds lobechat_real)
  compose.yaml                                                # MODIFIED Slice 1 (adds lobechat + lobechat-smoke services)
  lobechat_smoke.sh                                           # NEW Slice 1
  verify_step_lobechat_real.sql                               # NEW Slice 1
docs/internal/slices/
  COV_<seq>_d34_recipe_and_smoke.md                           # NEW Slice 1 slice doc
  COV_<seq+1>_d34_screenshots.md                              # NEW Slice 2 slice doc (optional)
```

---

## 3. Page skeleton (target output of Slice 1)

Canonical structure for `docs/site-v2/src/content/docs/docs/drop-in/lobechat.md`. Bracketed `[ … ]` blocks are author instructions and must not appear in the shipped file.

```markdown
---
title: "LobeChat + SpendGuard (drop-in via OPENAI_PROXY_URL)"
description: >-
  Point LobeChat's OpenAI provider at a running SpendGuard egress proxy
  with one environment variable. Every chat is reserved before the call
  and committed after; no LobeChat code change, no fork, no plugin install.
---

> [Hero — one sentence: LobeChat ships with an `OPENAI_PROXY_URL` env
>  var that rewrites the OpenAI upstream for every server-side chat;
>  SpendGuard exposes an OpenAI-compatible endpoint at
>  `/v1/chat/completions`; setting one env var puts SpendGuard in the
>  request path. Five-minute setup, one env var, one smoke.]

## Prerequisites

- LobeChat self-hosted (Docker, Vercel, or Kubernetes). LobeChat Cloud
  cannot point at a `localhost` SpendGuard; deploy SpendGuard with the
  [Helm chart](../../deployment/helm/) and use the public URL instead.
- A running SpendGuard egress proxy reachable from LobeChat. The fastest
  path is `make demo-up DEMO_MODE=proxy` from a clone of the SpendGuard
  repo; production deployments use the [Helm chart](../../deployment/helm/).
- One real OpenAI / Azure OpenAI key (SpendGuard forwards to the real
  upstream).

## Step 1: set OPENAI_PROXY_URL on your LobeChat container

The single decisive env var is `OPENAI_PROXY_URL`. Set it to your
SpendGuard egress proxy's `/v1` endpoint.

```bash
docker run -d --name lobe-chat \
    -p 3210:3210 \
    -e OPENAI_API_KEY=sk-...                          \
    -e OPENAI_PROXY_URL=http://host.docker.internal:9000/v1 \
    -e ACCESS_CODE=your-access-code                   \
    lobehub/lobe-chat:1.40.0
```

The trailing `/v1` is required — LobeChat appends `/chat/completions`
to whatever you set in `OPENAI_PROXY_URL`. Omitting `/v1` gives
`http://.../chat/completions` which the egress proxy does not match.

## Step 2: confirm the env var landed

```bash
docker exec lobe-chat env | grep OPENAI_PROXY_URL
# OPENAI_PROXY_URL=http://host.docker.internal:9000/v1
```

If the env var is empty, your `docker run` invocation dropped it. Set
it explicitly or use the compose snippet in Step 5.

## Step 3: send a test chat from the LobeChat UI

Open `http://localhost:3210`, enter your `ACCESS_CODE`, start a new chat
in the default `OpenAI / gpt-4o-mini` session, send "Say hi". LobeChat
POSTs to `/api/chat/openai`; the server route reads `OPENAI_PROXY_URL`,
forwards to SpendGuard's `/v1/chat/completions`; SpendGuard calls the
contract, reserves the predicted tokens, forwards to OpenAI, and on
response commits the actual usage.

## Step 4: verify end-to-end

```bash
# From a clone of github.com/m24927605/agentic-spendguard:
export OPENAI_API_KEY=sk-...
make demo-up DEMO_MODE=lobechat_real
```

The smoke boots SpendGuard + LobeChat, sends one chat through LobeChat's
`/api/chat/openai` route, and asserts the audit chain in Postgres shows
one `reserve` row and one `commit_estimated` row for the call.
Successful run prints `[lobechat-smoke] OK: reserve+commit verified`.

## Step 5: docker-compose snippet (production-like)

```yaml
services:
  lobe-chat:
    image: lobehub/lobe-chat:1.40.0
    ports:
      - "3210:3210"
    environment:
      OPENAI_API_KEY: ${OPENAI_API_KEY}
      OPENAI_PROXY_URL: http://spendguard-egress-proxy:9000/v1
      ACCESS_CODE: ${LOBECHAT_ACCESS_CODE}
      NEXT_PUBLIC_SERVICE_MODE: server
    depends_on:
      spendguard-egress-proxy:
        condition: service_healthy
```

## Deployment notes

### Docker / docker-compose

The Docker image is `lobehub/lobe-chat:1.40.0`. Mount a volume at
`/app/data` if you need conversation persistence (server-mode default
uses in-memory store).

### Vercel

LobeChat-on-Vercel inherits `OPENAI_PROXY_URL` from the Vercel project's
Environment Variables panel. Add the env var with the production URL of
your SpendGuard egress proxy (a Kubernetes Service URL or a hosted
SpendGuard URL — `localhost` will not work from Vercel).

### LobeChat Cloud (lobechat.com)

LobeChat Cloud cannot point at a `localhost` SpendGuard. Deploy SpendGuard
with [Helm](../../deployment/helm/) and use the public Kubernetes
Service URL as the proxy URL.

### Client mode (browser-side keys)

Server mode is recommended. If you must run client mode, each user
overrides the base URL per-session: **Settings → Language Model →
OpenAI → API Proxy Address** → your SpendGuard URL. Per-session
overrides do not honour `OPENAI_PROXY_URL`; the env var only applies
to the server route.

## Gotchas

- **Trailing `/v1` is mandatory.** LobeChat appends `/chat/completions`
  to whatever you set. Omitting `/v1` breaks the egress proxy route.
- **`OPENAI_API_KEY` is sent verbatim.** SpendGuard's egress proxy
  ignores the inbound `Authorization` header and substitutes the
  upstream credential from its own config. The LobeChat-side key
  exists only to satisfy LobeChat's "key present" validation.
- **`ACCESS_CODE` does not enforce auth at `/api/chat/openai`.** The
  ACCESS_CODE gates the UI; the server route still requires the access
  code as a request header in production. Set it.
- **Client mode bypasses the env var.** `OPENAI_PROXY_URL` is read by
  the LobeChat server only. Client-mode browsers call OpenAI directly;
  use the per-session UI override instead.
- **Streaming works.** LobeChat uses Server-Sent Events for streaming
  responses; the SpendGuard egress proxy passes SSE through unchanged
  and commits on the terminating `[DONE]` event.

## What next

- [Set a real budget](../../deployment/helm/)
- [Open the SpendGuard dashboard](../../operations/dashboard/)
- [Cover the rest of your stack (Pattern 3 install)](../../install/)
- Back to the [drop-in landing](../)

**Maintainer docs:** [LobeChat — Environment Variables](https://lobehub.com/docs/self-hosting/environment-variables/model-provider#openai_proxy_url)
```

---

## 4. Demo wiring (Slice 1)

### 4.1 `deploy/demo/Makefile` — add `lobechat_real` mode

After the `anythingllm_real` block (added by D33), insert:

```makefile
else ifeq ($(DEMO_MODE),lobechat_real)
	@echo "[demo] DEMO_MODE=lobechat_real → postgres + ledger + canonical-ingest + sidecar + egress-proxy + LobeChat."
	@echo "[demo] Boots lobehub/lobe-chat:1.40.0 with OPENAI_PROXY_URL pointing at"
	@echo "[demo]   egress-proxy, sends one chat through /api/chat/openai, asserts reserve+commit."
	@test -n "$$OPENAI_API_KEY" || (echo "[demo] FATAL: OPENAI_API_KEY required for DEMO_MODE=lobechat_real" >&2; exit 8)
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar egress-proxy lobechat
```

And in the post-bring-up smoke dispatcher:

```makefile
else ifeq ($(DEMO_MODE),lobechat_real)
	@echo "[demo] running LobeChat smoke..."
	$(COMPOSE) run --rm lobechat-smoke
```

### 4.2 `deploy/demo/compose.yaml` — two new services

```yaml
  lobechat:
    image: lobehub/lobe-chat:1.40.0
    profiles: ["lobechat_real"]
    environment:
      OPENAI_API_KEY: ${OPENAI_API_KEY}
      OPENAI_PROXY_URL: http://egress-proxy:9000/v1
      OPENAI_MODEL_LIST: "+gpt-4o-mini"
      ACCESS_CODE: "smoke-access-code"
      NEXT_PUBLIC_SERVICE_MODE: server
    ports:
      - "127.0.0.1:3210:3210"
    depends_on:
      egress-proxy:
        condition: service_healthy
    healthcheck:
      test: ["CMD-SHELL", "wget -qO- http://localhost:3210/api/health || exit 1"]
      interval: 5s
      timeout: 3s
      retries: 20

  lobechat-smoke:
    build:
      context: .
      dockerfile: lobechat_smoke.Dockerfile
    profiles: ["lobechat_real"]
    environment:
      LOBECHAT_URL: http://lobechat:3210
      LOBECHAT_ACCESS_CODE: "smoke-access-code"
      PROXY_URL: http://egress-proxy:9000
      POSTGRES_URL: postgres://spendguard:spendguard@postgres:5432/spendguard
      OPENAI_API_KEY: ${OPENAI_API_KEY}
    depends_on:
      lobechat:
        condition: service_healthy
    entrypoint: ["/bin/bash", "/smoke/lobechat_smoke.sh"]
    volumes:
      - ./lobechat_smoke.sh:/smoke/lobechat_smoke.sh:ro
      - ./verify_step_lobechat_real.sql:/smoke/verify.sql:ro
```

(The `lobechat_smoke.Dockerfile` is a four-line `FROM alpine:3.20`
with `curl`, `jq`, `postgresql-client` installed — identical to D33's
smoke Dockerfile.)

### 4.3 `deploy/demo/lobechat_smoke.sh`

Modelled on `anythingllm_smoke.sh`. Three steps (simpler than D33 because
LobeChat has no admin update-env API — the env var did the work at boot):

```bash
#!/bin/bash
# DEMO_MODE=lobechat_real smoke — LobeChat → SpendGuard → OpenAI round-trip.
set -euo pipefail

log() { echo "[lobechat-smoke] $*" >&2; }
fail() { log "FAIL: $*"; exit 1; }

# Step 0: LobeChat /api/health
curl -sS --max-time 5 "${LOBECHAT_URL}/api/health" >/dev/null \
    || fail "LobeChat not ready"

# Step 1: confirm env var was honoured (defensive — boot env var should
# already be in place; this step catches a stale image / regression).
log "step 1: confirm OPENAI_PROXY_URL on the container..."
# (Container env is opaque from this side; we infer by sending the chat
#  and asserting it landed in the audit chain — Step 3 is the real check.)

# Step 2: send one chat through /api/chat/openai
log "step 2: chat round-trip via SpendGuard..."
RESP=$(curl -sS --max-time 30 -X POST "${LOBECHAT_URL}/api/chat/openai" \
    -H 'Content-Type: application/json' \
    -H "X-LOBE-CHAT-AUTH: ${LOBECHAT_ACCESS_CODE}" \
    -d '{
        "model": "gpt-4o-mini",
        "messages": [{"role":"user","content":"Say hi in two words."}],
        "stream": false
    }')
echo "$RESP" | jq -e '.choices[0].message.content | length > 0' >/dev/null \
    || fail "no chat response: $RESP"
log "  chat OK"

# Step 3: verify audit chain
log "step 3: verify reserve+commit in ledger..."
psql "${POSTGRES_URL}" -v ON_ERROR_STOP=1 -f /smoke/verify.sql \
    || fail "verify SQL failed"

log "OK: reserve+commit verified"
```

### 4.4 `deploy/demo/verify_step_lobechat_real.sql`

Identical pattern to `verify_step_anythingllm_real.sql`. Asserts ≥ 1
`reserve` row and ≥ 1 `commit_estimated` row in `ledger_transactions`
for the demo tenant within the last 10 minutes. Fail-closed `DO $$ … $$`
block raises an exception if either row is missing.

---

## 5. Per-slice file plan

### 5.1 Slice 1 — Recipe page + smoke (`COV_<seq>_d34_recipe_and_smoke`)

**Adds:** `lobechat.md` (replaces D03 stub), `lobechat_smoke.sh`,
`verify_step_lobechat_real.sql`, `lobechat_smoke.Dockerfile`,
`citations/lobechat-environment-variables.pdf`, slice doc.

**Modifies:** `deploy/demo/Makefile` (adds `lobechat_real` mode),
`deploy/demo/compose.yaml` (adds `lobechat` + `lobechat-smoke` services).
D03 row 11 `Verified` column confirmed `Live` (already so per D03 design
§3.2; D34 closes the conditional).

**Build / verification (per `tests.md`):** Astro build green;
`DEMO_MODE=lobechat_real make demo-up` exits 0; smoke prints
`OK: reserve+commit verified`; upstream docs link-check passes.

### 5.2 Slice 2 — Screenshots + UX polish (optional)

**Adds:** One PNG under `screenshots/` (the Settings → Language Model →
OpenAI panel showing the `API Proxy Address` field, for the client-mode
callout). Inline reference inserted into the page where Slice 1's prose
documents the per-session UI path.

**Modifies:** `lobechat.md` — replace placeholder prose with embedded
screenshot syntax + caption.

**Verification:** Visual diff between Slice 1 baseline and Slice 2
matches the documented change set.

Descope rule: Slice 2 is skipped automatically if Slice 1 ships under
400 LOC AND the R1 reviewer flags Slice 2 as low marginal value (page
is already self-sufficient from prose).
