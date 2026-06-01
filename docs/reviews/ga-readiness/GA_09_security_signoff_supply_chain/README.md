# GA_09 Security Scan Evidence

- Result: pass
- Commit: `b7d7f878e44174a24840f3626b22f905a3caf2cd`
- Branch: `ga/GA_09_security_signoff_supply_chain`
- Started UTC: `2026-06-01T07:18:33Z`
- Missing optional external tools: none
- Release-mode command: `scripts/security/ga-security-scan.sh --require-external-tools`

## Checks

- PASS `runtime_dockerfiles_user_65532`: all runtime Dockerfiles set USER 65532:65532
- PASS `publish_workflow_trivy`: Trivy scan step present
- PASS `publish_workflow_sbom`: Buildx SBOM enabled
- PASS `publish_workflow_provenance`: Buildx provenance enabled
- PASS `publish_workflow_cosign`: cosign signing step present
- PASS `publish_workflow_no_latest_promotion`: no latest/latest-main promotion
- PASS `publish_workflow_oidc`: OIDC permission present for keyless signing
- PASS `production_values_no_plaintext_db`: no plaintext DB URL in production values
- PASS `production_render_no_plaintext_db`: no plaintext DB URL in production render
- PASS `production_render_has_networkpolicy`: NetworkPolicy rendered
- PASS `production_render_has_svid_certificate`: per-tenant SVID Certificate rendered
- PASS `rls_no_bypassrls_grants`: no executable BYPASSRLS grants
- PASS `replay_dedup_table`: replay dedup table exists
- PASS `replay_dedup_key`: producer/event and global event replay keys enforced
- PASS `pii_shadow_default_denied`: PII shadow default denies raw text
- PASS `pii_shadow_worker_guard`: shadow worker checks tenant opt-in
- PASS `count_tokens_quota_present`: per-tenant count_tokens quota present
- PASS `svid_template_exact_uri`: Helm Certificate URI uses exact predictor-client tenant prefix
- PASS `svid_runtime_exact_uri`: runtime validator uses exact predictor-client tenant prefix
- PASS `cargo_sbom_generated`: 226 Cargo packages recorded
