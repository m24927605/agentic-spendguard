# SpendGuard for Coze Studio (D31)

Coze Studio is ByteDance's open-source no-code agent builder
(Apache-2.0, Go microservices, ~10k+ stars). This recipe wires SpendGuard
budget control behind every Coze Studio chat-flow / agent / workflow LLM
call without modifying Coze source or installing a Coze plugin SDK.

The integration is **Pattern 2 — OpenAI-compatible base-URL redirect**: Coze
already lets workspace admins point its "OpenAI" model provider at any
OpenAI-compatible HTTPS endpoint. SpendGuard's sidecar HTTP companion
(`D09 SLICE 1`) is that endpoint. Once configured, every Coze model call
flows: Coze → companion (reserve) → upstream OpenAI → companion (commit) →
Coze. DENY surfaces as HTTP 502 in Coze's workflow trace; ALLOW surfaces as
the normal OpenAI response.

## What you get

| Concern | How SpendGuard adds value |
|---------|---------------------------|
| Per-workspace budgets | Coze has no native budget primitive. SpendGuard reserves spend pre-call against a `BudgetBinding`, denies when the projection would overshoot. |
| Signed audit chain | Every Coze model call writes a KMS-signed `LLM_CALL_PRE` + `LLM_CALL_POST` row in `audit_outbox`. Replayable from the audit log alone. |
| Real-spend reconciliation | End-of-stream the companion reads OpenAI's `usage.completion_tokens` and commits true cost, not the worst-case estimator. |
| Multi-workspace attribution | Three custom headers (`X-SpendGuard-Tenant-Id` / `X-SpendGuard-Budget-Id` / `X-SpendGuard-Window-Instance-Id`) route each workspace into its own budget. |
| Fail-closed default | On DENY/DEGRADE Coze sees a real HTTP error (502 / 503). No "fail open" foot-gun in the v1 snippet. |

## Prereqs

- Coze Studio ≥ v1 (OSS release with workspace-level custom-endpoint provider
  config — verified against image `ghcr.io/coze-dev/coze-studio` pinned by
  SHA256 digest, see `docker-compose.coze.yaml`).
- A SpendGuard sidecar built from `main` with the HTTP companion endpoint
  available (`services/sidecar/src/http_companion/`, shipped by `D09 SLICE 1`).
- An mTLS bundle covering the sidecar host + a Coze-side client cert/key.
- A SpendGuard control-plane install (or seed SQL) with one budget +
  one open window instance per Coze workspace you want to gate.
- An `OPENAI_API_KEY` env var available to Coze. This recipe passes the key
  through; it never appears in any file under `examples/coze-studio/`
  (D31 INV-6).

## Install in 4 steps

### 1. Extract the three header values

The Coze workspace ID is the `X-SpendGuard-Tenant-Id` value. The SpendGuard
budget UUID + open window-instance UUID are the other two header values.
See `headers-cheatsheet.md` for the exact format rules + control-plane
queries.

```bash
# Coze workspace ID — Workspace → Settings → General → Workspace ID
COZE_WORKSPACE_ID=7234567890123456789

# SpendGuard control plane lookup
SPENDGUARD_BUDGET_ID=$(curl -fsS "$SPENDGUARD_CP/api/v1/budgets" |
    jq -r '.[] | select(.name=="Coze Acme prod") | .budget_id')
SPENDGUARD_WINDOW_INSTANCE_ID=$(curl -fsS \
    "$SPENDGUARD_CP/api/v1/budgets/${SPENDGUARD_BUDGET_ID}/window-instances?state=open" |
    jq -r '.[0].window_instance_id')
```

### 2. Paste the workspace config snippet

Open the target Coze workspace UI:
**Workspace → Settings → Model Provider → OpenAI (custom endpoint)**.

If your Coze version exposes the YAML editor, paste
`coze-workspace-config.yaml` verbatim and substitute the three placeholders
(`<COZE_WORKSPACE_ID>`, `<SPENDGUARD_BUDGET_ID>`,
`<SPENDGUARD_WINDOW_INSTANCE_ID>`) with the values from step 1.

If your Coze version only exposes the web form, copy each field individually
from the YAML body. The required fields are: `provider`, `display_name`,
`base_url`, `api_key`, `custom_headers` (three `X-SpendGuard-*` rows), and
the `tls` block (CA + client cert + key paths).

### 3. Mount the mTLS bundle into Coze

The sidecar companion is mTLS-only (`D09 SLICE 1 §3.1` lock). Coze must
present a client cert. The snippet expects the bundle at:

```
/etc/coze/spendguard-ca.pem      # CA that signed the sidecar's server cert
/etc/coze/coze-client.pem        # Coze's client cert (signed by SpendGuard CA)
/etc/coze/coze-client.key        # Coze's client key
```

**docker-compose install.** The smoke harness (`smoke.sh`) wires this via the
shared `pki-init` volume — see `docker-compose.coze.yaml` for the mount layout
that gets the bundle into the Coze container at `/etc/coze/`.

**Kubernetes install.** Create a Secret with the three PEMs and project it
into the Coze pod's `/etc/coze/`. Example:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: coze-spendguard-mtls
  namespace: coze
stringData:
  spendguard-ca.pem: |
    <CA bundle>
  coze-client.pem: |
    <client cert>
  coze-client.key: |
    <client key>
---
# Coze Deployment patch
spec:
  template:
    spec:
      containers:
        - name: coze-studio
          volumeMounts:
            - name: spendguard-mtls
              mountPath: /etc/coze
              readOnly: true
      volumes:
        - name: spendguard-mtls
          secret:
            secretName: coze-spendguard-mtls
            defaultMode: 0o400
```

### 4. Run the smoke

```bash
export OPENAI_API_KEY=sk-...       # real key, real upstream
bash examples/coze-studio/smoke.sh
```

The smoke replays Coze's "test connection" probe through the companion,
asserts HTTP 200, asserts the response body is OpenAI-shaped, and asserts a
reserve+commit row pair shows up in `spendguard_ledger.audit_outbox` with
`decision_context->>'integration' = 'coze_studio'`. It also exercises the
negative path — missing tenant header → 400 with `MISSING_TENANT` — so you
know the fail-closed contract is live.

## Demo mode

A full end-to-end demo (Coze Studio + real OpenAI) ships as
`DEMO_MODE=coze_studio_real`:

```bash
make demo-down
export OPENAI_API_KEY=sk-...
make demo-up DEMO_MODE=coze_studio_real
```

The demo runs three steps — ALLOW (small prompt fits budget), DENY
(pre-exhausted budget → 502 surface in Coze), STREAM (SSE end-of-stream
commit) — and asserts every audit row through
`verify_step_coze_studio_real.sql`.

## Troubleshooting

### Coze: "Failed to connect to upstream provider"

The companion is unreachable from Coze. Check:

- `kubectl exec -it deploy/coze-studio -- curl -v --cacert /etc/coze/spendguard-ca.pem --cert /etc/coze/coze-client.pem --key /etc/coze/coze-client.key https://spendguard-sidecar.spendguard.svc.cluster.local:8443/healthz` returns 200.
- The cert chain in `spendguard-ca.pem` covers the sidecar's server cert (CN
  must match the host portion of `base_url`).
- Network policy allows Coze pod → sidecar service on TCP 8443.

### Coze: "HTTP 400 MISSING_TENANT"

Coze is hitting the companion but the `X-SpendGuard-Tenant-Id` header is
empty. Re-check the workspace YAML — Coze's custom-header field can silently
drop empty values. Re-paste the snippet and ensure a non-empty workspace ID.

### Coze: "HTTP 400 INVALID_BUDGET_ID" / "INVALID_WINDOW_INSTANCE_ID"

The header is set but not a canonical UUID v4. Re-extract from the control
plane (`headers-cheatsheet.md` shows the queries) — Coze does NOT
normalize hyphens / case, so a copy from logs without hyphens silently fails.

### Coze: "HTTP 502 Bad Gateway" on every call

The companion is denying every call. Either:

- The budget is exhausted (intended fail-closed behaviour). Check
  `GET /api/v1/budgets/{id}/state` — `available_micro_usd` should be > the
  projection.
- The contract bundle denied the request shape. Check sidecar logs for a
  `DENY` line + reason code.
- The window instance is closed
  (`HTTP 409 WINDOW_INSTANCE_CLOSED` if so — re-extract the open
  window-instance UUID from the control plane).

### Coze: "HTTP 503 Service Unavailable" intermittently

The companion is in DEGRADE state — most often the ledger or canonical-ingest
is unreachable. Sidecar pods serve 503 until they recover; Coze retries via
its own retry/error UI. Check `kubectl get pods -n spendguard` and the
sidecar's `/readyz` endpoint.

### Sidecar audit row missing after a successful call

The call landed but the canonical-ingest writer hasn't drained yet. Wait 5s
and re-query:

```sql
SELECT decision_id, decision_context->>'integration', created_at
  FROM audit_outbox
 WHERE decision_context->>'integration' = 'coze_studio'
 ORDER BY created_at DESC
 LIMIT 5;
```

If still empty after 30s, check the outbox-forwarder pod logs — it's the
component that drains `audit_outbox` into the canonical chain.

## Anti-scope (v1)

- **No native Coze plugin SDK route.** That's v1.1 — tracked at design.md §3.6.
- **No Anthropic / Gemini / Bedrock provider slots.** v1 covers OpenAI-compatible
  only (design.md §3.5). Operators with non-OpenAI traffic install the
  SpendGuard egress proxy (D02/D03).
- **No mid-stream cap enforcement.** Commit fires end-of-stream
  (inherits D09 SLICE 1 §3.3).
- **No Coze Cloud (SaaS) automation.** Coze Cloud's provider config is gated
  and not validateable without a Coze partnership. Self-hosted Coze only.
- **No multi-workspace federation via a single config.** Per-workspace snippet
  only — paste a copy into each workspace you want gated.

## See also

- `headers-cheatsheet.md` — the three `X-SpendGuard-*` header format rules +
  control-plane queries that produce each value.
- `coze-workspace-config.yaml` — the snippet operators paste.
- `docker-compose.coze.yaml` — minimal Coze + sidecar stack for the smoke
  harness.
- `smoke.sh` — curl-driven smoke that replays Coze's test-connection probe.
- `docs/site/docs/integrations/coze-studio.md` — public docs page with the
  decision matrix (D31 base-URL vs D02 / D03 egress-proxy).
- `docs/specs/coverage/D31_coze_studio/` — design + acceptance specs.
