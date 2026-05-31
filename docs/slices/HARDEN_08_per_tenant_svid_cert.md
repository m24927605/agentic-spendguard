# HARDEN 08 — Per-tenant SVID certificate minting

> **Branch**: `harden/HARDEN_08_per_tenant_svid_cert`
> **Status**: implementation in review
> **Spec ancestor(s)**: `predictor-upgrade-hardening-spec-v1alpha1.md`, `output-predictor-plugin-contract-v1alpha1.md`
> **Depends on prior slices**: HARDEN_01 through HARDEN_07
> **Blocks subsequent slices**: production-ready completion
> **Estimated change size**: medium-large; cert-manager templates, cert issuer pipeline, plugin mTLS validation, tests

---

## §0. TL;DR

Implement the SLICE_07 deferred per-tenant SVID pipeline. SpendGuard mints a per-tenant client certificate with subject `spiffe://spendguard.platform/predictor-client/<tenant_id>`, mounts it for predictor-to-plugin calls, and plugin endpoint validation rejects mismatched tenant subjects.

---

## §1. Architectural context

SLICE_07 shipped Strategy C with global client cert fingerprint pinning as an interim control. Production isolation requires per-tenant identity so a compromised or misconfigured plugin connection cannot cross tenant boundaries. This slice closes GH #171 and removes the major identity deferral from the predictor upgrade.

---

## §2. Scope (must-do)

- Cert issuer pipeline for per-tenant SVID client certs
- Subject format exactly `spiffe://spendguard.platform/predictor-client/<tenant_id>`
- cert-manager Issuer template
- cert-manager Certificate template per tenant/plugin binding
- Helm chart mounts cert/key/CA bundle for predictor-to-plugin connection
- output_predictor plugin client selects the tenant-specific cert for each request
- Plugin endpoint validation checks client cert subject tenant matches PredictRequest tenant
- Rotation path with overlap window and no hot-path outage
- Tests for correct subject, mismatch rejection, missing cert fail-closed, and rotation
- Close or update GH #171 with commit and verification

---

## §3. Out of scope

| Item | Pushed to |
|---|---|
| Full SPIRE server deployment | cert-manager-based SVID-shaped certs are enough for this hardening slice |
| Non-plugin service mesh identity | Future platform identity work |
| Customer-managed CA onboarding UI | Future control-plane UX |

---

## §4. File-level change list

### 4.1 New files

- `charts/spendguard/templates/output_predictor_plugin_svid.yaml`
- `services/output_predictor/src/plugin_svid.rs`
- `services/output_predictor/tests/plugin_svid_mtls.rs`
- `contrib/output_predictor_template/svid_validation.py`
- `docs/reviews/hardening/HARDEN_08/svid-cert-validation.md`

### 4.2 Modified files

- `services/output_predictor/src/plugin_client.rs` and endpoint cache wiring
- `services/output_predictor/src/config.rs`
- `services/output_predictor/src/main.rs`
- `charts/spendguard/templates/output_predictor.yaml`
- `charts/spendguard/values.yaml` and production values documentation
- `contrib/output_predictor_template/predictor_server.py` and `mtls_setup.md`
- `docs/slices/SLICE_07_output_predictor_plugin_c.md` adoption/history note if needed

---

## §5. Schema / proto changes

No public proto changes expected. Control-plane plugin binding schema may gain certificate resource references if existing binding rows cannot identify the tenant cert secret.

---

## §6. Audit-chain impact

- Plugin registration/update audit events should include whether per-tenant SVID is enabled for the binding
- Tenant-binding violation from certificate subject mismatch must be auditable and fail closed
- Cert rotation events should be operator-audited if control-plane state changes
- Strategy C fallback to B remains allowed only for recoverable plugin failures, not tenant binding violations

---

## §7. Failure mode coverage

| Scenario | Expected behavior |
|---|---|
| Cert subject tenant differs from request tenant | Reject with TenantBindingViolation / failed_precondition |
| Tenant cert missing | Fail closed for Strategy C binding or fall to B only if configured as recoverable and no binding violation occurs |
| Cert expired | Reject and mark plugin unhealthy |
| Rotation produces old and new certs | Both accepted during overlap if CA and tenant subject are valid |
| Plugin omits client-cert validation | Conformance test fails |
| Helm production lacks Issuer/Certificate config | Template or readiness gate fails |

---

## §8. Acceptance criteria

### 8.1 Certificate minting

- Helm renders Issuer and Certificate resources for tenant plugin bindings
- Certificate subject exactly matches the required SPIFFE URI shape

### 8.2 Predictor client

- output_predictor loads/selects tenant-specific client cert per plugin request
- Rotation reload path is tested or documented with bounded restart behavior

### 8.3 Plugin validation

- Reference plugin validates client certificate subject against request tenant
- Mismatch test rejects cross-tenant request

### 8.4 Helm and security gates

- Demo and production Helm templates render
- Production profile refuses insecure global-client-cert-only Strategy C binding unless explicitly marked legacy

### 8.5 Demo-mode regression

- `make demo-up DEMO_MODE=plugin_c_synthetic` runs with per-tenant cert validation enabled

---

## §9. Slice-specific adversarial review checklist

1. Is the certificate subject exactly `spiffe://spendguard.platform/predictor-client/<tenant_id>`?
2. Is `tenant_id` parsed and compared as `uuid::Uuid`, not raw string?
3. Can tenant A's cert ever call tenant B's plugin binding?
4. Does output_predictor select certs per request, not only at process boot?
5. Does cert rotation avoid downtime or clearly require bounded rolling restart?
6. Do Helm templates render cert-manager resources only when enabled and required?
7. Does production profile reject insecure legacy global cert use?
8. Does the reference plugin validate client cert subject, not just CA?
9. Are tenant binding violations auditable and not silently falling back?
10. Is GH #171 closed only after the validation demo/test passes?

---

## §10. Out-of-scope deferrals

| Item | Why deferred |
|---|---|
| SPIRE workload API integration | Larger platform identity project |
| Multi-cluster trust federation | Future enterprise deployment |
| UI for certificate lifecycle | Control-plane UX phase |

---

## §11. Risk / rollback plan

- Risk: cert-manager dependency complicates demo installs. Mitigation: gate resources and document prerequisite; demo may use generated local certs with the same subject shape.
- Risk: rotation logic creates brief Strategy C outage. Mitigation: overlap window and fallback-to-B for recoverable unavailability, never for tenant mismatch.
- Rollback: disable Strategy C per-tenant SVID only by reverting this slice and reopening #171; production should not use legacy global cert mode.

---

## §12. AIT execution notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must inspect cert subject construction, tenant UUID comparison, Helm production gates, and cross-tenant rejection tests.

---

## §13. Adoption history

| Round | Reviewer / panelist | Decision | Outcome |
|---|---|---|---|
| Design | Software Architect | Use cert-manager SVID-shaped certs instead of introducing SPIRE server now | §3 keeps SPIRE out of scope |
| Design | Backend Architect | Predictor must select cert by tenant per request | §8 and §9 require per-request selection |
| Design | Security Engineer | Tenant mismatch is fail-closed, not fallback-to-B | §6, §7, and §9 require rejection |
| Design | Database Optimizer | Binding schema changes are allowed only if existing rows cannot reference cert secret | §5 constrains schema |
| Design | Plugin domain expert | Reference plugin must validate subject, not just trust CA | §8.3 and §9 require it |
| Implementation | Backend Architect | Reuse existing `predictor_plugin_endpoints.client_cert_id`; no schema migration needed | `client_cert_id` selects the SVID subdirectory |
| Implementation | Security Engineer | Include mounted cert/key/CA fingerprint in channel cache identity | Rotation rebuilds channels on the next request |
| Review R1 | codex CLI adversarial reviewer | Fix 4 Majors + 1 Minor: TLS without client CA, rotation reload fallback, control-plane `client_cert_id` validation, real mTLS demo/test, exact URI SAN | Commit `0209c42` closes all R1 findings |
| Review R2 | codex CLI adversarial reviewer | Bound rotation fallback, cap `client_cert_id` for K8s-safe names, reject extra Python peer SVID URI identities | Commit `4c34877` closes all R2 findings |

---

## §14. Merge checklist

- [x] Per-tenant cert Issuer/Certificate templates render
- [x] output_predictor uses tenant-specific certs
- [x] Reference plugin validates subject tenant
- [x] Cross-tenant mismatch test fails closed
- [x] `plugin_c_synthetic` demo runs with SVID validation
- [ ] GH #171 closed with fixing commit
- [ ] AIT adversarial review passes or Staff+ arbitration is recorded (R1/R2 fixed; R3 pending)

---

*Slice version: HARDEN_08_per_tenant_svid_cert v1alpha1 | Spec ancestor: predictor-upgrade-hardening-spec-v1alpha1 | Branch: `harden/HARDEN_08_per_tenant_svid_cert`*
