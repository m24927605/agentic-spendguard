# Phase 4 Onboarding Roadmap — Slice Plan

**Status**: planning artifact — slices are independently shippable units, each gated by demo + Codex adversarial review per `feedback_codex_review.md` + `feedback_demo_quality_gate.md`. No time estimates per `feedback_working_principles.md`.

**Branch convention**: `feat/onboarding-O<N>-<short-name>` (one branch per slice).

---

## 0. Context

Phase 2B + Phase 3 wedge proved the runtime architecture:
- Sidecar mTLS gRPC + Contract DSL hot-path evaluator
- Postgres SERIALIZABLE ledger + transactional audit_outbox
- 6 demo modes including real-OpenAI / real-Anthropic
- Audit chain end-to-end durable to canonical_events

What does **NOT** exist for an external user (per honest 2026-05-09 assessment):
- No `pip install spendguard-sdk` package
- No Helm chart / k8s manifest
- No Terraform module
- No SaaS control plane / tenant provisioning
- No dashboard / operator UI
- No live pricing table; pricing is hardcoded
- No adapter for LangChain / Semantic Kernel / OpenAI Agents / AGT
- No documentation site
- No `usd_micros` budget unit (still token-denominated; cross-provider broken)

This roadmap slices these gaps into 10 independently-shippable units.

---

## Dependency Graph

```
                      ┌── O1 SDK package ──┬─→ O5 LangChain adapter
                      │                    ├─→ O6 AGT plugin
                      │                    └─→ all other SDK consumers
                      │
                      ├── O3 Pricing table ──→ O4 USD-denominated budget
                      │                         │
                      │                         └─→ O7 Dashboard
                      │                              │
                      ├── O2 Helm chart ──→ O9 Terraform module
                      │                         │
                      │                         └─→ O8 Control plane
                      │
                      └── O10 Docs site (continuous; refreshed after each slice)
```

Suggested execution order (when no specific user pull): O1 → O3 → O5 → O4 → O2 → O7 → O6 → O9 → O8, with O10 refreshed after every slice.

If a specific design partner has a stack (e.g., LangChain on AKS), pull O5 + O2 + O1 first regardless.

---

## Slice O1 — SDK Publishable Package

**Goal**: external developers can `pip install spendguard-sdk` instead of forking the repo.

### Design

- Source: `adapters/pydantic-ai/` becomes a publishable PyPI package
- Rename to `spendguard-sdk` (since SpendGuardClient is framework-agnostic; the Pydantic-AI bits become an optional extra)
- Package layout:
  ```
  spendguard-sdk/
    pyproject.toml           # name = spendguard-sdk
    src/spendguard/
      client.py              # SpendGuardClient (UDS gRPC, mTLS optional)
      errors.py              # DecisionStopped, etc.
      ids.py                 # derive_idempotency_key, new_uuid7
      _proto/                # generated stubs
      integrations/
        pydantic_ai.py       # SpendGuardModel (was model.py)
        # langchain.py — slot for O5
        # openai_agents.py — future
    examples/
      pydantic_ai_basic.py
      hello_world.py
    README.md, CHANGELOG.md, LICENSE
  ```
- Versioning: semver `0.1.0` for first publish; pin proto-stubs version to compatible spendguard wire-protocol version
- Extras:
  - `pip install spendguard-sdk` → core client only
  - `pip install spendguard-sdk[pydantic-ai]` → also installs pydantic-ai
  - (future) `[langchain]`, `[openai-agents]`, `[anthropic]`, etc.

### Implementation

1. Restructure `adapters/pydantic-ai/` directory layout per design
2. Update `pyproject.toml` with metadata, classifiers, optional-deps
3. Update `Makefile` with `make publish-test` (twine to test.pypi.org) + `make publish` (PyPI)
4. Write `examples/` standalone scripts that work with a remote sidecar (UDS or TCP gRPC)
5. Update `deploy/demo/runtime/Dockerfile.adapter` to install from local source via extras
6. Add `__version__` constant + sync with git tags

### Test plan

- Build sdist + wheel: `python -m build`
- In a fresh venv: `pip install dist/spendguard_sdk-0.1.0-*.whl[pydantic-ai]`
- Run `examples/hello_world.py` against a running sidecar; assert successful handshake + RequestDecision
- Round-trip: existing `DEMO_MODE=agent` still passes after SDK reorganization
- `python -m twine check dist/*` passes

### Acceptance criteria

- [ ] `pip install spendguard-sdk` succeeds in a clean Python 3.10+ venv
- [ ] `from spendguard import SpendGuardClient` works without optional deps
- [ ] `from spendguard.integrations.pydantic_ai import SpendGuardModel` works with `[pydantic-ai]` extra
- [ ] Existing demo modes unchanged (no regression)
- [ ] CHANGELOG documents wire-protocol compatibility matrix

### Codex adversarial review focus

- Backwards-compat: any imports that downstream `deploy/demo/demo/run_demo.py` uses must keep working OR be cleanly deprecated
- API surface stability: anything exported from `spendguard.__init__` becomes a hard contract — review for premature exposure
- Proto stub version pinning: if SDK ships proto stubs that drift from sidecar wire format, decision RPCs break silently. Verify version compat check at handshake
- Optional-deps boundaries: `from spendguard.integrations.pydantic_ai import ...` MUST raise a clean `ImportError` with install hint when `[pydantic-ai]` extra is missing, not a confusing `ModuleNotFoundError` deep in the stack

---

## Slice O2 — Helm Chart for Sidecar Stack

**Goal**: `helm install spendguard ./charts/spendguard` deploys the full sidecar topology to a k8s cluster.

### Design

- Single umbrella chart `charts/spendguard/` with subcharts:
  - `postgres` (or use Bitnami's chart as dependency)
  - `pki-init` (job: generate CA + per-service certs into a Kubernetes Secret)
  - `bundles-init` (job: bake contract.yaml into a ConfigMap or volume)
  - `ledger` (Deployment + Service + mTLS Secret mount)
  - `canonical-ingest` (Deployment + Service + mTLS)
  - `sidecar` (DaemonSet OR sidecar-pattern injection — pick DaemonSet for POC)
  - `webhook-receiver` (Deployment + Ingress for HTTPS)
  - `outbox-forwarder` (Deployment, single replica with leader-election TBD)
  - `ttl-sweeper` (Deployment, single replica)
- `values.yaml` schema:
  - Tenant id, budget id, window id, unit id (or seeded via init job)
  - Pricing freeze fields
  - PKI: option to use cert-manager + ClusterIssuer instead of pki-init
  - Postgres: option to point at external instance
- Init jobs use Bash + psql per existing demo init scripts — no new init code
- Helm post-install hook applies ledger migrations against the Postgres
- Probe configuration: liveness on /healthz, readiness on /readyz

### Implementation

1. `charts/spendguard/Chart.yaml` + dependencies
2. Templates for each Deployment, Service, ConfigMap, Secret
3. Helper templates for mTLS volume mount + PKI secret naming
4. `templates/job-migrate.yaml` post-install hook
5. `values.yaml` + `values.example.yaml` (full deployment) + `values.minimal.yaml` (single-pod test)
6. `helmignore`, `NOTES.txt` with quickstart commands after install

### Test plan

- `helm lint charts/spendguard`
- `kind create cluster && helm install spendguard ./charts/spendguard`
- Deploy a minimal pydantic-ai test client pod that talks to the in-cluster sidecar via UDS (volume mount)
- Run `client.request_decision(...)` and verify ledger row appears
- Pod restart test: `kubectl delete pod sidecar-*` and verify reservation TTLs recover (existing demo `ttl_sweep` mode behavior)

### Acceptance criteria

- [ ] `helm install` on a fresh kind / k3d cluster produces a healthy spendguard namespace within 2 minutes
- [ ] `helm upgrade` with new contract bundle hot-swaps without reservation loss (per Contract spec; if hot-reload not yet supported, document the gap)
- [ ] PKI secrets rotate without bringing down the stack (manual test acceptable)
- [ ] Migration job runs once per release and is idempotent

### Codex adversarial review focus

- **Multi-pod safety**: if `replicas > 1` for sidecar / outbox-forwarder, do producer_sequence races corrupt audit_outbox? POC is single-pod; spec says GA-blocking. Document with a `replicas: 1` guardrail in values.yaml + chart-level warning if user overrides
- **Fencing scope across pod restart**: existing POC limit — sidecar restart loses fencing lease. Helm chart needs to either pre-create fencing scope rows in DB seed OR document operator playbook for "restart sidecar" scenarios
- **Secret rotation**: how does cert-manager-rotated cert reach a running sidecar pod? CSI driver vs Secret reload?
- **Postgres dependency**: external vs bundled tradeoff — bundled is easier for POC trial but production users WILL want their own RDS / Cloud SQL. Schema migrations must be idempotent + backward compatible for the lifetime of one Helm release
- **Webhook receiver Ingress TLS**: who owns the cert? Demo uses self-signed; production uses Let's Encrypt via cert-manager — chart must support both

---

## Slice O3 — Pricing Table + Auto-Update

**Goal**: replace hardcoded `pricing_version=demo-pricing-v1` with a real per-(provider, model, token_kind) → $/1M-token table that updates without redeploys.

### Design

- New table `pricing_table` in canonical_ingest (or new `spendguard_pricing` DB):
  ```sql
  CREATE TABLE pricing_table (
    pricing_version TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    token_kind TEXT NOT NULL CHECK (token_kind IN
      ('input', 'output', 'cached_input', 'vision_input', 'audio_input')),
    price_usd_per_million_tokens NUMERIC(20, 8) NOT NULL,
    fetched_at TIMESTAMPTZ NOT NULL,
    source TEXT NOT NULL,  -- 'manual' | 'openai_pricing_api' | 'anthropic_console' | ...
    PRIMARY KEY (pricing_version, provider, model, token_kind)
  );
  ```
- New crate `services/pricing_sync/`:
  - Periodic poll (default 24hr) against per-provider source
  - Manual table override via YAML committed to repo for providers without pricing API
  - Writes new rows with monotonically-increasing `pricing_version`
  - Emits audit event when version bumps
- Sidecar bundle build picks up the latest `pricing_version` at bundle-time and freezes it (as today)
- Deprecation: Stage 2 §9.4 spec covers this; this slice is the implementation, not a new design

### Implementation

1. SQL migration adds `pricing_table` to canonical or a new pricing DB
2. Manual seed YAML at `deploy/pricing/seed.yaml` covering OpenAI + Anthropic models
3. `services/pricing_sync/` Rust crate that reads YAML + writes to DB
4. Bundle builder reads from DB at bundle build time + embeds pricing snapshot in metadata
5. SDK helper `spendguard.pricing.lookup(provider, model, kind) → $/1M` for adapters that need it locally

### Test plan

- Seed YAML for `gpt-4o-mini` input=$0.15/1M, output=$0.60/1M, cached_input=$0.075/1M
- Seed YAML for `claude-haiku-4-5` input=$1.00/1M, output=$5.00/1M
- `pricing_sync` runs once → DB has all rows
- Bundle build picks up `pricing_version=v2026.05.09-1`
- Test: a 100-input-token + 50-output-token request resolves to `(100 × 0.15 + 50 × 0.60) / 1M USD = 0.000045 USD = 45 micro-USD`
- Replay determinism: same `pricing_version` always returns same USD value

### Acceptance criteria

- [ ] `pricing_table` populated for at least OpenAI (gpt-4o, gpt-4o-mini, o1) + Anthropic (claude-haiku-4-5, claude-sonnet-4-5) + Azure OpenAI + Bedrock
- [ ] Bundle build references real pricing_version, not `demo-pricing-v1`
- [ ] Adapter can compute USD from a token count + model id without round-tripping to ledger
- [ ] Pricing version bump emits an audit event for compliance tracking

### Codex adversarial review focus

- **Stale pricing handling**: if pricing_sync fails for 7 days, does the system reject all decisions or coast on last-known-good? Design should pick fail-closed (reject) for safety, but with operator override
- **Race between bundle build and pricing update**: if bundle build reads pricing_version V at the same moment pricing_sync writes V+1, which version goes into the bundle? Explicit lock or monotonic version with snapshot isolation
- **Currency conversion**: spec mentions `fx_rate_version`. Pricing table is USD-only; what about EUR / GBP customers? Probably out of scope for this slice — note for future
- **Cached vs non-cached tokens**: OpenAI's "cached input" is half-price; if adapter doesn't distinguish, customer is overcharged by 2x on cached cases. Adapter contract must require token_kind precision
- **Tokenizer drift**: a request that's "100 tokens" by tiktoken might be "97 tokens" by model server's count. Which wins? Probably provider-reported (LLM_CALL_POST), but pre-call estimate uses local tokenizer — variance is real

---

## Slice O4 — USD-Denominated Budget

**Goal**: a single budget can be debited by both OpenAI and Anthropic calls (the cross-provider story).

**Prerequisite**: O3 (need real pricing table for sidecar to convert tokens → USD).

### Design

- New `unit_id`: `usd_micros` (1 unit = 1 micro-USD = $0.000001)
- BudgetClaim semantics change: `amount_atomic` now means "micro-USD this call will consume"
- Sidecar pre-claim computation:
  1. Adapter says "I'm calling gpt-4o-mini with 200 input + 100 output tokens (projected)"
  2. Sidecar looks up frozen pricing_version → `(200 × 0.15 + 100 × 0.60) / 1M = 75 µUSD`
  3. Reserves 75 µUSD against `usd_micros` budget
- Sidecar post-commit: provider returns actual usage; sidecar recomputes USD from actual tokens; commits real µUSD
- Multi-provider in same tenant:
  - One `available_budget` account in unit `usd_micros`
  - Both OpenAI calls and Anthropic calls debit the same account
- Token-denominated budgets stay supported (per-model token caps) via separate unit_ids; the USD path is additive, not replacing

### Implementation

1. Add `usd_micros` unit creation to demo seed
2. Adapter helper: `compute_usd_micros(model, input_tokens, output_tokens, pricing_version) → BigInt`
3. Update SpendGuardModel to call helper in PRE/POST hooks based on adapter config
4. New `DEMO_MODE=multi_provider` that calls both OpenAI and Anthropic in same agent session against same USD budget
5. Verify SQL: assert sum of OpenAI commit + Anthropic commit = total USD spent

### Test plan

- Seed budget with $5.00 = 5_000_000 µUSD in `usd_micros` unit
- Demo: agent calls gpt-4o-mini once (~10¢ worth) + claude-haiku-4-5 once (~12¢ worth)
- After: available_budget should be ~$4.78, committed_spend should be ~$0.22
- Edge case: a third call that would push over $5.00 → contract rule fires DENY

### Acceptance criteria

- [ ] Demo mode `multi_provider` runs end-to-end with two real providers
- [ ] Ledger committed_spend in µUSD matches sum of both providers' actual costs (within rounding tolerance)
- [ ] Hard-cap rule fires on cross-provider total, not per-provider

### Codex adversarial review focus

- **Rounding**: µUSD is fixed-point; tokens × price / 1M may produce sub-µUSD remainders. Round-up vs round-down? Round-up is fail-safe for the customer
- **Pricing freeze drift mid-session**: if pricing_version changes between PRE and POST of the same call, which one is authoritative? Spec says PRE freezes for the lifetime of the reservation; this slice must preserve that
- **FX**: if customer's invoice is in EUR but pricing_table is USD, fx_rate_version comes back into play. Probably out of scope for this slice — guard with explicit USD-only assertion
- **Contract rule expressiveness**: existing rules match on `claim_amount_atomic_gt` against a static threshold. With USD-denominated, threshold is now in µUSD — does the rule YAML need a `currency: USD` field? Or just convention?
- **Mixed-unit budget**: customer wants "$X total + max Y output tokens for gpt-4". Multi-claim ReserveSet handles this in spec but POC is single-claim. Document gap

---

## Slice O5 — LangChain Adapter

**Goal**: a LangChain user wraps `init_chat_model("gpt-4o-mini")` with a SpendGuard middleware and gets the same lifecycle.

**Prerequisite**: O1 (SDK package).

### Design

- Add `spendguard.integrations.langchain` module
- LangChain has multiple integration points:
  - **Runnable middleware**: wraps a Runnable, intercepts `.invoke()` / `.ainvoke()`
  - **Chat model subclass**: subclass of `BaseChatModel` that delegates to a provider model + emits SpendGuard events
  - **Callback handler**: less invasive but harder to gate (callbacks fire after the call)
- Pick chat-model-subclass for clearest gating semantics:
  ```python
  from spendguard.integrations.langchain import SpendGuardChatModel
  
  guarded = SpendGuardChatModel(
      inner=ChatOpenAI(model="gpt-4o-mini"),
      client=client,
      budget_id=...,
      claim_estimator=lambda messages: [...],
  )
  
  agent = create_react_agent(guarded, tools=[...])
  ```
- LangChain async/sync split: support both `_generate` and `_agenerate`

### Implementation

1. New module file in SDK
2. LangChain `BaseChatModel` subclass that wraps `inner` chat model
3. Same SpendGuardClient + RequestDecision + LLM_CALL_POST flow as Pydantic-AI version
4. Token usage extraction: LangChain returns `AIMessage.usage_metadata` with input/output/total tokens
5. Demo mode `agent_real_langchain` that mirrors `agent_real`

### Test plan

- LangChain ReAct agent with single tool, runs against real OpenAI
- Verify ledger lifecycle: reserve → commit_estimated with real usage
- Verify DENY path: configure rule to deny → LangChain agent raises proper exception (not silent fall-through)
- Compare with Pydantic-AI behavior: same prompt should produce same ledger rows (modulo non-determinism of LLM)

### Acceptance criteria

- [ ] `pip install spendguard-sdk[langchain]` works
- [ ] LangChain agent end-to-end demo PASS with real provider
- [ ] DENY path raises a LangChain-friendly exception type (not generic SpendGuardError)
- [ ] Streaming path also gated (LangChain `stream()` / `astream()` should still emit PRE/POST events)

### Codex adversarial review focus

- **Streaming**: LangChain heavily uses streaming. Token counts may not be available until `astream` finalizes — when does LLM_CALL_POST fire? Spec says POST is at "boundary close", which for streaming is final-chunk; verify SpendGuardChatModel actually emits POST then, not at request start
- **Tool calls**: LangChain's tool-calling agents make multiple LLM calls per `agent.invoke()`. Each one is a separate LLM_CALL_PRE/POST? Spec yes, but verify the adapter doesn't collapse them
- **Retry / fallback**: LangChain has built-in retry on RateLimitError. Each retry is a separate reservation? If YES, retry loop can drain budget. If NO, retried call must use original reservation (replay path)
- **Async event loop**: LangChain async path uses different message shapes than Pydantic-AI; SDK helpers must work for both
- **DEGRADE path**: if a contract rule fires DEGRADE (e.g., "force smaller model"), LangChain's BaseChatModel can't easily swap models mid-call — what does the adapter do? Probably just CONTINUE for POC, document DEGRADE as not-yet-supported in LangChain integration

---

## Slice O6 — Microsoft AGT PolicyEngine Plugin

**Goal**: SpendGuard becomes a budget-evaluator plugin that AGT users can wire into their PolicyEngine.

**Prerequisite**: O1 (SDK).

### Design

- Add `spendguard.integrations.agt` module (Python first since AGT primary SDK is Python)
- Pattern A: AGT `PolicyAction` extension
  - AGT's `PolicyDocument` has rules with `action: ALLOW | DENY`
  - We extend with a virtual action `BUDGET_EVALUATE` that calls SpendGuard sidecar before returning ALLOW/DENY
  - Implementation: a custom `PolicyEvaluator` subclass that intercepts BUDGET_EVALUATE rules
- Pattern B: separate evaluator chain
  - User's app calls `agt_evaluator.evaluate(...)` first, then `spendguard_client.request_decision(...)` second
  - Two audit chains; harder to reconcile
- Pick Pattern A: cleaner, single audit per request. Document Pattern B as fallback for users who want SpendGuard pre-AGT

### Implementation

1. New module in SDK with AGT-compatible `PolicyEvaluator` subclass
2. YAML extension: `action: BUDGET_EVALUATE` with embedded SpendGuard config (budget_id, etc.)
3. Demo: an AGT-governed agent (using AGT's quickstart) + SpendGuardChatModel + a YAML policy that combines tool-name DENY (AGT) and budget DENY (SpendGuard)
4. Audit alignment: AGT writes to its own audit log; we write to ours. Document the dual-audit story OR add a `spendguard.audit.relay_to_agt(event)` helper

### Test plan

- Reuse AGT's Python quickstart from their README
- Add a SpendGuard policy rule + budget seed
- Run a tool call that AGT allows but SpendGuard denies (over budget) → expect DENY with reason from SpendGuard
- Run a tool call that SpendGuard allows but AGT denies (e.g., `delete_file`) → expect DENY with reason from AGT
- Verify both audit chains receive their respective records

### Acceptance criteria

- [ ] AGT-compatible plugin works against AGT's `agt verify` CLI
- [ ] Combined policy YAML (AGT rules + SpendGuard `BUDGET_EVALUATE`) parses cleanly
- [ ] Demo shows both DENY directions (AGT-deny vs SpendGuard-deny)
- [ ] Documentation references AGT's docs site for "how to deploy AGT" (don't duplicate)

### Codex adversarial review focus

- **Identity mismatch**: AGT uses SPIFFE/SVID for agent identity; SpendGuard uses fencing scope. Are they reconcilable? Probably yes via mapping table (spiffe_id → fencing_scope_id), but call this out
- **Audit duplication**: same decision producing two audit rows in two systems is fine for compliance but expensive. Future work: AGT-event-receiver that ingests SpendGuard audit events
- **Policy evaluation order**: AGT first then SpendGuard, or vice versa? Bug risk: an action AGT-denies that SpendGuard pre-reserves leaves a stale reservation. Order must be: AGT first (cheap), SpendGuard second (expensive but conditional on AGT-allow)
- **Latency budget**: AGT promises < 0.1ms; SpendGuard promises < 5ms. Combined < 5.1ms is acceptable for most agents but document
- **MIT license compatibility**: SpendGuard's license vs AGT's MIT. If we open-source the plugin, double-check no GPL transitive deps

---

## Slice O7 — Operator Dashboard MVP

**Goal**: a tenant operator can see budget burn, recent decisions, DENY reasons in a browser without writing SQL.

**Prerequisite**: O4 (USD budget so the dashboard's $-axis is meaningful).

### Design

- New service `services/dashboard/` — Rust + axum + minimal HTML/HTMX for POC (or FastAPI if we want to share client.py)
- Read-only views:
  1. **Budget overview**: per-tenant available / reserved / committed (last 7 days timeline)
  2. **Recent decisions**: last 100 decisions with kind (CONTINUE / STOP / etc.), reason_codes, matched_rule_ids, $ amount
  3. **DENY reasons**: histogram of reason_codes by hour for the past 24h
  4. **Audit chain health**: outbox forwarder lag, oldest-pending-row, canonical_events delta
- Auth: bearer token from the same Entra-issued JWT (tied to O8) — for MVP, hardcoded basic auth + per-tenant URL token
- All queries hit ledger DB read replica (not primary) to avoid impact

### Implementation

1. Service skeleton with axum + sqlx
2. 4 view templates (HTMX-friendly, no SPA framework)
3. SQL queries against ledger + audit_outbox + canonical_events
4. Compose service entry + Helm template
5. CSS: minimal Pico.css or similar; this is operator UI, not consumer

### Test plan

- Dashboard loads against running demo
- "Recent decisions" view shows DENY row from `agent_real_anthropic` test
- After running `agent_real`, "Budget overview" shows correct $ debit
- Outbox forwarder lag view shows realistic numbers (≈ poll interval)

### Acceptance criteria

- [ ] Read-only dashboard with 4 views
- [ ] Per-tenant URL pattern (one tenant per URL)
- [ ] Auth gate (any token-based)
- [ ] No direct DB credentials shipped; reads via internal RPC

### Codex adversarial review focus

- **Multi-tenant isolation**: a single dashboard instance serving N tenants must NEVER cross tenant_id boundary. SQL queries must always parameterize tenant_id; verify with adversarial query injection test
- **Auth**: bearer token in URL is bad practice; should be Authorization header. POC may slip but call out
- **Read replica lag**: if dashboard reads from a replica with 5s lag, "Recent decisions" misses just-committed rows. Either use primary for last-N-seconds or label data freshness
- **Performance**: a tenant with 1M audit_outbox rows querying "last 100 decisions" naively scans the table. Need indexes; verify with EXPLAIN
- **PII**: contract rule names + matched_rule_ids may leak business intent (e.g., "competitor_blocklist_rule"). Consider role-based field hiding for non-admin operators
- **Budget alerting**: dashboard shows current state but doesn't push alerts. Future feature; document as not in MVP

---

## Slice O8 — SaaS Control Plane (Tenant Provisioning API)

**Goal**: a new customer signs up via API/UI and gets a working tenant within 60 seconds.

**Prerequisites**: O1, O2, O3, O4, O7 (most foundation).

### Design

- New service `services/control_plane/` (Rust + axum + tonic clients)
- REST API surface:
  - `POST /v1/tenants` → creates tenant, fencing scope, default budget, seed deposit (returns API key + sidecar bootstrap config)
  - `GET /v1/tenants/{id}` → tenant overview (proxy to dashboard data)
  - `POST /v1/tenants/{id}/budgets` → create additional budget under tenant
  - `POST /v1/tenants/{id}/contracts` → upload contract YAML, build bundle, sign, push to bundle store
  - `DELETE /v1/tenants/{id}` → soft-delete (mark tombstoned, keep audit chain immutable)
- Auth: Microsoft Entra ID (OIDC) for human admins; API keys for programmatic
- Stripe / billing integration: out of scope for MVP, just track $ committed; future slice attaches invoicing

### Implementation

1. Service skeleton with axum + sqlx + tonic clients to ledger / canonical-ingest
2. Provisioning workflow: insert tenant + fencing scope + budget + seed deposit in single transaction
3. Contract bundling: receive YAML body, build .tgz, sigstore-sign, write to S3-compatible store, push reference to bundle store
4. Entra OIDC integration via standard middleware
5. Basic admin UI (HTMX) for human flow
6. Helm chart entry

### Test plan

- `curl -X POST /v1/tenants -d {"name": "acme"}` → returns config block
- Use returned config to bootstrap a sidecar locally → can issue decisions
- `curl -X POST /v1/tenants/{id}/contracts -d @policy.yaml` → contract live within 60s
- Tombstoning: `DELETE /v1/tenants/{id}` then attempt new decision → fails closed

### Acceptance criteria

- [ ] Full self-service onboarding from POST → working sidecar
- [ ] Per-tenant API keys with scope (read-only vs write)
- [ ] Contract upload + bundle pipeline integrated
- [ ] Tombstoning preserves audit chain immutability

### Codex adversarial review focus

- **Tenant id leakage**: every query / event MUST be tenant-scoped at the SP level, not just at the API boundary. Penetration test: forge tenant_id in API call, verify rejection
- **Bootstrap secrets**: returning API key once-only is correct; verify no logging of the key value, no DB plaintext
- **Bundle signing**: control plane has the signing key — that's a privileged identity. Key rotation? KMS-backed? sigstore Fulcio?
- **Race between tenant create and first decision**: provisioning is multi-step; if API returns success before all rows are committed, first decision fails. Use single transaction or saga
- **Contract YAML uploaded by user**: arbitrary YAML can blow up parser (billion-laughs, deep nesting, etc.). YAML parser must have hard limits
- **Compliance**: tenants in regulated industries need data residency. EU tenants must have EU Postgres replica; US tenants vice versa. Out of scope for MVP; document
- **Quota / abuse**: a malicious tenant can flood requests, exhaust shared resources. Per-tenant rate limits at API gateway

---

## Slice O9 — Terraform Module

**Goal**: `terraform apply` provisions all cloud infra needed for SpendGuard on AWS / GCP / Azure.

**Prerequisite**: O2 (Helm chart needs cluster + DB + secrets stored).

### Design

- One module per cloud: `terraform/aws/`, `terraform/gcp/`, `terraform/azure/`
- Each provisions:
  - Managed k8s (EKS / GKE / AKS)
  - Managed Postgres (RDS / Cloud SQL / Azure DB)
  - Managed secrets (Secrets Manager / Secret Manager / Key Vault)
  - Bundle store (S3 / GCS / Blob)
  - Optional: managed cert (ACM / Certificate Manager / Key Vault)
- Outputs: kubeconfig, Postgres connection string (via secret reference), bundle store URL
- Helm chart consumes outputs via wrapper module

### Implementation

1. AWS module first (largest market)
2. GCP module
3. Azure module last (most complex IAM; AKS + Entra integration overlap with O8)
4. Wrapper module that composes Terraform + Helm provider for one-shot apply

### Test plan

- `terraform apply -var-file=test.tfvars` in a sandbox account; full deployment up
- E2E demo runs against deployed stack
- `terraform destroy` cleanly removes all resources (no orphans)
- Cost: per-day cost should be under $50 for minimum config (alert if over)

### Acceptance criteria

- [ ] AWS module fully working; tested in sandbox account
- [ ] Module documented with `tfvars` example and required IAM permissions
- [ ] State backend: S3 + DynamoDB lock for AWS; equivalents for GCP/Azure
- [ ] No hardcoded resource names that prevent multi-deployment-per-account

### Codex adversarial review focus

- **IAM blast radius**: apply requires admin? Or scoped? Document principle-of-least-privilege IAM doc
- **Network exposure**: by default, is webhook receiver public? Yes (it has to be — providers POST to it). Network policy must restrict source IPs to provider CIDR ranges
- **Secrets in state**: Terraform state is sensitive. Backend MUST be encrypted (S3 server-side encryption + KMS); state MUST NOT be committed
- **Cost overrun**: a misconfigured Cloud SQL with multi-AZ + high CPU can cost $1k+/mo. Module defaults must be cheap; expensive options opt-in
- **Multi-region**: spec mentions cross-region failover as out-of-POC. If user requests it, point to GA roadmap

---

## Slice O10 — Documentation Site

**Goal**: a developer reading docs.spendguard.io can go from zero to a deny-mode demo in 5 minutes without reading the spec.

**Continuous**: refreshed after each slice ships.

### Design

- mkdocs-material site at `docs/site/` published via GitHub Pages
- Information architecture:
  - **Quickstart** (5min): docker compose path
  - **Concepts**: 6 primitives (T→L→C→D→E→P) with one paragraph each + diagram
  - **Deployment**: 5 deployment modes (k8s_saas / self_hosted / lambda / cloud_run / air_gapped)
  - **Authoring contracts**: contract.yaml schema, examples, evolution path
  - **Adapter integrations**: Pydantic-AI, LangChain, AGT, OpenAI Agents (as they ship)
  - **Operations**: dashboard, alerts, runbook
  - **Reference**: API surface, proto, ledger schema, error codes
  - **Security**: threat model, audit invariants
  - **POC vs GA gates**: explicit list of what's not yet production-ready
- Auto-generated API ref from proto + Rust docstrings
- Versioned docs (mike or similar) so v0.1 docs survive past v0.2

### Implementation

1. mkdocs scaffold with the navigation structure
2. Migrate existing PHASE_2B_CHECKPOINT.md + spec docs into this structure (preserve as reference, not primary content)
3. Write quickstart (~500 words), concepts (~1500 words), authoring guide (~2000 words)
4. Auto-build on each slice merge; tag docs version with each release
5. Search via Algolia DocSearch (free for OSS)

### Test plan

- A new dev runs through quickstart on a clean machine; reaches green DENY demo in < 10 minutes
- All code blocks in quickstart actually execute
- Search works
- Mobile rendering works

### Acceptance criteria

- [ ] docs.spendguard.io live with 8+ pages
- [ ] Quickstart tested by someone other than the author
- [ ] Each slice's "what landed" gets a release-notes entry within 24 hours of merge
- [ ] No 404s in internal links

### Codex adversarial review focus

- **Drift**: docs that lie are worse than no docs. CI must lint that all referenced commands / file paths actually exist in the repo
- **Spec vs docs**: spec docs (`docs/contract-dsl-spec-v1alpha1.md` etc) are LOCKED; docs site is mutable. Any conflict: spec wins, docs site references the spec
- **Examples must run**: every code block tagged `bash` / `python` / `yaml` should be executable / parseable; dedicated CI job
- **POC honesty**: every page that demos a feature must clearly mark POC limits (per Phase 2B Checkpoint convention)
- **i18n**: AGT ships with 4 languages from day 1. We're zh-TW + en at minimum given user's primary language; others can wait

---

## Per-slice ship template

Every slice follows the same lifecycle (per `feedback_codex_review.md` + `feedback_demo_quality_gate.md`):

1. **Design round** — strawman in `/tmp/<slice-id>-r1.md`, send to Codex adversarial review
2. **Lock** — fold P0/P1 findings, mark spec LOCKED in commit message
3. **Implement** — keep diffs small; modular per file
4. **Demo gate** — wire into deploy/demo or new equivalent; PASS assertion in verify SQL or test script
5. **Codex implementation challenge** — second Codex round on the patch
6. **Document** — update O10 + memory `project_overview.md`
7. **Push to PR** — append commit to `feat/onboarding-O<N>-...` branch

---

## Cross-cutting concerns

These apply to multiple slices; centralized here:

### Security
- Every new service follows existing mTLS pattern; no plain HTTP except `/healthz` per spec
- Secrets via mounted volumes (k8s Secret) or env from secrets manager; never plaintext in compose
- Per-tenant identity propagation: tenant_id must travel with every RPC and audit row

### Operability
- Every service emits Prometheus metrics on `/metrics`
- Tracing via OpenTelemetry; spans propagated via gRPC metadata
- Logs are JSON; correlation via `decision_id` / `audit_decision_event_id`

### Backward compatibility
- Wire protocol (proto) version pinned; bump on breaking change with deprecation window
- Migration files immutable post-merge; new changes via new migrations
- Adapter SDK semver

### Testing
- Every slice ships with at least one demo mode that exercises the new feature end-to-end
- POC tolerates known limits but never silent regressions in existing modes (CI runs all `DEMO_MODE=*`)

---

## Phase 4 closure criteria

Phase 4 is "done" when:

- [ ] All 10 slices have shipped to PR + are documented in O10
- [ ] At least one external developer has successfully run quickstart end-to-end (validation, not just internal)
- [ ] The honest "no onboarding flow" assessment from 2026-05-09 no longer applies — `pip install` + `helm install` is real
- [ ] At least one design partner (LangChain user, AGT user, or LangGraph user) has wired SpendGuard against their stack with documented feedback

What Phase 4 does **NOT** close (still GA gates per checkpoint §3.1):
- Multi-pod work distribution (producer_sequence races)
- Real signing key rotation (strict_signatures=true)
- Chaos test suite per Stage 2 §13
- ORPHAN_OUTCOME reaper
- CI quarantine durability

These remain Phase 5 (GA hardening).
