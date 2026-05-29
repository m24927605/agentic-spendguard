# Slice 03 — Tokenizer service skeleton (Tier 2 OpenAI only)

> **Branch**: `slice/SLICE_03_tokenizer_service_skeleton_openai`
> **Status**: draft
> **Spec ancestor(s)**: `tokenizer-service-spec-v1alpha1.md` (primary), `audit-chain-prediction-extension-v1alpha1.md` (audit columns)
> **Depends on prior slices**: SLICE_01 (tokenizer_versions table + audit columns)
> **Blocks subsequent slices**: SLICE_04 (Anthropic/Gemini expansion), SLICE_05 (Tier 1 shadow), SLICE_10 (egress_proxy decision rewrite)
> **Estimated PR size**: medium-large (new service crate + library crate + dispatch table + asset bundling; ~1500-2000 LOC)

---

## §0. TL;DR

New `services/tokenizer/` crate + new `spendguard-tokenizer` library crate. Tier 2 OpenAI-only implementation (tiktoken-rs encoders cl100k_base / o200k_base / p50k_base). Tier 3 fallback for unknown models. Helm template + Docker compose entry. Populates tokenizer_versions registry with initial OpenAI entries.

---

## §1. Architectural context

per `tokenizer-service-spec-v1alpha1.md` §2.1 (two co-existing deployment forms — gRPC service + library crate); §3 (Tier 2 hot-path). Serves Q2 (Tier 2 as source of truth).

---

## §2. Scope (must-do)

- Create `services/tokenizer/` crate with `Cargo.toml`, `src/main.rs`, `src/lib.rs`, `src/server.rs`
- Create `spendguard-tokenizer` library crate
- Define `proto/spendguard/tokenizer/v1/tokenizer.proto` with `Tokenize` + `ShadowVerify` (Tier 1 stubbed)
- Implement Tier 2 OpenAI path using `tiktoken-rs` dependency
- Vendor encoders: cl100k_base, o200k_base, p50k_base via embedded assets
- Dispatch table per `tokenizer-service-spec-v1alpha1.md` §3.1 (OpenAI subset only)
- Tier 3 fallback per §5 (5% conservative margin formula)
- Helm chart entry under `charts/spendguard/templates/tokenizer.yaml` (production-profile fail-gate per `sidecar.yaml:8-23` pattern)
- Local Docker compose entry under `deploy/demo/compose.yaml` for `make demo-up` tests
- Initial population of `tokenizer_versions` rows (3 OpenAI encoders + 1 HEURISTIC marker)
- Signed bundle integrity check at boot per §7.4

---

## §3. Out of scope

| 項目 | 推給 |
|---|---|
| Anthropic / Gemini BPE | SLICE_04 |
| Tier 1 shadow worker | SLICE_05 |
| Provider HTTP clients for Tier 1 | SLICE_05 |
| Egress proxy integration | SLICE_10 |
| Calibration-report integration | SLICE_13 |

---

## §4. File-level change list

### 4.1 New files

- `services/tokenizer/Cargo.toml`, `src/main.rs`, `src/lib.rs`, `src/server.rs`, `src/dispatch.rs`
- `crates/spendguard-tokenizer/Cargo.toml`, `src/lib.rs`, `src/dispatch.rs`, `src/encoder_cache.rs`, `src/tier3.rs`, `data/` (asset directory)
- `proto/spendguard/tokenizer/v1/tokenizer.proto`
- `charts/spendguard/templates/tokenizer.yaml`
- `charts/spendguard/values-production-profile.yaml` snippet
- `benchmarks/tokenizer/` directory + bench harness

### 4.2 Modified files

- `deploy/demo/compose.yaml` — add tokenizer service entry
- `services/sidecar/Cargo.toml` — add `spendguard-tokenizer = { path = "../crates/spendguard-tokenizer" }` dependency
- `services/sidecar/src/decision.rs` — placeholder integration (full integration in SLICE_10)

### 4.3 Helm / config

- Production profile fail-gate: `tokenizer.tier3_alert_threshold: 0.001` (0.1% per spec health invariant)

---

## §5. Schema / proto changes

per `tokenizer-service-spec-v1alpha1.md` §2.2 (proto definition). `tokenizer_versions` rows inserted via migration script.

---

## §6. Audit-chain impact

- Library `tokenize()` writes `tokenizer_tier` + `tokenizer_version_id` to caller's `BudgetClaim` metadata
- Initial integration only Tier 2 (T2) and Tier 3 (T3); Tier 1 (T1) flag not yet emitted
- SLICE_01's `tokenizer_versions` registry table populated

---

## §7. Failure mode coverage

| 依賴 | 失敗情境 | 預期行為 |
|---|---|---|
| Asset signature invalid | tampered embedded encoders | refuse-to-start (fail-fast at boot) |
| tiktoken-rs encoder panic | unexpected input | hot path raises error → sidecar fail-closed |
| Unknown model in dispatch | fine-tune / off-list | Tier 3 fallback + emit `tokenizer_unknown_model` metric |
| Library / service version skew | binary mismatch | boot-time check refuse-to-start |
| gRPC service available but library missing | sidecar config error | fail-closed |

---

## §8. Acceptance criteria

### 8.1 Unit tests

- Each dispatch table entry maps to correct encoder + version_id
- Tier 2 OpenAI tokenize matches tiktoken Python reference output for 50 golden samples
- Tier 3 fallback formula correct for unknown model
- Asset signature check fails on tampered bytes

### 8.2 Integration tests

- Library form: tokenize from sidecar process under 1ms p99
- gRPC form: tokenize call under 3ms p99 (over UDS)
- `tokenizer_versions` registry returns correct rows after Helm deploy

### 8.3 Benchmarks

- p99 < 1ms (library form) per `tokenizer-service-spec-v1alpha1.md` §10.1; commit to `benchmarks/tokenizer/` repo + CI run

### 8.4 Audit invariant tests

- Library calls write correct `tokenizer_tier` + `tokenizer_version_id`; `verify-chain` regression green

### 8.5 Demo-mode regression

`make demo-up DEMO_MODE=proxy` shows tokenizer in service map; tokenize call latency reported in audit row metadata

---

## §9. Slice-specific adversarial review checklist

1. How does dispatch handle `gpt-4o-2024-08-06`? `gpt-4o-mini-2024-07-18`? Test fixtures explicit?
2. tiktoken-rs version pin: what version + lockfile entry?
3. Asset sha256 verification: bypassable if env var override allowed? (No env override should exist.)
4. Tier 3 margin: 1.05 vs 2.0 — why 1.05? Reviewer should accept §5.1 reasoning.
5. Library vs service form: what's the canonical entry point for sidecar? Show import path.
6. Multi-tenant: tokenizer service uses tenant from request only for telemetry; no tenant-bound state. Confirmed?
7. Encoder cache RAM usage: < 50 MB total? Measure baseline.
8. Helm production-profile fail-gate: which values are required-input gates?
9. Concurrent tokenize: thread safety of Arc<EncoderRef>? Tested under load?
10. Schema_bundle_id rotation triggered when tokenizer service is restarted?

---

## §10. Out-of-scope deferrals

| 項目 | 推給 |
|---|---|
| Tier 1 shadow worker | SLICE_05 |
| Hot-reload mechanism | SLICE-extra post-launch |
| Vendored Anthropic / Gemini BPE | SLICE_04 |

---

## §11. Risk / rollback plan

- Risk: incorrect encoder dispatch → systematic under/over estimate
- Mitigation: 50 golden sample tests; tiktoken parity
- Rollback: disable tokenizer service in Helm; sidecar falls back to legacy `estimate_tokens` heuristic (deferred SLICE_10 integration means heuristic still in proxy)

---

## §12. AIT execution notes

- Recommended `--agent Backend Architect`
- `--review-budget deep`
- Expected rounds: 3-4 (new crate + asset bundling has many touchpoints)

---

## §13. Adoption history (filled during review)

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder) |

---

## §14. Merge checklist

- [ ] §8 all green; §10.1 p99 benchmark < 1ms verified
- [ ] §9 slice-specific clear
- [ ] universal §1.1 + §1.2 + §1.8 green
- [ ] Tokenizer_versions registry properly populated
- [ ] PR references `tokenizer-service-spec-v1alpha1.md`

---

*Slice version: SLICE_03_tokenizer_service_skeleton_openai v1alpha1 (draft) | Spec ancestor: tokenizer-service-spec-v1alpha1.md §3 §6 §8 | Depends: SLICE_01 | Branch: `slice/SLICE_03_tokenizer_service_skeleton_openai`*
