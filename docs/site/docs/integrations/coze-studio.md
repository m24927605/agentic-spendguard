---
description: >-
  Run Agentic SpendGuard budget control behind Coze Studio (ByteDance OSS,
  Apache-2.0) so every workspace chat-flow / agent / workflow LLM call is
  reserved pre-call and committed post-call through the SpendGuard ledger —
  without touching Coze source or installing a Coze plugin SDK.
---

# Coze Studio integration

> [Coze Studio](https://github.com/coze-dev/coze-studio) is ByteDance's
> open-source no-code agent builder (Apache-2.0, Go microservices, ~10k+
> stars). SpendGuard plugs into Coze through Coze's own workspace-level
> "OpenAI" custom-endpoint provider — your workspace continues to look like
> it's calling OpenAI, but every call routes through SpendGuard's HTTP
> companion. Hard budget caps, signed audit trail, real-spend
> reconciliation. No Coze plugin code, no Coze fork.

## Why you'd want this

- **Per-workspace budget caps.** Coze has no native budget primitive.
  SpendGuard reserves spend pre-call against a `BudgetBinding`, denies when
  the projection would overshoot.
- **Signed audit chain.** Every Coze model call writes a KMS-signed
  `LLM_CALL_PRE` + `LLM_CALL_POST` row in `audit_outbox`. Replayable from
  the audit log alone.
- **Real-spend reconciliation.** End-of-stream the companion reads OpenAI's
  `usage.completion_tokens` and commits true cost, not the worst-case
  estimator.
- **Multi-workspace attribution.** Three custom headers route each Coze
  workspace into its own budget — one SpendGuard sidecar serves multiple
  Coze workspaces with one install.
- **Fail-closed by default.** On DENY/DEGRADE Coze sees a real HTTP error
  (502 / 503) and surfaces it through its own workflow trace UI. No
  "permissive mode" foot-gun in the v1 snippet.

---

## How it works

```text
┌──────────────────────┐    ┌──────────────────────────────────┐    ┌──────────────────┐
│  Coze Studio          │    │   SpendGuard sidecar             │    │  OpenAI (real)   │
│  workspace            │    │   HTTP companion (mTLS, :8443)   │    │                  │
│                       │    │                                  │    │                  │
│  Model Provider →     │    │   1. validate X-SpendGuard-*     │    │                  │
│  OpenAI (custom URL)  │ ─► │   2. RequestDecision(RESERVE)    │    │                  │
│  base_url:            │    │   3. fwd ─────────────────────── │ ─► │  /v1/chat/comp   │
│  https://sidecar:8443 │    │   4. read upstream usage         │    │                  │
│  /v1/openai           │    │   5. ConfirmPublishOutcome       │    │                  │
│                       │ ◄─ │   6. fwd response                │ ◄─ │  response.json   │
└──────────────────────┘    └──────────────────────────────────┘    └──────────────────┘
                                          │
                                          ▼
                              ┌──────────────────────────┐
                              │  SpendGuard ledger        │
                              │  audit_outbox + commits   │
                              │  (KMS-signed, hash chain) │
                              └──────────────────────────┘
```

## Install (self-hosted Coze)

### 1. Verify prereqs

- Coze Studio ≥ v1 with the workspace-level **Model Provider → OpenAI →
  Custom Endpoint** form. The form must accept (a) a custom `base_url`,
  (b) a custom `api_key` field, (c) custom headers, and (d) client
  certificate material.
- A SpendGuard sidecar built from `main` with the HTTP companion endpoint
  available (`services/sidecar/src/http_companion/`, shipped by
  `D09 SLICE 1`).
- An mTLS bundle: a CA, a sidecar server cert + key, and a Coze-side
  client cert + key.
- A SpendGuard control-plane install with one budget + one open
  window-instance per Coze workspace you want to gate.

### 2. Paste the workspace config

The full snippet lives at
[`examples/coze-studio/coze-workspace-config.yaml`](https://github.com/m24927605/agentic-spendguard/blob/main/examples/coze-studio/coze-workspace-config.yaml).
Verbatim:

```yaml
provider: openai
display_name: SpendGuard-Gated OpenAI
base_url: https://spendguard-sidecar.spendguard.svc.cluster.local:8443/v1/openai
api_key: ${OPENAI_API_KEY}
custom_headers:
  X-SpendGuard-Tenant-Id: "<COZE_WORKSPACE_ID>"
  X-SpendGuard-Budget-Id: "<SPENDGUARD_BUDGET_ID>"
  X-SpendGuard-Window-Instance-Id: "<SPENDGUARD_WINDOW_INSTANCE_ID>"
tls:
  ca_cert_path: /etc/coze/spendguard-ca.pem
  client_cert_path: /etc/coze/coze-client.pem
  client_key_path: /etc/coze/coze-client.key
models:
  - id: gpt-4o-mini
  - id: gpt-4o
  - id: gpt-3.5-turbo
```

Replace the three `<...>` placeholders with the Coze workspace ID + the
SpendGuard budget UUID + the open window-instance UUID. The
[`headers-cheatsheet.md`](https://github.com/m24927605/agentic-spendguard/blob/main/examples/coze-studio/headers-cheatsheet.md)
documents how to extract each value.

### 3. Mount the mTLS bundle into Coze

The sidecar companion is mTLS-only. Mount the three PEMs at `/etc/coze/`:

```yaml
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

### 4. Verify with the smoke

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard
export OPENAI_API_KEY=sk-...        # real key, real upstream
bash examples/coze-studio/smoke.sh
```

The smoke replays Coze's "Test connection" probe through the companion,
asserts HTTP 200 + OpenAI-shaped response + a reserve/commit audit row pair
with `decision_context->>'integration' = 'coze_studio'`, exercises the
negative path (missing tenant → 400 `MISSING_TENANT`), and tears down
cleanly.

---

## Demo mode

```bash
make demo-down
export OPENAI_API_KEY=sk-...
make demo-up DEMO_MODE=coze_studio_real
```

This boots the full SpendGuard demo stack with the Coze overlay and runs a
3-step matrix (ALLOW + DENY + STREAM) against the companion endpoint. On
success the runner prints:

```
[demo] coze_studio_real ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

The verify SQL runs all 7 D31 gates inline — including the INV-1 "DENY
never hits upstream" assertion via a counting-stub call-delta check.

The Coze Studio container itself is profiled-off by default (`profiles:
[coze]`) because the headline invariants (INV-1 through INV-5) are proved
at the companion contract boundary. Set `COMPOSE_PROFILES=coze` before
`make demo-up` to bring up the full Coze UI alongside the driver.

---

## Decision matrix

You can drop SpendGuard in front of Coze through multiple install paths.
Pick by what else your team runs.

| Install path | When it's the right choice | Tradeoffs |
|--------------|----------------------------|-----------|
| **D31 — base-URL redirect (this page)** | You only run Coze Studio. Want minimal install footprint. Want fail-closed by default. | One per Coze workspace; per-workspace cert provisioning. Coze Cloud not supported (gated SaaS form). |
| **D02/D03 — SpendGuard egress proxy** | You also run terminal CLIs, scripts, other apps from the same pod / VM. One install gates everything that talks OpenAI on that host. | More moving parts. Requires SpendGuard egress proxy in path of all OpenAI traffic. |
| **D10 — Dify plugin** | You're on Dify, not Coze. Not applicable here. | n/a — different platform. |
| **Future D31 v1.1 — Coze plugin SDK** | You want a native Coze plugin tool that intercepts LLM calls intra-process. Tighter binding, more Coze-specific work. | Deferred — Pattern 2 (this page) already covers 100% of Coze model calls. v1.1 GH issue tracks the SDK route. |

Quick rule of thumb:

- **Coze + terminal CLIs + other apps on same pod** → D02/D03 egress proxy
  (one install, broader coverage).
- **Coze-only**, **want minimum config** → D31 base-URL (this page).

---

## Limitations

v1 of the D31 integration does NOT cover:

- **Coze Cloud (SaaS).** Coze Cloud's provider config form is gated and
  not validatable without a Coze partnership. Self-hosted Coze only.
- **Anthropic / Gemini / Bedrock provider slots.** v1 covers OpenAI-compatible
  only. Operators with non-OpenAI traffic install the SpendGuard egress
  proxy (D02/D03) — that path is provider-agnostic. Anthropic / Gemini /
  Bedrock surfaces in Coze ship in v1.1.
- **Mid-stream cap enforcement.** Commit fires end-of-stream (inherits
  D09 SLICE 1 §3.3). A streaming call that crosses the budget mid-stream
  completes the stream, then the next reserve denies.
- **Native Coze plugin SDK route.** That's Pattern 3 — Go plugin tool
  intercepting LLM calls intra-process. v1.1 follow-up.
- **Multi-workspace federation via a single config.** Per-workspace snippet
  only — paste a copy into each workspace you want gated.
- **Coze upstream PR.** Recipe lives in the SpendGuard repo only.

---

## Troubleshooting

### Coze: "Failed to connect to upstream provider"

The companion is unreachable from Coze. Check:

- Curl from inside the Coze pod: `kubectl exec -it deploy/coze-studio --
  curl -v --cacert /etc/coze/spendguard-ca.pem --cert
  /etc/coze/coze-client.pem --key /etc/coze/coze-client.key
  https://spendguard-sidecar.spendguard.svc.cluster.local:8443/healthz`
  must return 200.
- The cert chain in `spendguard-ca.pem` covers the sidecar's server cert
  (CN must match the host portion of `base_url`).
- Network policy allows Coze pod → sidecar service on TCP 8443.

### Coze: "HTTP 400 MISSING_TENANT"

Coze is hitting the companion but the `X-SpendGuard-Tenant-Id` header is
empty. Re-check the workspace config — Coze's custom-header field
silently drops empty values. Re-paste the snippet and confirm a non-empty
workspace ID.

### Coze: "HTTP 400 INVALID_BUDGET_ID" / "INVALID_WINDOW_INSTANCE_ID"

The header is set but not a canonical UUID v4. Re-extract from the
control plane (`headers-cheatsheet.md` shows the queries) — Coze does
not normalize hyphens / case, so a copy from logs without hyphens
silently fails.

### Coze: "HTTP 502 Bad Gateway" on every call

The companion is denying every call. Either:

- The budget is exhausted (intended fail-closed behaviour). Check
  `GET /api/v1/budgets/{id}/state` — `available_micro_usd` should be > the
  projection.
- The contract bundle denied the request shape. Check sidecar logs for
  a `DENY` line + reason code.
- The window instance is closed (HTTP 409 `WINDOW_INSTANCE_CLOSED` if so —
  re-extract the open window-instance UUID from the control plane).

### Sidecar audit row missing after a successful call

The call landed but the canonical-ingest writer hasn't drained yet. Wait
5s and re-query:

```sql
SELECT decision_id, decision_context->>'integration', created_at
  FROM audit_outbox
 WHERE decision_context->>'integration' = 'coze_studio'
 ORDER BY created_at DESC
 LIMIT 5;
```

If still empty after 30s, check the outbox-forwarder pod logs — that's
the component draining `audit_outbox` into the canonical chain.

---

## See also

- [examples/coze-studio/README.md](https://github.com/m24927605/agentic-spendguard/tree/main/examples/coze-studio) —
  operator-facing walkthrough.
- [examples/coze-studio/headers-cheatsheet.md](https://github.com/m24927605/agentic-spendguard/blob/main/examples/coze-studio/headers-cheatsheet.md) —
  the three `X-SpendGuard-*` header format rules + control-plane queries.
- [examples/coze-studio/smoke.sh](https://github.com/m24927605/agentic-spendguard/blob/main/examples/coze-studio/smoke.sh) —
  the smoke harness.
- [Dify Model Provider Plugin](https://github.com/m24927605/agentic-spendguard/tree/main/plugins/dify/spendguard) —
  sibling no-code platform (D10).
