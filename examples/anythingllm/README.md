# SpendGuard + AnythingLLM — drop-in recipe

End-to-end config bundle for wiring [AnythingLLM](https://anythingllm.com/)
into a SpendGuard egress proxy via its **Generic OpenAI** provider tile.
No SDK install, no AnythingLLM code change, no plugin sideload — one
field in the LLM Preference panel routes every Workspace chat through
the SpendGuard pre-call budget gate and KMS-signed audit chain.

Spec: [`docs/specs/coverage/D33_anythingllm_recipe/design.md`](../../docs/specs/coverage/D33_anythingllm_recipe/design.md)

Walkthrough: [docs.agenticspendguard.dev/docs/drop-in/anythingllm/](https://agenticspendguard.dev/docs/drop-in/anythingllm/)

## Topology

```
AnythingLLM Workspace chat
      |
      v
AnythingLLM Generic OpenAI provider (Base URL = SpendGuard egress)
      |
      v
SpendGuard egress proxy :9000  /v1/chat/completions
      |  reserve via sidecar -> ledger
      v
OpenAI / Anthropic / Bedrock / Vertex / Azure OpenAI (upstream)
      |  usage extracted from response
      v
SpendGuard commits actual usage to the ledger
```

## Files

| File | Purpose |
|------|---------|
| `anythingllm.env` | Drop-in env file for `docker run` / `docker compose` boots of AnythingLLM. Sets `STORAGE_DIR`, `SERVER_PORT`, and the Generic OpenAI defaults SpendGuard expects. |
| `generic-openai-config.json` | Payload for AnythingLLM's `/api/v1/system/update-env` endpoint. POST it once per fresh AnythingLLM instance to pre-configure the Generic OpenAI provider without clicking through the UI. |
| `setup.sh` | Five-step bootstrap (account create, provider configure, workspace create, smoke chat, audit assertion). Mirrors `deploy/demo/anythingllm_smoke.sh` but is operator-runnable against a real deployment. |

## Quick start (Docker, real OpenAI key)

```bash
# 1. Boot SpendGuard locally (binds http://localhost:9000/v1).
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard
export OPENAI_API_KEY=sk-...
make demo-up DEMO_MODE=proxy

# 2. From this directory, boot AnythingLLM with the prepared env file.
docker run -d --name anythingllm \
  --env-file ./anythingllm.env \
  -p 3001:3001 \
  -v anythingllm-storage:/app/server/storage \
  mintplexlabs/anythingllm:1.8.4

# 3. Wait for /api/ping, then configure the provider.
./setup.sh
```

`setup.sh` prints `OK: reserve+commit verified` when the SpendGuard
ledger shows one `reserve` row and one `commit_estimated` row for the
smoke chat.

## Manual configuration (GUI path)

If you want to configure AnythingLLM by hand instead of running
`setup.sh`:

1. Open `http://localhost:3001` in your browser.
2. Click **Settings → LLM Preference**.
3. Scroll to the **Generic OpenAI** provider tile (not the plain
   **OpenAI** tile — that one targets `api.openai.com` directly and
   does not expose a Base URL field).
4. Fill in the four fields:
   - **Base URL**: `http://localhost:9000/v1`
   - **API Key**: any non-empty string (e.g. `sk-anythingllm-spendguard`)
   - **Chat Model Name**: `gpt-4o-mini` (or whichever upstream model
     your SpendGuard egress proxy is configured to forward to)
   - **Token context window**: `128000`
5. Click **Save Settings**.
6. Open any Workspace and send a chat message. The call routes through
   SpendGuard.

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

For the full automated verify, run `make demo-up
DEMO_MODE=anythingllm_real` from the SpendGuard repo root — that boots
this exact configuration plus the AnythingLLM container plus a smoke
runner that asserts the audit chain.

## Production notes

- **AnythingLLM Cloud** cannot reach `http://localhost:9000`. Deploy
  SpendGuard via the [Helm chart](../../docs/site-v2/src/content/docs/docs/deployment/helm.mdx)
  and use the public Service URL as the Base URL.
- **AnythingLLM Desktop** has no admin API, so `setup.sh` cannot drive
  it. Follow the Manual configuration steps above; verify by watching
  the SpendGuard dashboard during your next chat.
- **Image pin.** This recipe was verified against
  `mintplexlabs/anythingllm:1.8.4`. Floating `:latest` works most days
  but will surface UI label drift on its own schedule.
- **One Workspace, one provider.** AnythingLLM applies the LLM
  Preference globally across Workspaces. Run two AnythingLLM
  instances if you need to split SpendGuard-gated and direct traffic.

## What this recipe does not cover

- Embedding provider configuration (SpendGuard's egress proxy gates
  chat completions; embeddings are out of scope).
- Agent-Skills, voice, multimodal — chat path only.
- AnythingLLM's `/api/v1/document/upload` flow.
