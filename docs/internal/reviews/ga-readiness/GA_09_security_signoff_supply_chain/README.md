# GA_09 Security Scan Evidence

- Result: pass
- Commit: `a28a3fab8f6f2f046c8c6d097e9582d5bdace813`
- Branch: `ga/GA_09_security_signoff_supply_chain`
- Started UTC: `2026-06-01T08:41:27Z`
- Worktree dirty at start: `false`
- Missing optional external tools: none
- Optional external scanner failures: none
- Release-mode command: `scripts/security/ga-security-scan.sh --require-external-tools`

## Checks

- PASS `runtime_dockerfiles_user_65532`: all runtime Dockerfiles set USER 65532:65532
- PASS `publish_workflow_trivy`: Trivy scan step present
- PASS `publish_workflow_sbom`: Buildx SBOM enabled
- PASS `publish_workflow_provenance`: Buildx provenance enabled
- PASS `publish_workflow_cosign`: cosign signing step present
- PASS `publish_workflow_no_latest_promotion`: no latest/latest-main promotion
- PASS `publish_workflow_oidc`: OIDC permission present for keyless signing
- PASS `publish_workflow_repo_scan_single_job`: repository Trivy scan runs once before the image matrix
- PASS `publish_workflow_dispatch_has_sha_tag`: manual dispatch publishes immutable sha tag
- PASS `sidecar_image_precreates_secret_links`: sidecar image prepares root-owned paths before USER switch
- PASS `sidecar_entrypoint_nonroot_safe`: sidecar entrypoint only verifies mounted paths after USER switch
- PASS `pki_volume_chowned_for_runtime_uid`: pki-init hands cert/key volume to runtime UID 65532
- PASS `pki_ca_key_remains_root_only`: pki-init keeps demo CA private key out of runtime UID
- PASS `bundles_volume_chowned_for_runtime_uid`: bundles-init hands writable bundle volume to runtime UID 65532
- PASS `compose_sidecar_uds_volume_handoff`: compose hands existing sidecar UDS named volume to runtime UID before sidecar starts
- PASS `publish_workflow_covers_production_chart_images`: publish workflow covers 12 production chart images under spendguard/<component>
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
- PASS `evidence_no_local_paths`: GA_09 evidence strips developer-local paths
