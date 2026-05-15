# Session Prompt — Egress Proxy Slices 4-7

Self-contained prompt for a fresh Claude Code session to continue
the auto-instrument egress proxy implementation.

**Context**: Slices 1-3 shipped on branch `feat/auto-instrument-egress-proxy-impl`
(merged to main as commits `b27ad84` / `27f640a` / `aa34f89` / `265580f`).
Spec is LOCKED at v7 with 11 slices defined. Slices 4-7 + 8 remain.

---

## Prompt to paste into a fresh session

```
任務上下文
=========
繼續 Agentic SpendGuard 的 auto-instrument egress proxy 實作。Slices
1-3 已在 main:
  - Slice 1: docs/specs/auto-instrument-egress-proxy-spec.md v7 LOCKED
            (+ docs/specs/auto-instrument-egress-proxy-staff-escalation-r5.md)
  - Slice 2: services/ids/ shared crate + services/egress_proxy/ skeleton + /healthz
  - Slice 3: POST /v1/chat/completions HTTP pass-through forwarder

剩下 4 個 implementation slices (per spec §15) + 1 final slice:
  - Slice 4 (a/b/c): sidecar UDS gRPC client + DecisionRequest + fail-closed routing
  - Slice 5: Commit lane (LLM_CALL_POST + ConfirmPublishOutcome + error release)
  - Slice 6: Identification (env-default + per-request header override)
  - Slice 7 (a/b): DEMO_MODE=proxy + e2e smoke
  - Slice 8: Final sweep + memory + README

工作目錄：/Users/michael.chen/products/agentic-spendguard
GitHub：https://github.com/m24927605/agentic-spendguard
Branch (continue or new): feat/auto-instrument-egress-proxy-impl

關鍵戰略決定（不要再質疑）
========================
1. spec v7 LOCKED — 不接受結構性 redesign，只接受 minor patches
2. cross-language byte-equivalence via services/ids/ — 已 ship + 已驗;
   後續 slice 不要再質疑 blake2b/UUIDv4-shape/unified `:call:` discriminator
3. Per memory feedback_codex_iteration_pattern.md: per slice ≤5 codex
   rounds, Staff escalation per §14.1 if RED at r5
4. 每 slice 必須 demo PASS (per feedback_demo_quality_gate.md); slice 7
   是 demo-quality-gate slice

關鍵檔案（讀這些建立 context）
=============================
- docs/specs/auto-instrument-egress-proxy-spec.md — LOCKED v7, 1038 lines
- docs/specs/auto-instrument-egress-proxy-staff-escalation-r5.md — r5 audit trail
- services/ids/ — blake2b helpers + cross-lang fixture
- services/egress_proxy/src/main.rs — axum entrypoint with healthz + /v1/chat/completions
- services/egress_proxy/src/forward.rs — slice 3 HTTP forwarder (no gating)
- services/egress_proxy/src/redacted_auth.rs — slice 2 RedactedAuth newtype
- services/sidecar/src/server/adapter_uds.rs — sidecar UDS server (slice 4 client is its mirror)
- sdk/python/src/spendguard/integrations/pydantic_ai.py:615-634 — reference for LLM_CALL_POST → ConfirmPublishOutcome order
- sdk/python/src/spendguard/integrations/pydantic_ai.py:535-541 — reference DecisionRequest construction
- proto/spendguard/sidecar_adapter/v1/adapter.proto — wire contract
- services/sidecar/src/decision/transaction.rs:881-891 — pricing validation; commit_estimated lane reference

啟動程序
========
1. cd /Users/michael.chen/products/agentic-spendguard
2. git checkout feat/auto-instrument-egress-proxy-impl (or rebase off main)
3. cat docs/specs/auto-instrument-egress-proxy-spec.md §15 行 slice 表
4. cargo check -p spendguard-egress-proxy (sanity)
5. 開始 Slice 4a

實作流程（嚴格按 spec）
====================

### Slice 4a — sidecar UDS gRPC client + handshake + /readyz

新檔 services/egress_proxy/src/sidecar_client.rs:
- UDS client to /var/run/spendguard/adapter.sock (or env override)
- connect() + handshake() with 1s backoff retry up to 30s total
- /readyz route reflects handshake state
- 整 proto stubs: tonic-build OR 直接借用 services/sidecar 的 proto (path dep)

Reference: services/sidecar/src/server/adapter_uds.rs handshake handler

acceptance: 啟 sidecar → 啟 proxy → /readyz returns 200 (was 503 pre-handshake)

### Slice 4b — DecisionRequest construction + ReservationContext

新檔 services/egress_proxy/src/decision.rs:
- 從 forward.rs::chat_completions 抽出 body parse + signature derivation
- DecisionRequest:
  - tenant_id, route="llm.call", trigger=LLM_CALL_PRE
  - ids: run_id (header or fresh), step_id = f"{run_id}:call:{sig}" (per spec §4.1),
    llm_call_id = derive_uuid_from_signature(sig, "llm_call_id") (per services/ids/),
    decision_id = fresh UUIDv7
  - idempotency.key = per-attempt sha256(body || nanos)[..16] (per spec §3.2)
  - projected_claims = [BudgetClaim{...}]
  - projected_unit = UnitRef{token_kind="output_token", model_family=parsed}
- ReservationContext struct (per spec §4.1.5) FROZEN-at-PRE pricing via arc_swap +
  runtime.env mtime check
- Pricing tuple read from /var/lib/spendguard/bundles/runtime.env at PRE time

Acceptance: integration test — overwrites runtime.env mid-request; assert POST-time
payload carries OLD pricing hash (per spec §15 row 4c P2-r3.G test)

### Slice 4c — Fail-closed decision routing

forward.rs::chat_completions 改:
- 在 forward to OpenAI 之前 call sidecar_client.request_decision(req)
- 嚴格 match (per spec §4.2 table):
  - Continue → forward
  - Stop → 429 + Retry-After: 86400 (hard-cap) + structured body
  - RequireApproval | Degrade → 503
  - Skip → 429 with skip body
  - Unknown variant → 502
  - tonic Status::Cancelled / DeadlineExceeded / other → 502 + spendguard_sidecar_unavailable
- Test matrix: spawn sidecar mock returning each variant; assert no OpenAI call when
  decision != Continue (in-test counter MUST be 0)

### Slice 5 — Commit lane

forward.rs::chat_completions 在 forward success 之後:
- Parse response usage.total_tokens
- 12a: sidecar_client.emit_llm_call_post(LlmCallPostPayload{outcome: SUCCESS,
  estimated_amount_atomic: total_tokens.to_string(), unit, pricing from ReservationContext})
- 12b: sidecar_client.confirm_publish_outcome(decision_id, effect_hash, APPLIED)
- Order matters — verify against pydantic_ai.py:615-634

Error paths (per spec §4.4 single-RPC rule):
- OpenAI 4xx/5xx/timeout: emit_llm_call_post(PROVIDER_ERROR or CLIENT_TIMEOUT)
- Proxy-internal: confirm_publish_outcome(APPLY_FAILED)
- NEVER both (double-release guard at transaction.rs:1078-1083 would reject 2nd call)

SIGTERM: axum graceful_shutdown drains each handler natural; NO global enumeration
(per spec §9 + r2 P2-r2.D fix).

acceptance: integration test fault-injection — kill sidecar mid-flight, assert
release path fires; verify both audit_outbox rows land for happy path.

### Slice 6 — Identification

decision.rs 加 (priority order):
1. X-SpendGuard-Tenant-Id / Budget-Id / Window-Instance-Id header → 用 header
2. fallback to SPENDGUARD_PROXY_DEFAULT_TENANT_ID / _BUDGET_ID / _WINDOW_INSTANCE_ID env
3. neither → 400 spendguard_missing_identification

UUIDv4 validation at startup (env vars) — fail fast on bad UUID.

This unblocks the "1-env-var" launch claim:
```
SPENDGUARD_PROXY_DEFAULT_TENANT_ID=00000000-0000-4000-8000-000000000001
SPENDGUARD_PROXY_DEFAULT_BUDGET_ID=44444444-4444-4444-8444-444444444444
SPENDGUARD_PROXY_DEFAULT_WINDOW_INSTANCE_ID=55555555-5555-4555-8555-555555555555
```
+ user's openai-python:
```python
OpenAI(base_url="http://localhost:9000/v1", api_key="sk-...")
```

NO header injection in user code. 

### Slice 7a — DEMO_MODE=proxy compose entry

deploy/demo/Makefile 加 DEMO_MODE=proxy case:
- 起 postgres + pki-init + bundles-init + ledger + canonical-ingest + sidecar + egress-proxy
- Makefile 設 SPENDGUARD_PROXY_DEFAULT_* env to demo seed IDs

deploy/demo/compose.yaml egress-proxy:
- depends_on: sidecar (service_healthy)
- volume: sidecar-uds:/var/run/spendguard:ro (sidecar UDS)
- volume: bundles-data:/var/lib/spendguard/bundles:ro (pricing freeze source)
- env: SPENDGUARD_PROXY_SIDECAR_UDS_PATH + SPENDGUARD_PROXY_DEFAULT_*

### Slice 7b — e2e smoke driver

deploy/demo/proxy_demo.sh:
- Python container: `pip install openai`; OpenAI(base_url="http://egress-proxy:9000/v1", ...)
- CONTINUE path: small claim → 200 + real LLM response
- STOP path: huge claim (budget set to small) → 429 with spendguard_blocked body
- 5xx: kill sidecar → 502 with spendguard_sidecar_unavailable
- Assert in ledger: reservation + commit_estimated audit chain visible
- Assert in logs: zero `Bearer sk-` or decoy-token substring

Acceptance: PASS on fresh-volume DEMO_MODE=proxy make demo-up.

### Slice 8 — Final sweep + spec lock + memory + README + capability advertising

- Codex final adversarial review on full slice 4+5+6+7 surface
- spec FROZEN; memory project_overview.md gains "egress-proxy v0.1" section
- README repositioned: lead with "1 env var: OPENAI_BASE_URL=http://localhost:9000/v1"
- services/sidecar/src/config.rs:43 doc-comment adds `egress_proxy_opt_in` to
  accepted strings (no validation change per Staff #3)
- Final demo PASS

不要做
======
- 任何 spec v7 結構性改動 (Staff lock 過)
- 不要嘗試 streaming SSE pass-through (v0.2)
- 不要嘗試 multi-provider routing (v0.2)
- 不要嘗試 NetworkPolicy / Helm templates (v0.2)

時間/Effort
==========
不要用「需要多少天」做排序論證 (per memory feedback_working_principles.md)。每個 slice 完成就 commit + 進下個。

完成通知
========
全 4 slices (4a+4b+4c+5+6+7a+7b+8) 都 ship + demo PASS 後通知用戶。
中間不要停下來請示，per memory feedback_dont_stop_per_slice.md。
```
