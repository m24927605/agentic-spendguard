# POST_GA 05 - Provider Coverage

> **Branch**: `post-ga/POST_GA_05_provider_coverage`
> **Status**: implemented; Staff+ arbitration accepted; ready to merge
> **Spec ancestor(s)**: `post-ga-backlog-spec-v1alpha1.md`, `tokenizer-service-spec-v1alpha1.md`
> **Issues**: #139
> **Estimated change size**: medium; Tier 1 provider clients and tests

---

## §0. TL;DR

Add Cohere and Llama Tier 1 provider count_tokens support, including
envelope tuning, opt-in safety, quota behavior, and drift evidence. Llama
uses Bedrock Runtime CountTokens to match the locked tokenizer spec's
Bedrock model IDs.

## §1. Architectural Context

Tier 1 shadow tokenization compares provider-reported counts against
Tier 2 local estimates. OpenAI/Anthropic/Gemini coverage shipped earlier;
Cohere and Llama remained deferred because provider-specific envelope
behavior and legal/operational constraints needed narrower treatment.

## §2. Scope

- #139: Cohere and Llama Tier 1 provider clients
- Provider-specific request envelope builders
- Llama Bedrock Runtime CountTokens production backend
- Llama HTTP-compatible backend for local/private gateway tests
- Quota and PII opt-in integration with HARDEN_05 controls
- Circuit breaker behavior per provider
- Drift alert integration and evidence
- Mocked and opt-in real-provider tests

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Llama asset performance tuning | POST_GA_04 |
| Generic provider-key rotation runbook | POST_GA_02 |
| New dashboard panels | Future observability slice unless needed for acceptance |

## §4. File-Level Changes

- Modify tokenizer shadow provider code under `services/tokenizer/src/shadow/**`
- Add Cohere and Llama client modules if absent
- Add provider envelope fixtures and tests
- Update Helm/config docs for provider opt-in environment variables
- Add evidence under `docs/internal/reviews/post-ga/POST_GA_05_provider_coverage/`

## §5. Schema / Proto

No proto changes expected. Config additions must be additive and default
off for providers requiring customer keys or license acceptance.

## §6. Audit-Chain Impact

Drift alerts emitted from new providers must preserve existing
`spendguard.audit.*` routing and canonical ingest envelope requirements.
No new audit columns are expected.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Provider key missing | Provider shadow disabled for that tenant/provider |
| Tenant has no raw-text opt-in | No raw prompt text sent |
| Provider API rate limit | Shadow skipped or breaker opens without hot-path impact |
| Envelope mismatch | Drift sample records provider-specific evidence |
| Provider license not accepted | Llama path remains disabled |
| Provider error body echoes prompt | Error body redacted before it reaches logs |

## §8. Acceptance Gates

- `cargo build && cargo test` for `services/tokenizer`
- Mock provider tests for Cohere and Llama success/failure
- PII opt-in and quota tests include new providers
- Optional real-provider tests run only when required env vars are set
- Helm demo/production templates render with secure defaults
- Evidence under `docs/internal/reviews/post-ga/POST_GA_05_provider_coverage/`
- `make demo-up DEMO_MODE=default` actually runs from a clean demo volume

## §9. Review Checklist

1. Are new provider clients default-off without tenant opt-in?
2. Is raw prompt text blocked unless allowed?
3. Are provider errors isolated from the hot path?
4. Do drift alerts use the canonical envelope?
5. Are license and API-key docs accurate and non-secret?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Provider-specific UI toggles | Control-plane UX outside scope |
| New calibration dashboards | Future observability work |

## §11. Risk / Rollback

Provider integrations risk raw-text egress and cost surprises. Keep
feature flags default-off and preserve quota enforcement. Roll back by
disabling provider config and reverting provider modules.

## §12. Review Execution Notes

Reviewer: codex CLI via `codex review --base main`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must inspect PII, quota, provider-error, and license paths.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Software Architect | Keep provider expansion one slice because issue scope is narrow | #139 only |
| Backend Architect | Provider clients must share existing breaker/quota structure | §7 |
| Security Engineer | Raw prompt egress remains tenant opt-in only | §7 and §9 |
| Database Optimizer | No schema change unless provider state cannot fit current tables | §5 |
| Tokenizer Domain Expert | Envelope fixtures are required before real-provider confidence | §8 |
| Software Architect | Keep Cohere/Llama clients confined to tokenizer shadow worker | `services/tokenizer/src/shadow/provider_clients/`; hot-path grep evidence |
| Backend Architect | Use Bedrock Runtime CountTokens with InvokeModel body shape for Llama instead of Together embeddings because locked tokenizer dispatch and Tier 2 raw-text accounting use Bedrock Llama model IDs | `services/tokenizer/src/shadow/provider_clients/llama.rs` |
| Security Engineer | Redact provider error bodies, SDK service error details, and provider client Debug output so API keys, URL userinfo, and raw prompt echoes cannot reach logs | `cohere.rs`, `llama.rs`, `config.rs` |
| Database Optimizer | Reuse existing sample/quota/PII tables; no migration | §5 |
| Tokenizer Domain Expert | Cohere Tier 1 count is `tokens.len() + 1` for non-empty raw text; native dispatch covers both `YYYYMMDD` and documented `MM-YYYY` dated IDs; Bedrock Cohere model normalization strips any numeric revision accepted by dispatch | `services/tokenizer/src/shadow/provider_clients/cohere.rs`, `crates/spendguard-tokenizer/src/dispatch.rs` |
| Staff+ Panel | Round 5 P2 on Bedrock Llama request-id propagation is valid and must be fixed before merge; no Round 6 review after arbitration | `docs/internal/reviews/post-ga/POST_GA_05_provider_coverage/round-5-codex-review-summary.md`, `services/tokenizer/src/shadow/provider_clients/llama.rs` |
| Backend Architect | Bedrock SDK `CountTokensOutput.request_id()` must populate `ProviderCount.request_id`; absence remains `None` | `services/tokenizer/src/shadow/provider_clients/llama.rs` |

## §14. Merge Checklist

- [x] #139 fixed and tested
- [x] Secure defaults verified
- [x] Provider evidence recorded
- [x] Codex review clean or Staff+ arbitration recorded
- [ ] Memory updated
