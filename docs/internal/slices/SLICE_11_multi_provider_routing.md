# Slice 11 — Multi-provider routing in egress_proxy

> **Branch**: `slice/SLICE_11_multi_provider_routing`
> **Status**: draft
> **Spec ancestor(s)**: `tokenizer-service-spec-v1alpha1.md` §3.1 (Bedrock routing patterns); existing `egress-proxy-v0.3-responses-api.md`; new `egress-proxy-multi-provider-spec.md` to be drafted post-this-slice
> **Depends on prior slices**: SLICE_04 (Anthropic / Gemini / Cohere / SentencePiece Tier 2)
> **Blocks subsequent slices**: SLICE_15 (E2E benchmark across providers)
> **Estimated PR size**: large (provider routing + per-provider usage extractors + NetworkPolicy; ~2000 LOC)

---

## §0. TL;DR

Replace `services/egress_proxy/src/forward.rs:36-37` two hard-coded constants with a provider routing table. Add `ProviderConfig` struct + implementations for OpenAI / Anthropic Messages / Bedrock InvokeModel / Vertex generateContent / Azure OpenAI. Per-provider response-usage extractors. NetworkPolicy K8s template for L2 bypass-resistance.

---

## §1. Architectural context

per `tokenizer-service-spec-v1alpha1.md` §3.1 (per-provider dispatch); README's L2 (egress_proxy_hard_block) capability promise. Cross-cutting on Q2 (tokenizer per provider) + L2 capability semantics preserved.

---

## §2. Scope (must-do)

- Replace forward.rs:36-37 hard-coded constants with routing table
- New struct `ProviderConfig { inbound_path, upstream_url_template, request_shape, tokenizer_kind, usage_extractor }`
- Implementations:
  - OpenAI: `/v1/chat/completions` + `/v1/responses`
  - Anthropic: Messages API
  - Bedrock: InvokeModel (per-model dispatch: anthropic.claude-* / cohere.* / meta.llama*)
  - Vertex: generateContent
  - Azure OpenAI
- Per-provider response-usage extractors:
  - OpenAI: `usage.{prompt_tokens, completion_tokens, total_tokens}`
  - Anthropic: `usage.{input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens}`
  - Bedrock: per-model varying (anthropic shape vs cohere shape)
  - Vertex: `usageMetadata.{promptTokenCount, candidatesTokenCount, totalTokenCount}`
  - Azure OpenAI: same as OpenAI
- NetworkPolicy template `charts/spendguard/templates/networkpolicy.yaml` gated by `values.networkPolicy.enabled`
- L2 bypass-resistance: K8s NetworkPolicy forbids egress except via proxy

---

## §3. Out of scope

| 項目 | 推給 |
|---|---|
| Multi-region routing | Phase 2+ |
| Provider-specific authentication caching | SLICE-extra |
| New provider additions (Mistral / DeepSeek) | post-launch enhancement |

---

## §4. File-level change list

### 4.1 New files

- `services/egress_proxy/src/routing.rs` — ProviderConfig + routing table
- `services/egress_proxy/src/providers/openai.rs`
- `services/egress_proxy/src/providers/anthropic.rs`
- `services/egress_proxy/src/providers/bedrock.rs`
- `services/egress_proxy/src/providers/vertex.rs`
- `services/egress_proxy/src/providers/azure_openai.rs`
- `services/egress_proxy/src/usage_extractors/` directory per provider
- `charts/spendguard/templates/networkpolicy.yaml`

### 4.2 Modified files

- `services/egress_proxy/src/forward.rs` — delete hard-coded constants; use routing table
- `services/egress_proxy/src/decision.rs` — pass provider context to estimate_call_cost (extends SLICE_10)

---

## §5. Schema / proto changes

No proto changes. Internal routing table only.

---

## §6. Audit-chain impact

- `cloudevent_payload.provider` field captures which provider this call routed to
- `tokenizer_version_id` correctly reflects per-provider encoder
- Backward compat: existing OpenAI audit rows unaffected

---

## §7. Failure mode coverage

| 場景 | 行為 |
|---|---|
| Unknown inbound path | 404 + `unknown_inbound_path` metric |
| Provider 5xx | proxy returns 5xx; audit row records OUTCOME=PROVIDER_ERROR |
| NetworkPolicy not enabled | works as before (L2 capability advertised as advisory until policy enabled) |
| Provider response-usage missing | sidecar falls to estimated commit per Contract §5 commit state machine |
| Bedrock model dispatch unknown | tokenizer Tier 3 + metric; proxy still forwards |

---

## §8. Acceptance criteria

### 8.1 Unit tests

- Each provider's request_shape correctly handled
- Each provider's usage_extractor parses real response samples (5+ per provider)
- Routing table dispatches correctly for all inbound paths

### 8.2 Integration tests

- Real OpenAI request → forward correctly
- Real Anthropic request → forward + audit row with anthropic_kind tokenizer
- Real Bedrock anthropic.claude-* → correctly tokenizes via Anthropic BPE
- LiteLLM proxy with multi-provider tenant: each call's provider correctly identified

### 8.3 Helm tests

- NetworkPolicy renders correctly when enabled
- ChartProfile=production refuses if NetworkPolicy disabled (depends on existing helm gate logic)

### 8.4 Demo-mode regression

- `make demo-up DEMO_MODE=multi_provider_usd` shows correct provider-routed handling
- All 8+ demos pass

### 8.5 Backwards compat

- Existing OpenAI / Chat Completions / Responses API requests unaffected by behavior

---

## §9. Slice-specific adversarial review checklist

1. Hard-coded constants at forward.rs:36-37 actually removed?
2. Per-provider request_shape: do they handle streaming SSE correctly? (per existing `egress-proxy-v0.2-streaming-sse.md`)
3. Bedrock model dispatch table aligned with tokenizer dispatch (SLICE_04)?
4. NetworkPolicy schema validated against K8s ≥ 1.21?
5. NetworkPolicy enforcement actually blocks egress (chaos test verifies)?
6. Anthropic cache_creation_input_tokens / cache_read_input_tokens: treated separately in audit?
7. Vertex authentication: GCP IAM vs API key vs ADC? Which paths supported?
8. Azure OpenAI: deployment_id routing handled?
9. Concurrent requests for different providers: routing table thread-safe?
10. Unknown provider routing fall-through: 404 vs forward-with-warning?

---

## §10. Out-of-scope deferrals

| 項目 | 推給 |
|---|---|
| Real provider key gateway (L3) | Post-launch |
| Per-tenant provider allowlist | Post-launch enhancement |
| Streaming response token counting per provider | Use existing SSE path |

---

## §11. Risk / rollback plan

- Risk: routing table misclassifies provider → wrong tokenizer applied → systematic under/over estimate
- Mitigation: per-provider integration tests; demo regression
- Rollback: revert SLICE_11; egress proxy reverts to OpenAI-only forwarding

---

## §12. Review Execution Notes

- Recommended reviewer profile: Backend Architect
- Review depth: deep
- Expected rounds: 3-4 (provider integration + NetworkPolicy details)

---

## §13. Adoption history (filled during review)

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder) |

---

## §14. Merge checklist

- [ ] §8 acceptance green
- [ ] §9 specific clear
- [ ] universal §1.7 (L0-L3 capability semantics unchanged; L2 still works) verified
- [ ] All providers tokenize correctly per SLICE_04 dispatch
- [ ] NetworkPolicy renders + enforces
- [ ] PR references multiple specs

---

*Slice version: SLICE_11_multi_provider_routing v1alpha1 (draft) | Spec ancestors: tokenizer-service-spec §3.1 + L2 capability README | Depends: SLICE_04 | Branch: `slice/SLICE_11_multi_provider_routing`*
