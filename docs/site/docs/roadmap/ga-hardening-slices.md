# GA hardening slice plan

Status: planning artifact.

Purpose: split the remaining production blockers into small,
reviewable slices. Each slice must be independently understandable,
small enough for one PR, and gated by design review, implementation
review, test evidence, and operational acceptance.

This plan turns these product requirements into engineering work:

1. Multi-pod safety, leader election, and fencing acquire.
2. Real signing keys, KMS/key rotation, and audit export.
3. Provider usage reconciliation and pricing auto-update.
4. Real `REQUIRE_APPROVAL` workflow.
5. SSO/RBAC, tenant isolation, and retention policy.
6. One-workflow onboarding in half a day.
7. Explicit fail-open/fail-closed policy and SLOs.

## Global definition of done

A slice is complete only when all of these are true:

- Design: the PR includes a short design note or links to this document
  with the selected design, rejected alternatives, failure modes, and
  backward-compatibility impact.
- Implementation: config flags, migrations, proto changes, service code,
  SDK changes, chart/Terraform changes, and docs are in the same slice
  when they are needed for the behavior to work.
- Tests: unit tests cover pure logic; integration tests cover DB/gRPC
  contracts; one end-to-end demo or verification script proves the user
  workflow.
- Operations: metrics, logs, health/readiness semantics, rollback notes,
  and runbook updates are included for hot-path behavior.
- Security: identity, tenant boundary, key handling, and data retention
  impacts are reviewed.
- Review: an adversarial reviewer can answer "what breaks if this fails"
  from the PR without asking the author.

## Dependency map

```text
S1 leader lease primitive
  -> S2 singleton workers use leases
  -> S5 multi-pod chart enablement

S3 ledger AcquireFencingLease RPC
  -> S4 sidecar lease lifecycle
  -> S5 multi-pod chart enablement

S6 signing abstraction
  -> S7 key registry + rotation
  -> S8 strict canonical verification
  -> S9 audit export

S10 provider usage ingestion
  -> S11 OpenAI usage reconciliation
  -> S12 Anthropic/provider reconciliation

S13 pricing authority auto-update
  -> S11/S12 reconciliation accuracy

S14 approval state model
  -> S15 approval API/UI
  -> S16 adapter resume/deny/timeout

S17 SSO foundation
  -> S18 RBAC + tenant isolation
  -> S19 retention/redaction policy

S20 onboarding templates
  -> S21 doctor/readiness verifier

S22 fail policy matrix
  -> S23 SLOs, alerts, and incident drills
```

## Track A - Multi-pod, leader election, fencing

### S1 - Lease primitive for singleton background workers

Goal: provide one shared lease primitive that `outbox-forwarder`,
`ttl-sweeper`, and future pollers can use before any multi-pod replica
is allowed to process work.

Design:

- Use Kubernetes Lease API when running in k8s.
- Use Postgres row-lock lease as the non-k8s fallback for compose and
  local integration tests.
- Lease identity is `(service_name, workload_instance_id, region)`.
- Lease state must expose `leader`, `standby`, and `unknown`.
- Workers must process batches only while `leader`.
- Lost lease must stop new work before the next poll interval.

Implementation:

- Add a small lease module shared by singleton workers.
- Add config:
  - `SPENDGUARD_LEADER_ELECTION_MODE=k8s|postgres|disabled`.
  - `SPENDGUARD_LEADER_LEASE_NAME`.
  - `SPENDGUARD_LEADER_RENEW_INTERVAL_MS`.
  - `SPENDGUARD_LEADER_TTL_MS`.
- Wire it into `services/outbox_forwarder` and `services/ttl_sweeper`.
- Add Helm RBAC for `coordination.k8s.io/Lease`.
- Keep `disabled` allowed only when replicas are exactly `1`.

Test and acceptance:

- Unit: lease state transitions for acquire, renew, expire, and conflict.
- Integration: start two worker instances against the same lease; only
  one processes rows.
- Chaos: kill the leader process; standby takes over after TTL without
  double-forwarding an audit row.
- Helm: `replicas=2` is rejected unless leader election is enabled.

Review standard:

- Verify no worker does real work before a lease is acquired.
- Verify lost lease stops future batches and does not interrupt an
  already committed DB transaction.
- Verify lease TTL is longer than renew interval and shorter than the
  operational recovery objective.
- Verify logs and metrics identify the active leader.

### S2 - Producer sequence partitioning

Goal: remove `producer_sequence` races when more than one instance of a
producer exists.

Design:

- `producer_sequence` allocation must be scoped by stable
  `producer_instance_id`, not by service name alone.
- Canonical ingest still enforces per-producer monotonic sequence.
- For sidecar pods, sequence identity is the workload instance id.
- For singleton workers, sequence identity is the current leader
  workload instance id.

Implementation:

- Review `audit_outbox_global_keys` and producer sequence allocation.
- Add migration if the current unique key is too coarse.
- Make each producer emit `producer_instance_id` in every audit outbox
  row.
- Update canonical ingest validation to reject missing or ambiguous
  instance identity.
- Update demo seed data and chart env vars.

Test and acceptance:

- Integration: two sidecars reserve against different tenant/budget
  scopes without sequence collision.
- Integration: two forwarders with leader election enabled never forward
  the same audit row twice.
- Negative: two producers using the same `producer_instance_id` fail
  closed with a clear error.

Review standard:

- Verify sequence allocation is deterministic after restart.
- Verify idempotent replay does not allocate a new sequence.
- Verify DB constraints protect the invariant even if application code
  is wrong.

### S3 - Ledger `AcquireFencingLease` RPC

Goal: replace seeded `current_epoch=1` fencing with a ledger-owned CAS
lease acquisition API.

Design:

- Ledger remains the single source of fencing authority.
- New RPC: `AcquireFencingLease`.
- Request includes `scope_id`, `tenant_id`, `workload_instance_id`,
  `requested_ttl`, and expected owner semantics.
- Response returns `epoch`, `ttl_expires_at`, and lease state.
- Re-acquire by the current owner renews TTL without changing epoch.
- Takeover after expiry increments epoch.
- Takeover before expiry is rejected unless an explicit administrative
  force flag is used.

Implementation:

- Add proto messages and RPC in `proto/spendguard/ledger/v1/ledger.proto`.
- Add ledger handler and persistence function.
- Add SQL function or transaction block that locks the fencing row,
  verifies owner/expiry, increments epoch on takeover, and appends a
  `fencing_scope_events` row.
- Return `FENCING_EPOCH_STALE` or a dedicated lease conflict error for
  unsafe acquisition.
- Add SDK/client support where needed by sidecar and workers.

Test and acceptance:

- Unit: current owner renew does not increment epoch.
- Integration: expired owner takeover increments epoch exactly once.
- Race: two contenders acquire the same expired scope; only one wins.
- Ledger: every successful acquire writes a history event.
- Regression: existing ReserveSet rejects epoch `0`.

Review standard:

- Verify CAS is fully inside one DB transaction.
- Verify no caller can mint its own epoch.
- Verify stale owners cannot write after takeover.
- Verify error codes are stable for adapters and runbooks.

### S4 - Sidecar lease lifecycle

Goal: sidecar must acquire, renew, and stop using a fencing lease before
it issues any ledger mutation.

Design:

- Sidecar startup blocks decision processing until lease acquire
  succeeds.
- Lease renew runs in the background.
- If renew fails past the grace window, sidecar enters `draining` and
  rejects new decisions fail-closed.
- Existing reservations can only be committed/released if the current
  epoch is still valid.
- Readiness returns unhealthy when no valid lease exists.

Implementation:

- Wire `AcquireFencingLease` into `services/sidecar` bootstrap.
- Replace static `SPENDGUARD_SIDECAR_FENCING_INITIAL_EPOCH` usage.
- Store active lease in sidecar state.
- Add renewal task and shutdown/drain coordination.
- Update Helm values and control plane `sidecar_config_env`.

Test and acceptance:

- E2E: fresh sidecar starts with epoch from ledger and runs `deny` demo.
- Restart: sidecar restarts, renews/acquires correctly, no epoch reuse.
- Stale: force DB epoch forward, next decision fails closed.
- Readiness: sidecar reports not ready when lease is absent/stale.

Review standard:

- Verify no decision path can use epoch `0` or a stale cached epoch.
- Verify readiness and liveness differ: stale lease is not ready, but
  process can stay alive to recover.
- Verify shutdown drains in-flight decisions before dropping lease.

### S5 - Multi-pod enablement gate

Goal: safely allow more than one replica for components after S1-S4 are
done.

Design:

- Chart defaults can remain conservative, but the chart must stop
  calling multi-pod a GA blocker after invariants are tested.
- Sidecar scaling is allowed only when each pod has a unique workload
  instance id and matching fencing scope ownership.
- Worker scaling is allowed only with leader election enabled.

Implementation:

- Update `charts/spendguard/values.yaml` validation rules.
- Add Helm template checks for replica/lease combinations.
- Add deployment docs for sidecar, outbox-forwarder, ttl-sweeper.
- Add operational runbook for leader changes and fencing takeovers.

Test and acceptance:

- `helm template` rejects unsafe values.
- Kind test: two sidecars, two forwarders, two sweepers, all healthy.
- Chaos: delete active leader pod and verify takeover without duplicate
  audit rows.
- Replay: audit chain remains contiguous after leader failover.

Review standard:

- Verify every multi-pod mode has a deterministic identity story.
- Verify dashboards expose current leader and lease age.
- Verify rollback to single-pod mode does not require DB surgery.

## Track B - Signing, KMS, key rotation, audit export

### S6 - Producer signing abstraction

Goal: replace placeholder signatures with a concrete signing interface.

Design:

- All audit-producing services sign canonical payload bytes before they
  write to `audit_outbox` or submit to canonical ingest.
- Development signer supports local Ed25519 keys from file.
- Production signer supports KMS-backed signing through an interface,
  not provider-specific calls embedded in handlers.
- Signature metadata includes `key_id`, algorithm, created time, and
  producer identity.

Implementation:

- Add signing trait/module shared by ledger, sidecar, webhook receiver,
  outbox-forwarder if needed.
- Add local Ed25519 signer implementation.
- Add config for `SIGNING_MODE=local|kms|disabled`.
- Keep `disabled` allowed only for demo/test profiles.
- Update audit outbox row builder to require signature metadata.

Test and acceptance:

- Unit: canonical payload produces stable signature.
- Integration: ledger writes non-empty signatures.
- Negative: missing signing config refuses startup outside demo.
- Compatibility: existing demo can still run with explicit demo mode.

Review standard:

- Verify signing input is canonical and excludes mutable transport
  fields.
- Verify private keys never appear in logs or audit rows.
- Verify disabled signing cannot be accidentally enabled in Helm
  production profile.

### S7 - Key registry and rotation

Goal: support key discovery, verification, rotation, and revocation.

Design:

- Key registry stores public keys and validity windows.
- Producers sign with an active private key.
- Verifiers accept signatures only when event time is inside the key
  validity window.
- Rotation is additive first, then cutover, then revoke after retention
  overlap.
- KMS implementation is provider-pluggable.

Implementation:

- Add `signing_keys` table or reuse endpoint catalog if it already owns
  signed manifests.
- Add publish/rotate command for local keys.
- Add AWS KMS implementation first if Terraform AWS is the production
  target; leave Azure/GCP as interface-compatible future work.
- Add Helm/Terraform variables for key ids.
- Add runbook for rotation and emergency revocation.

Test and acceptance:

- Integration: old and new keys verify during overlap.
- Negative: event signed by expired/revoked key is rejected.
- Rotation drill: rotate key without service downtime.
- Audit: rotation itself emits an audit event.

Review standard:

- Verify key validity is evaluated against signed event time, not ingest
  wall clock alone.
- Verify rotation cannot orphan old audit records.
- Verify KMS permissions are least privilege: sign only, no key admin in
  runtime pods.

### S8 - Strict canonical signature verification

Goal: make `canonical_ingest.strict_signatures=true` production-ready.

Design:

- Canonical ingest verifies producer signature before accepting events.
- Invalid signatures go to quarantine with enough metadata to debug,
  but never enter immutable audit log.
- Strict mode is default for non-demo deployments.

Implementation:

- Complete canonical ingest verification against key registry.
- Implement `audit_quarantine` persistence for invalid signatures and
  schema failures.
- Add metrics for accepted, rejected, quarantined, and unknown-key
  events.
- Update docs and chart values.

Test and acceptance:

- Valid signed event is accepted.
- Invalid signature is quarantined.
- Unknown key is quarantined.
- Strict mode cannot be disabled by chart defaults in production
  profile.

Review standard:

- Verify quarantine rows are immutable enough for forensics.
- Verify rejected events cannot be replayed into canonical log after
  mutation.
- Verify metrics distinguish operator error from attacker-like input.

### S9 - Audit export

Goal: provide customer-usable audit export without weakening the
canonical chain.

Design:

- Export is read-only from canonical events.
- Supported sinks: object storage first; SIEM/webhook later.
- Export format includes payload, signature metadata, chain pointers,
  tenant id, event type, and verification status.
- Exports are resumable by cursor and tenant scoped.

Implementation:

- Add `audit-exporter` worker or control plane endpoint.
- Add object storage sink config and retention tags.
- Add export manifest with hash of exported batch.
- Add CLI or API to verify exported batch integrity.
- Update RBAC in S18 when available; until then use admin token only.

Test and acceptance:

- Export a tenant/date range and verify hashes.
- Resume after interrupted export without duplicate missing records.
- Negative: tenant A cannot export tenant B.
- SIEM placeholder docs clearly mark unsupported sink types.

Review standard:

- Verify export does not expose prompt/payload fields beyond retention
  policy.
- Verify cursor semantics are stable.
- Verify export errors do not block the hot decision path.

## Track C - Provider reconciliation and pricing

### S10 - Provider usage ingestion foundation

Goal: define one normalized ingestion path for provider usage records.

Design:

- Provider usage records are not trusted to mutate ledger directly.
- They enter as provider reports, are matched to reservations, then
  reconciled by ledger procedures.
- Matching keys include provider, model, request id, llm_call_id,
  run_id, tenant id, and time window.
- Unmatched usage goes to reconciliation quarantine.

Implementation:

- Add normalized provider usage schema.
- Extend webhook receiver or create `provider-usage-ingest`.
- Add quarantine table for unmatched/ambiguous usage.
- Add idempotency key derivation for provider records.
- Document provider-specific evidence limitations.

Test and acceptance:

- Duplicate provider usage record is idempotent.
- Unmatched record is quarantined.
- Matched record produces provider_report/invoice flow.
- Ambiguous match fails closed for ledger mutation.

Review standard:

- Verify provider data cannot bypass ledger validation.
- Verify matching logic is deterministic and explainable.
- Verify all provider records preserve raw source evidence.

### S11 - OpenAI usage poller and reconciliation

Goal: reconcile OpenAI usage even when there is no billing webhook.

Design:

- Poll OpenAI usage APIs on a schedule with cursor/window overlap.
- Normalize costs and usage into S10 schema.
- Use provider request ids and local llm_call ids where available.
- Late-arriving usage updates are reconciled idempotently.

Implementation:

- Add OpenAI poller adapter.
- Add config for org/project/key scope.
- Add rate-limit/backoff and partial failure handling.
- Add reconciliation report view for matched, unmatched, and adjusted
  costs.

Test and acceptance:

- Mock OpenAI usage response imports successfully.
- Re-running the same window is idempotent.
- Late usage changes create an adjustment, not duplicate spend.
- API outage preserves last successful cursor and alerts.

Review standard:

- Verify API credentials are tenant scoped or clearly operator scoped.
- Verify no prompt content is fetched unless explicitly required.
- Verify price used for reconciliation matches frozen pricing policy or
  records the delta reason.

### S12 - Anthropic and generic provider reconciliation

Goal: add a second real provider and prove the reconciliation model is
not OpenAI-specific.

Design:

- Anthropic implementation should reuse S10 primitives.
- If provider has webhook support, validate provider signatures.
- If provider only has usage export/polling, use the same cursor model
  as S11.
- Keep provider adapters small and declarative where possible.

Implementation:

- Add Anthropic adapter.
- Add provider signing verification if webhook path is available.
- Add provider-specific model/token-kind mapping.
- Add docs for adding future providers.

Test and acceptance:

- Mock Anthropic usage reconciles into the same USD budget as OpenAI.
- Bad webhook signature is rejected/quarantined.
- Provider-specific token kinds map correctly.
- Multi-provider demo still passes.

Review standard:

- Verify no provider-specific assumptions leak into ledger core.
- Verify provider raw payloads are retained per retention policy.
- Verify errors identify provider and tenant without leaking secrets.

### S13 - Pricing authority auto-update

Goal: replace static pricing YAML with a controlled pricing authority
that refreshes and freezes price snapshots.

Design:

- Pricing updates are never read live in the hot path.
- Pricing authority creates immutable `pricing_version` snapshots.
- Bundle build freezes `pricing_version`, `price_snapshot_hash`,
  `fx_rate_version`, and `unit_conversion_version`.
- Sidecar uses cached snapshot; ledger validates snapshot identity.
- Stale pricing policy is explicit: continue with last-known-good until
  max staleness, then fail closed unless operator override is active.

Implementation:

- Add pricing sync worker or command.
- Add source adapters for official provider pricing pages/APIs where
  available and manual overrides where not.
- Add snapshot hash computation and signing.
- Add chart config for max staleness and override.
- Add dashboard/control plane view for active pricing versions.

Test and acceptance:

- Pricing sync creates an immutable new version.
- Same inputs produce the same snapshot hash.
- Bundle build reads one consistent snapshot.
- Ledger rejects unknown pricing version/hash.
- Stale pricing beyond policy blocks new bundles or decisions according
  to the configured fail policy.

Review standard:

- Verify update races cannot produce mixed pricing snapshots.
- Verify manual override requires audit event and reviewer identity.
- Verify cached-input, output, reasoning, multimodal, and provider
  aliases are represented or explicitly unsupported.

## Track D - Real approval workflow

### S14 - Approval state model

Goal: make `REQUIRE_APPROVAL` a resumable state, not a terminal POC
result.

Design:

- Approval request is a first-class record with state:
  `pending`, `approved`, `denied`, `expired`, `cancelled`.
- Approval has TTL, approver policy, requested effect, and immutable
  decision context.
- Approval request creation is audited atomically with the decision.
- Approving does not mutate the original decision; it appends an
  approval event and resumes with a new idempotent operation.

Implementation:

- Add approval tables/migrations.
- Extend contract evaluator output for approval metadata.
- Add ledger/control plane APIs to create and resolve approval
  requests.
- Add audit events for create, approve, deny, expire.

Test and acceptance:

- Contract rule returns `REQUIRE_APPROVAL` and creates pending record.
- Pending approval has immutable decision context.
- TTL expiry changes state exactly once.
- Repeated approve/deny calls are idempotent.

Review standard:

- Verify approval cannot be used to exceed budget without a fresh
  ledger check.
- Verify approver identity is required and auditable.
- Verify approval payload cannot be modified after creation.

### S15 - Approval API, UI, and notification hooks

Goal: give operators a usable path to approve or deny pending agent
actions.

Design:

- Control plane exposes approval list/detail/approve/deny APIs.
- Dashboard shows pending approvals and decision context.
- Notification hooks start with webhook/Slack-compatible payloads.
- External notification failure must not lose the approval request.

Implementation:

- Add REST endpoints under control plane.
- Add dashboard approval view.
- Add notification dispatcher with retry/outbox.
- Add config for webhook URL and signing secret.

Test and acceptance:

- Operator can approve and deny from API.
- Dashboard shows pending approval with tenant and budget context.
- Notification retry handles temporary failure.
- Unauthorized caller cannot resolve approval.

Review standard:

- Verify UI/API never expose cross-tenant approval data.
- Verify approval action requires RBAC once S18 exists.
- Verify notification payload is signed or sent over authenticated
  channel.

### S16 - Adapter resume, deny, and timeout semantics

Goal: make SDK/adapters correctly pause, resume, deny, or timeout the
agent run.

Design:

- Adapter receiving `REQUIRE_APPROVAL` raises a typed exception or
  returns a resumable token depending on framework semantics.
- Resume checks approval state, revalidates budget, and publishes the
  effect only if approved.
- Deny maps to a clear application exception.
- Timeout maps to release/cancel semantics.

Implementation:

- Extend sidecar adapter protocol for approval token/resume.
- Update Python SDK errors and framework integrations.
- Add examples for Pydantic-AI and LangChain.
- Add demo mode `approval`.

Test and acceptance:

- Demo: request exceeds soft cap, approval pending, approve, run
  resumes and commits.
- Demo: deny produces deterministic exception and audit outcome.
- Timeout releases reservation and records outcome.
- Idempotent resume cannot publish effect twice.

Review standard:

- Verify framework-specific adapters do not hide approval failures.
- Verify resume path cannot skip Contract/Ledger recheck.
- Verify idempotency keys include approval/resume identity where needed.

## Track E - SSO/RBAC, tenant isolation, retention

### S17 - OIDC/SSO foundation

Goal: replace single admin bearer token with OIDC-based authentication.

Design:

- Support OIDC JWT validation first.
- Map issuer, subject, groups, and tenant claims into a principal.
- Keep static token only for local demo profile.
- Auth middleware is shared by dashboard and control plane.

Implementation:

- Add auth module for JWT validation and JWKS caching.
- Add config for issuer, audience, JWKS URL, clock skew.
- Update dashboard and control plane to use auth middleware.
- Add docs for Entra ID first, generic OIDC second.

Test and acceptance:

- Valid JWT accepted.
- Wrong issuer/audience rejected.
- Expired JWT rejected.
- Local demo still works with explicit demo profile.

Review standard:

- Verify JWKS cache refresh handles key rotation.
- Verify auth failures do not reveal tenant existence.
- Verify static token cannot be enabled accidentally in production
  chart values.

### S18 - RBAC and tenant isolation

Goal: enforce per-tenant and per-role access across control plane,
dashboard, export, approval, and budget APIs.

Design:

- Roles: `viewer`, `operator`, `approver`, `admin`, `auditor`.
- Resource scope: tenant, budget, contract bundle, approval request,
  audit export.
- Service-to-service auth remains mTLS plus workload identity.
- Every query must include tenant scope derived from auth context, not
  request body alone.

Implementation:

- Add RBAC policy table or config-backed policy for first iteration.
- Add middleware to attach allowed tenant ids and roles.
- Update SQL queries to enforce tenant scope.
- Add negative tests for cross-tenant access.

Test and acceptance:

- Viewer can read dashboard but cannot approve.
- Approver can resolve approval only for assigned tenant.
- Auditor can export audit data but cannot mutate budgets.
- Cross-tenant API attempts return 403 and emit audit/security log.

Review standard:

- Verify tenant id from URL/body is never trusted without auth context.
- Verify every new endpoint has an RBAC test.
- Verify service logs include principal id for mutating actions.

### S19 - Retention, redaction, and tenant data policy

Goal: define and enforce retention for prompt metadata, provider raw
payloads, audit records, exports, and operational logs.

Design:

- Immutable audit records are retained for the configured compliance
  window.
- Sensitive payload fields can be redacted or excluded before export.
- Tenant policy controls prompt retention and raw provider payload
  retention separately.
- Deletion/tombstone never breaks audit chain integrity.

Implementation:

- Add retention policy config/table.
- Add redaction layer for export and dashboard views.
- Add retention sweeper for non-immutable payload classes.
- Add data classification docs for each event field.

Test and acceptance:

- Tenant with prompt retention `0 days` stores only hashes/metadata.
- Retention sweeper removes eligible raw payloads without deleting audit
  chain rows.
- Export respects redaction policy.
- Tombstoned tenant remains auditable.

Review standard:

- Verify retention code cannot delete ledger/audit invariants.
- Verify redaction happens before data leaves service boundary.
- Verify docs identify which fields may contain prompts, PII, or
  provider secrets.

## Track F - Onboarding in half a day

### S20 - One-workflow onboarding templates

Goal: let a design partner connect one real workflow without learning
the whole platform first.

Design:

- Pick one golden path: Python app + Pydantic-AI or LangChain +
  sidecar + existing Postgres.
- Provide templates for contract, budget, pricing env, Helm values, and
  SDK adapter code.
- The first workflow should demonstrate hard cap `STOP`, soft cap
  `REQUIRE_APPROVAL`, and normal `CONTINUE`.

Implementation:

- Add `templates/onboarding/python-langchain` or equivalent.
- Add `spendguard init workflow` command or script if a CLI exists;
  otherwise provide Makefile targets.
- Add minimal contract bundle generator wrapper.
- Add docs with exact commands and expected outputs.

Test and acceptance:

- Fresh developer follows the guide and reaches a passing deny demo
  against their workflow within half a day.
- Template uses no demo UUIDs after initialization.
- Generated config is explicit about fail policy and retention policy.
- The guide includes rollback/removal steps.

Review standard:

- Verify no copy-paste secret values ship in docs.
- Verify generated config works in both local compose and Helm where
  possible.
- Verify onboarding path does not require reading internal specs first.

### S21 - Doctor/readiness verifier

Goal: make environment and integration failures diagnosable before the
customer runs a real agent.

Design:

- A verifier checks sidecar reachability, ledger connectivity, active
  fencing lease, pricing snapshot, contract bundle, signing mode, and
  tenant budget state.
- Output is machine-readable JSON plus human-readable summary.
- Verifier fails with actionable error codes.

Implementation:

- Add CLI or script `spendguard doctor`.
- Add control plane endpoint for readiness summary if needed.
- Add checks for Helm and local compose.
- Add docs mapping doctor failures to remediation.

Test and acceptance:

- Missing bundle produces a clear failure.
- Stale fencing lease produces a clear failure.
- Unknown pricing version produces a clear failure.
- Healthy stack returns all green and can run one dry-run decision.

Review standard:

- Verify doctor does not mutate production state except optional dry-run
  using a clearly marked test tenant.
- Verify secrets are redacted from output.
- Verify every fatal startup precondition has a doctor check.

## Track G - Fail policy, SLOs, observability

### S22 - Fail-open/fail-closed policy matrix

Goal: define what every component does when dependencies fail.

Design:

- Default stance for budget enforcement is fail-closed.
- Explicit exception can be configured per tenant/workflow for low-risk
  non-monetary tool calls.
- Matrix covers sidecar, ledger, canonical ingest, pricing authority,
  signing, provider reconciliation, approval, dashboard, and export.
- Each decision outcome must be auditable or explicitly marked as not
  published.

Implementation:

- Add `fail_policy` config in contract or tenant settings.
- Sidecar enforces policy before publishing effects.
- Add error codes for dependency failures that map to fail policy.
- Update docs and examples.

Test and acceptance:

- Ledger unavailable blocks monetary LLM calls by default.
- Canonical ingest unavailable follows configured durability policy and
  never silently drops audit evidence.
- Pricing stale beyond threshold follows configured policy.
- Low-risk workflow with explicit fail-open produces a clear audit
  marker.

Review standard:

- Verify fail-open requires explicit tenant/workflow config and is
  visible in audit logs.
- Verify no fail-open path can debit budget without later
  reconciliation evidence.
- Verify operator docs state blast radius and rollback behavior.

### S23 - SLOs, alerts, and incident drills

Goal: define production operating standards for a hot-path control
system.

Design:

- SLOs cover decision latency, decision availability, ledger commit
  success, audit forward lag, reconciliation lag, approval latency, and
  pricing freshness.
- Alerts target symptoms, not just process health.
- Incident drills prove failover, stale lease handling, signature
  failure handling, and pricing outage behavior.

Implementation:

- Add metrics where missing:
  - decision latency histogram.
  - ledger transaction error count by code.
  - lease age and leader identity.
  - audit outbox pending age.
  - canonical ingest reject/quarantine count.
  - pricing snapshot age.
  - provider reconciliation lag.
- Add dashboard panels and alert rule examples.
- Add runbooks for each alert.

Test and acceptance:

- Load test demonstrates target decision latency under expected QPS.
- Kill ledger and verify alert plus fail policy behavior.
- Break signing key and verify quarantine/alert.
- Stop pricing sync and verify freshness alert before enforcement
  failure.

Review standard:

- Verify SLOs are stated with numeric targets before GA.
- Verify every page has an owner and runbook.
- Verify alerts do not require reading raw DB tables to triage.

## PR review checklist

Use this checklist for every slice:

- Does the PR preserve "no effect without audit evidence"?
- Does it preserve ledger balance invariants under retry and crash?
- Are proto and DB changes backward compatible or explicitly versioned?
- Are tenant ids always derived from trusted context?
- Are secrets absent from logs, metrics, errors, and docs?
- Are failure modes explicit and tested?
- Does Helm/Terraform make unsafe production config difficult?
- Is there a rollback path that does not require deleting audit data?
- Does the user-facing doc state what is still unsupported?

## Release sequencing recommendation

First production-candidate milestone:

- S1, S2, S3, S4, S6, S8, S17, S18, S22.

This milestone is the minimum bar before telling a customer the runtime
guard can protect real production spend.

Second production-candidate milestone:

- S5, S7, S9, S10, S11, S13, S20, S21, S23.

This milestone makes the product operable by a design partner without
constant engineering support.

Third production-candidate milestone:

- S12, S14, S15, S16, S19.

This milestone expands the product into multi-provider reconciliation,
auditable approvals, and regulated data governance.
