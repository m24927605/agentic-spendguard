# D01 — Envoy AI Gateway ExtProc Sidecar — Review Standards

**Companion to:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md)
**Used by:** `superpowers:code-reviewer` skill, R1 through R5 per build plan §1.1
**Round-pass rule (LOCKED, build plan §1.1):** A round passes only when the reviewer's finding list is empty after fixes. Severity (Blocker / Major / Minor) is for triage / changelog only; all findings gate the round.

This checklist is the slice-specific extension to the universal cross-cutting checklist at [`docs/review-standards/predictor-review-checklist.md`](../../../review-standards/predictor-review-checklist.md) §1. Reviewer runs the universal checklist plus the per-slice §2-§8 below.

---

## §1. Universal checks (re-applied per slice)

Reviewer MUST apply the universal §1 checklist from [`docs/review-standards/predictor-review-checklist.md`](../../../review-standards/predictor-review-checklist.md). Specifically these clauses bind on every D01 slice:

- §1.1 audit-chain coverage (D01 emits via existing sidecar — see §8 below for the "no direct writes" rule)
- §1.2 tokenizer tier discipline (Tier 2 hot path < 1ms; no Tier 1 from ExtProc hot path)
- §1.3 Strategy A as reservation under STRICT_CEILING (v1 ships A-only per design §5)
- §1.6 Contract DSL strictly additive (D01 makes NO proto changes — verify by `git diff proto/`)
- §1.7 L0-L3 capability semantics unchanged (D01 inherits L2 from sidecar)
- §1.8 failure isolation (sidecar unreachable → ExtProc fall-closed 503)
- §1.9 multi-tenant isolation (SVID per-tenant pinning carries over from HARDEN_08)
- §1.10 observability (handler counters + latency histograms + structured logs)
- §1.11 backwards compatibility (existing demos still pass — gates 14-17 of `acceptance.md`)
- §1.12 SLO budgets (Contract §14 50ms p99 enforced via `bench_full_extproc_roundtrip_p99`)

---

## §2. SLICE 1 — Skeleton + provider-routing extraction

### §2.1 Blocker-class checks (SLICE 1-5 transport carve-out)

Per design §3.3 carve-out: SLICES 1-5 use **UDS** at `/var/run/spendguard/adapter.sock` for local-dev / docker-compose ergonomic; mTLS-over-TCP is a SLICE 6 hard-switch alongside Helm. The mTLS-TCP and SPIFFE checks below are **deferred to §7.1 (SLICE 6)**.

- [ ] `crates/spendguard-provider-routing` extracted cleanly: no dead code left in `services/egress_proxy/src/routing.rs`; only re-exports remain.
- [ ] `services/egress_proxy` regression: `cargo test -p spendguard-egress-proxy` passes byte-identically.
- [ ] All `routes_*` tests in [`tests.md`](tests.md) §1.1 moved with the code; nothing left orphaned in egress_proxy.
- [ ] `Cargo.toml` workspace exclude list updated for both new crates; `cargo build --workspace` still compiles.
- [ ] Transport: UDS at `/var/run/spendguard/adapter.sock` is acceptable here. The SLICE 1-5 deployment shape is same-pod / same-node; cross-pod mTLS is SLICE 6's responsibility.
- [ ] `tonic::net::UnixListener` / `UnixStream` is permitted in SLICE 1-5 code paths; the production `mTLS-TCP` hard-switch is gated to SLICE 6 (see §7.1 below).

### §2.2 Major-class checks (SLICE 1-5 transport carve-out)

- [ ] `Config::from_env` returns typed error (not `unwrap`) for missing required vars.
- [ ] Handshake failure on startup exits process with non-zero status. (Note: `/readyz` HTTP probe wiring is deferred to **§7.1 SLICE 6** alongside the mTLS-TCP hard-switch — co-deferred per design §3.3 carve-out, because the readiness signal is only consumed by Kubernetes, which is also where SLICE 6 lands.)
- [ ] No `unwrap()` / `expect()` on Result types in `main.rs` or `server.rs` request paths.

### §2.3 Adversarial questions

- What happens if `spendguard-provider-routing` and the egress_proxy in-tree consumer disagree on `ROUTING_TABLE` initialization order? Show the test that proves both crates see the same table at first access.
- The Bedrock routing regex `r"^/model/([^/]+)/invoke$"` is preserved. Does the ExtProc service correctly forward Bedrock model id from the URL into ClaimEstimate.model? Show the test fixture.
- If the sidecar's `HandshakeResponse.capability_required` is L4 (research), does the ExtProc service refuse to load? Show the assertion.

---

## §3. SLICE 2 — Token counter wire

### §3.1 Blocker-class checks

- [ ] Tier 2 hot path: tokenizer invoked in-process (library form), NOT via Tier 1 RPC. Verify by `grep -rn "tokenizer_t1\|count_tokens_remote" services/envoy_extproc/src/` returns 0 matches.
- [ ] `bench_token_counting_openai_p99` and `bench_token_counting_anthropic_p99` both meet < 1ms p99.
- [ ] Unknown model → `tokenizer_kind = None`, `input_tokens = 0`, `tokenizer_tier = "T3"`, and the `tokenizer_unknown_model` metric is emitted.
- [ ] No silent fallback that fakes Strategy B/C values. ClaimEstimate B/C MUST be 0 in this slice.

### §3.2 Major-class checks

- [ ] `parse_malformed_json_returns_error` exists and the caller maps it to ExtProc immediate_response 400.
- [ ] Bedrock model id resolution from URL path matches `routing.rs::resolve_model_id` byte-for-byte.

### §3.3 Adversarial questions

- A 4MiB JSON body — does Tier 2 still meet the 1ms p99 budget? Show the bench result with body size matching the spec §11 4MiB cap from POST_GA_03.
- Anthropic body with `cache_creation_input_tokens` only — does Tier 2 count cache tokens consistently with `services/egress_proxy`?

---

## §4. SLICE 3 — Budget query path

### §4.1 Blocker-class checks

- [ ] `RequestDecision` includes a non-empty `idempotency.key`; show the derivation (W3C trace context + ExtProc request id).
- [ ] Sidecar timeout enforced: tonic call gated by `SPENDGUARD_EXTPROC_REQUEST_TIMEOUT_MS`; on timeout, response is immediate_response 503 with Retry-After.
- [ ] No leak of sidecar internal error details into ExtProc response body (security: avoid info disclosure).
- [ ] `Decision::Stop` and `Decision::StopRunProjection` BOTH map to HTTP 429; `run_code_triggered` surfaced in response body for dashboard categorization.
- [ ] `Decision::RequireApproval` returns 403 with `approval_request_id` header so clients can poll for resolution.
- [ ] `Decision::Degrade` mutation patch is applied to the upstream request body via ExtProc `BodyMutation` — the patch is NOT silently dropped.
- [ ] Stream state map (`StreamState`) bounded; old entries expire after 60s to prevent OOM under chaos.

### §4.2 Major-class checks

- [ ] `Decision::Unspecified` (proto3 default) maps to 503 fail-closed, NOT 200 continue. Defense in depth.
- [ ] Per-handler `envoy_extproc_handler_total{handler,outcome}` counter incremented in every code path.

### §4.3 Adversarial questions

- Two concurrent ExtProc streams for the same `session_id` — does `bind_decision` race? Show the test.
- A `RequestDecision` returns `DEGRADE` with a mutation patch that adds a 100 KB system prompt. Is the patch applied within the 50ms hot-path budget? Show the bench.
- Sidecar replies CONTINUE but then closes the gRPC stream — does ExtProc keep serving the inbound stream or emit a typed error?

---

## §5. SLICE 4 — Audit emit (Response phase)

### §5.1 Blocker-class checks

- [ ] Exactly one `LLM_CALL_POST` event emitted per ExtProc stream (no double-commit on retry).
- [ ] `provider_reported_amount_atomic` and `unit` populated using the same `usage_extractor` function as egress_proxy — no re-implementation.
- [ ] Upstream 5xx OR stream drop → `LLM_CALL_POST.RUN_ABORTED` emitted (sidecar drives implicit release).
- [ ] Reservation_id propagated from `RequestDecision.reservation_ids[0]` into `LLM_CALL_POST.reservation_id` — no UUID generation in ExtProc.
- [ ] `audit_decision` row's `runtime_kind` MUST be `'envoy-ai-gateway'` (acceptance gate 36).
- [ ] No direct write to `audit_outbox` / `canonical_events` from ExtProc code; all audit flows through sidecar adapter RPC (acceptance gate 38).

### §5.2 Major-class checks

- [ ] Streaming SSE bodies (Server-Sent Events) explicitly out of scope — Response-Body phase commits once at end-of-stream. Re-read design §3.5.
- [ ] When upstream returns 200 but no `usage` block in response body, fall back to the input × 2 estimate per HARDEN_03 pattern; do NOT silently emit a 0-token commit.

### §5.3 Adversarial questions

- Anthropic response with `cache_creation_input_tokens > 0` and `cache_read_input_tokens > 0` — both surfaced in `LlmCallPostPayload.provider_reported`? Show the test and the sidecar-side mirror.
- Bedrock response with the streaming InvokeModelWithResponseStream shape — does the spec correctly defer this to v1.1 (per design §5)?
- A client cancels mid-stream after Request-Body but before Response-Body — within how many ms does ExtProc emit RUN_ABORTED? Show the test asserting < 100ms.

---

## §6. SLICE 5 — Conformance

### §6.1 Blocker-class checks

- [ ] Golden fixtures committed in-tree (no live network call required at test time).
- [ ] Both `token_counting.yaml` and `budget.yaml` reference fixtures byte-equal the produced response stream.
- [ ] Conformance harness produces deterministic output (no timestamps / UUIDs leaking into the golden file).
- [ ] Benchmark targets in `acceptance.md` §5 all met (gates 25, 26, 27).

### §6.2 Major-class checks

- [ ] Conformance fixture source / version pinned in the test docstring so future reviewers can re-derive.
- [ ] If Envoy AI Gateway v0.6 ships a backward-incompatible v0.7, the conformance harness should fail loudly, NOT silently regenerate.

### §6.3 Adversarial questions

- The Envoy AI Gateway reference manifest's `token_counting.yaml` budgets are different from SpendGuard defaults — how does the conformance test pin contract bundle state so the response stream is deterministic?
- Does the conformance test exercise the Bedrock + Vertex + Azure paths or just OpenAI + Anthropic? If not, gap it.

---

## §7. SLICE 6 — Helm sub-chart

### §7.1 Blocker-class checks (SLICE 6 production transport hard-switch)

Per design §3.3 carve-out, SLICE 6 is the transport hard-switch from UDS (SLICE 1-5) to mTLS-over-TCP. The SLICE 1-5 UDS code paths MUST be removed or guarded off here; the readiness probe lands alongside.

- [ ] `envoyExtproc.enabled` defaults to `false`; opt-in only (acceptance gate 21).
- [ ] No fail-open: `tenant_id` required at render time (matches GA_03 fail-closed posture).
- [ ] SVID volume mount uses `csi.spiffe.io` driver — matches `output_predictor_plugin_svid.yaml` (acceptance gate 23).
- [ ] NetworkPolicy ingress restricted to `app.kubernetes.io/name: envoy-ai-gateway` pods (acceptance gate 22).
- [ ] Container runs `runAsNonRoot: true`, `readOnlyRootFilesystem: true`, `capabilities.drop: ["ALL"]`.
- [ ] Image is non-root verified at the OCI layer (Trivy gate 29).
- [ ] Helm kind-cluster install passes (acceptance gate 24).
- [ ] **Transport hard-switch: mTLS-over-TCP configured. NO `tokio::net::UnixListener` or `SocketAddr::Unix` in production execution paths of `services/envoy_extproc/src/`. Re-read design §3.3.** (UDS-using test fixtures or `#[cfg(test)]`-gated helpers do NOT count.)
- [ ] **SVID cert pinning enforced on the sidecar-client side; SPIFFE URI SAN matches `spiffe://<tenant>/sidecar` regex.**
- [ ] **`/readyz` HTTP probe returns 503 until the mTLS sidecar handshake succeeds, then 200. Kubernetes readinessProbe in the Helm sub-chart points at `/readyz`.**

### §7.2 Major-class checks

- [ ] `image.tag` defaults to the chart `appVersion`; no hard-coded `:latest`.
- [ ] `replicaCount` defaults to 2 for HA.
- [ ] PodDisruptionBudget present (matches the existing sidecar chart shape).

### §7.3 Adversarial questions

- What happens if `envoyExtproc.enabled=true` but `tenant_id` is unset? Show the `helm template` error.
- Does the chart still render correctly under `helm template --strict`?
- Are there any `hostPath` mounts or privileged containers? (Should be zero — verify per GA_03 invariants.)

---

## §8. SLICE 7 — Demo + docs

### §8.1 Blocker-class checks

- [ ] `make demo-up DEMO_MODE=envoy_extproc` succeeds end-to-end within 180s.
- [ ] `verify_step_envoy_extproc.sql` asserts ≥ 5 paired `audit_decision` + `audit_outcome` rows with `runtime_kind = 'envoy-ai-gateway'`.
- [ ] `verify-chain` regression at end of demo run reports 0 failures (acceptance gate 13).
- [ ] All four existing demo modes still pass (regression — acceptance gates 14-17).
- [ ] `README.md` updated with the new adapter row.
- [ ] `docs/site/docs/integrations/envoy-ai-gateway.md` shows the Envoy `ExternalProcessor` config snippet pointing at the new sidecar service.

### §8.2 Major-class checks

- [ ] Demo cleanup (`make demo-down`) leaves no residual containers or volumes.
- [ ] Demo includes at least one `STOP` outcome path so the 429 mapping is exercised end-to-end.
- [ ] Demo logs contain no ERROR-level lines on the happy path.

### §8.3 Adversarial questions

- Does the demo's Envoy config use a stable, pinned Envoy image tag? Or `:latest` (rejection)?
- Does the demo cover the streaming SSE deferral — i.e., does it explicitly use non-streaming bodies so the v1 scope is honoured?

---

## §9. Cross-slice consistency checks

Every R1+ review must spot-check these end-to-end invariants:

1. **No proto changes** — `git diff main...HEAD -- proto/` is empty. (D01 reuses existing sidecar adapter proto verbatim.)
2. **No ledger changes** — `git diff main...HEAD -- services/ledger/` is empty.
3. **No audit chain schema changes** — `git diff main...HEAD -- services/canonical_ingest/` is empty.
4. **`spendguard-provider-routing` is the only new shared crate** — no other extractions.
5. **Sidecar adapter is the only audit write path** — `grep -rn "INSERT INTO audit_outbox\|INSERT INTO canonical_events" services/envoy_extproc/ crates/spendguard-provider-routing/` returns 0.
6. **Transport carve-out (design §3.3):**
   - **SLICE 1-5**: UDS at `/var/run/spendguard/adapter.sock` is permitted. The grep gate is NOT applied here.
   - **SLICE 6+**: production execution paths in `services/envoy_extproc/src/` MUST be UDS-free. `grep -rn "UnixListener\|UnixStream\|/var/run/spendguard/adapter.sock" services/envoy_extproc/src/ --exclude-dir=tests` returns 0 (or hits only `#[cfg(test)]`-gated code, which reviewers spot-check by hand).

---

## §10. Severity definitions (triage only, not pass gate)

Per [`predictor-review-checklist.md`](../../../review-standards/predictor-review-checklist.md) §3:

- **Blocker** — audit chain regression; mTLS / SVID bypass; fail-open by default; proto change; ledger change; benchmark p99 budget miss; any test failure.
- **Major** — observability gap; missing error path; spec-implementation drift; sidecar coupling violation (e.g. direct ledger access from ExtProc).
- **Minor** — naming inconsistency; doc polish; comment quality.

**Reminder:** all findings of all severities must be fixed before the round counts as passing.

---

## §11. Round-pass rule (locked)

Round pass = the round's reviewer finding list is empty after fixes. R5 still fails → Staff+ panel arbitration per [`staff-panel-arbitration-process.md`](../../../review-standards/staff-panel-arbitration-process.md).

---

*Review-standards version: D01 v1alpha1 | Companion: universal predictor-review-checklist v1alpha1 + staff-panel-arbitration-process.md*
