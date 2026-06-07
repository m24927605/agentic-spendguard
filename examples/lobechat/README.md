# SpendGuard + LobeChat — drop-in recipe

End-to-end config bundle for wiring [LobeChat](https://lobehub.com/)
into a SpendGuard egress proxy via its `OPENAI_PROXY_URL` environment
variable. No SDK install, no LobeChat code change, no plugin sideload —
one env var on the LobeChat container routes every server-side chat
through the SpendGuard pre-call budget gate and KMS-signed audit chain.

Spec: [`docs/specs/coverage/D34_lobechat_recipe/design.md`](../../docs/specs/coverage/D34_lobechat_recipe/design.md)

Walkthrough: [docs.agenticspendguard.dev/docs/drop-in/lobechat/](https://agenticspendguard.dev/docs/drop-in/lobechat/)

## Topology

```
LobeChat user session (server mode)
      |
      v
LobeChat /api/chat/openai server route  (reads OPENAI_PROXY_URL)
      |
      v
SpendGuard egress proxy :9000  /v1/chat/completions
      |  reserve via sidecar -> ledger
      v
OpenAI / Azure OpenAI (upstream)
      |  usage extracted from response
      v
SpendGuard commits actual usage to the ledger
```

## Files

| File | Purpose |
|------|---------|
| `lobechat.env` | Drop-in env file for `docker run --env-file` / `docker compose` boots of LobeChat. Sets `OPENAI_PROXY_URL`, `ACCESS_CODE`, and the server-mode flag SpendGuard expects. |
| `setup.sh` | Four-step bootstrap (LobeChat reachable, env-var confirmation, smoke chat, audit assertion). Mirrors `deploy/demo/lobechat_smoke.sh` but runs against an operator-supplied LobeChat instance instead of a profile-gated demo container. |

## Quick start (Docker, real OpenAI key)

```bash
# 1. Boot SpendGuard locally (binds http://localhost:9000/v1).
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard
export OPENAI_API_KEY=sk-...
make demo-up DEMO_MODE=proxy

# 2. From this directory, boot LobeChat with the prepared env file.
docker run -d --name lobe-chat \
  --env-file ./lobechat.env \
  -e OPENAI_API_KEY=${OPENAI_API_KEY} \
  -p 3210:3210 \
  lobehub/lobe-chat:1.40.0

# 3. Wait for /api/health, then verify the round-trip.
./setup.sh
```

`setup.sh` prints `OK: reserve+commit verified` when the SpendGuard
ledger shows one `reserve` row and one `commit_estimated` row for the
smoke chat. The full automated equivalent — bringing up SpendGuard
plus LobeChat plus a smoke runner in one command — lives at
`make demo-up DEMO_MODE=lobechat_real` from the SpendGuard repo root.

## Manual configuration (UI path)

If you want to configure LobeChat by hand instead of running
`setup.sh`:

1. Boot LobeChat with `OPENAI_PROXY_URL` set in the container env
   (the `lobechat.env` file in this directory does this for you).
2. Open `http://localhost:3210` in your browser and enter the
   `ACCESS_CODE` you set.
3. Start a new chat in the default `OpenAI / gpt-4o-mini` session
   and send any message. The server route reads `OPENAI_PROXY_URL`
   transparently — there is no UI step.
4. Confirm SpendGuard saw the call by watching the dashboard at
   `http://localhost:8090` during the chat.

LobeChat has no admin UI for `OPENAI_PROXY_URL` — the env var is the
single source of truth. Per-session UI overrides
(**Settings → Language Model → OpenAI → API Proxy Address**) apply
only in client mode and only for the browser session that sets them.

## Verification

If you bound to the SpendGuard demo Postgres:

```bash
psql postgres://spendguard:spendguard_demo@localhost:5432/spendguard_ledger \
  -c "SELECT operation_kind, COUNT(*)
        FROM ledger_transactions
       WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
         AND operation_kind IN ('reserve','commit_estimated')
         AND event_time > now() - interval '10 minutes'
       GROUP BY operation_kind;"
```

Expected output: one row per `operation_kind`, both with `count >= 1`.

For the full automated verify, run
`make demo-up DEMO_MODE=lobechat_real` from the SpendGuard repo root —
that boots this exact configuration plus the LobeChat container plus a
smoke runner that asserts the audit chain.

## Production notes

- **LobeChat Cloud** (lobechat.com) cannot reach `http://localhost:9000`.
  Deploy SpendGuard via the [Helm chart](../../docs/site-v2/src/content/docs/docs/deployment/helm.mdx)
  and use the public Service URL as the proxy URL.
- **Vercel.** LobeChat-on-Vercel inherits `OPENAI_PROXY_URL` from the
  Vercel project's Environment Variables panel. Set it to your
  production SpendGuard URL — a `localhost` value will not work from
  Vercel's edge runtime.
- **Client mode.** Browser-side keys bypass `OPENAI_PROXY_URL`
  entirely. If you must run client mode, each user sets
  **Settings → Language Model → OpenAI → API Proxy Address** in their
  own browser session. Server mode is the only configuration where
  one env var deterministically gates every call.
- **Image pin.** This recipe was verified against
  `lobehub/lobe-chat:1.40.0`. Floating `:latest` works most days but
  will surface env-var-rename drift on its own schedule.
- **Conversation persistence.** Server mode is in-memory by default;
  mount a volume at `/app/data` if you need chat history across
  container restarts.

## What this recipe does not cover

- Anthropic / Bedrock / Vertex / Gemini provider tiles. LobeChat reads
  provider-specific env vars (`ANTHROPIC_PROXY_URL` etc.); this recipe
  covers the OpenAI proxy only.
- Plugin marketplace, agent skills, TTS, image generation, vision.
  Chat path only.
- Client-mode browser keys. Documented via the per-session UI override
  callout above; not smoke-covered because the override is GUI-driven
  per browser session and cannot be driven from a CI smoke.
- LobeChat Cloud (lobechat.com). Cloud cannot reach `localhost`;
  Helm-deployed SpendGuard with a public URL is the only path.
