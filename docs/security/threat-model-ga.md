# GA Threat Model

Status: GA_09 release gate

## Scope

This model covers the predictor upgrade GA deployment path:

- sidecar, tokenizer, output_predictor, run_cost_projector, stats_aggregator
- canonical_ingest and ImmutableAuditLog routing
- Strategy C customer plugin mTLS
- production Helm values and published images
- release bundle, SBOM, provenance, vulnerability scan, and image signing

## Assets

| Asset | Security objective |
|---|---|
| Tenant budget and audit data | Tenant isolation, append-only audit history, replay-resistance |
| Contract/schema bundle identity | Integrity and provenance |
| Provider prompt text | No raw-text provider shadow without tenant opt-in |
| Plugin client certificate | Per-tenant identity, no cross-tenant credential reuse |
| Database credentials | Secret-only delivery, least privilege, RLS preserved |
| Container images | Non-root runtime, signed digest, reproducible SBOM/provenance |

## Threats And Controls

| Threat | Control |
|---|---|
| Cross-tenant plugin call through reused client cert | Per-tenant SVID Certificate with exact URI SAN and runtime subject validation |
| CloudEvent replay or tamper | `canonical_event_replay_dedup` keyed by `(producer_id, event_id)` plus payload hash mismatch rejection |
| RLS bypass through privileged DB role | GA_09 scan rejects executable `BYPASSRLS`; production uses Secret-bound least-privilege URLs |
| PII exfiltration through tokenizer shadow | `pii_shadow_enabled=false` default and tenant allowlist check before provider call |
| Provider quota exhaustion through `count_tokens` | Per-tenant quota claim path in tokenizer shadow security store |
| Mutable image tag substitution | Publish workflow does not publish `latest` or `latest-main`; production values validator rejects mutable tags |
| Unsigned or unscanned image promotion | Publish workflow runs Trivy, emits SBOM/provenance, and signs pushed digests with cosign OIDC |
| Root container breakout amplification | Runtime Dockerfiles set `USER 65532:65532`; Helm enforces non-root, read-only filesystem, no privilege escalation, and dropped capabilities |
| Plaintext database URL in operator config | Production values validator and GA_09 scan reject plaintext DB URLs |

## Checklist

- [x] Per-tenant SVID URI format documented and rendered by Helm.
- [x] Runtime validator enforces exact SVID prefix.
- [x] Replay dedup migration and append path exist.
- [x] Tokenizer PII shadow default is deny.
- [x] Tokenizer `count_tokens` quota path exists.
- [x] Production values are credential-free.
- [x] Runtime images run as UID 65532.
- [x] Publish workflow has SBOM/provenance/signing/scanning.
- [x] Mutable `latest` promotion is removed from publish workflow.
- [x] Local scan gate emits evidence under `docs/internal/reviews/ga-readiness/GA_09_security_signoff_supply_chain/`.

## Open Assurance Item

External penetration testing remains a future external audit. It is not required to merge GA_09, but any critical finding from such an audit blocks release until fixed or formally accepted outside this all-AI workflow.
