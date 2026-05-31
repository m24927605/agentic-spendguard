# HARDEN 08 SVID certificate validation

## Implementation summary

- `output_predictor` accepts `SPENDGUARD_OUTPUT_PREDICTOR_PLUGIN_CLIENT_SVID_DIR`.
- Runtime layout is `<svid_dir>/<client_cert_id>/{tls.crt,tls.key,ca.crt}`.
- `client_cert_id` is restricted to `[A-Za-z0-9_-]` before path construction.
- The client cert is parsed before use and must contain exactly one URI SAN:
  `spiffe://spendguard.platform/predictor-client/<tenant_id>`.
- Channel cache identity now includes endpoint URL, server fingerprint,
  `client_cert_id`, and mounted material fingerprint. Secret rotation changes
  the material fingerprint and forces the next request to build a fresh channel.
- Helm renders cert-manager `Issuer` and per-binding `Certificate` resources
  when `outputPredictor.pluginClientSvid.enabled=true`.
- The reference plugin validates peer certificate SVID subject against
  `PredictRequest.tenant_id` and fails closed on missing or mismatched SVID
  when mTLS client CA is configured.
- R1 hardening requires `--tls-client-ca` whenever the reference plugin starts
  in TLS mode; TLS without client auth is no longer a supported "secure" mode.
- Control-plane plugin registration/update validation now uses the same
  `[A-Za-z0-9_-]{1,44}` `client_cert_id` shape that Helm and runtime mounts require.
- R2 hardening bounds SVID reload cached-channel reuse to 60 seconds and
  makes the reference plugin reject extra SPIFFE URI identities.
- R3 hardening starts the 60-second rotation grace window at first reload
  failure, prevents Kubernetes resource-name truncation collisions by capping
  `client_cert_id` to 44 bytes, requires URI SAN rather than commonName in
  the reference plugin, and extends the real mTLS test to reject tenant-B
  SVIDs while accepting same-tenant rotated material.
- R4 hardening applies bounded rotation grace to new-channel failures during
  material changes, uses the bounded `plugin-client-svid-<clientCertId>`
  Certificate name without release-name truncation, and rejects any extra URI
  SAN identity in the reference plugin.

## Local verification

```text
cargo test --manifest-path services/output_predictor/Cargo.toml -- --nocapture
PASS: 150 lib tests, 7 binary tests, 20 integration tests/doc tests

cargo test --manifest-path services/control_plane/Cargo.toml validate_register -- --nocapture
PASS: 8 validation tests

python3 -m pytest contrib/output_predictor_template/conformance_test.py -q
PASS: 70 passed

helm template spendguard charts/spendguard --set chart.profile=demo
PASS

helm template spendguard charts/spendguard -f scripts/helm-validate-test-values.yaml
PASS

helm template spendguard charts/spendguard -f scripts/helm-validate-test-values.yaml \
  --set outputPredictor.pluginEndpointDatabaseEnabled=true \
  --set outputPredictor.pluginClientSvid.enabled=true \
  --set outputPredictor.pluginClientSvid.issuer.create=true \
  --set outputPredictor.pluginClientSvid.issuer.caSecretName=spendguard-plugin-client-ca \
  --set 'outputPredictor.pluginClientSvid.bindings[0].tenantId=018fcf9a-3d2d-7b37-9f21-0f27de0b20c1' \
  --set 'outputPredictor.pluginClientSvid.bindings[0].clientCertId=tenant-018fcf9a'
PASS: Certificate URI SAN and SVID mount rendered

helm template spendguard charts/spendguard -f scripts/helm-validate-test-values.yaml \
  --set outputPredictor.pluginEndpointDatabaseEnabled=true \
  --set outputPredictor.pluginClientSecretName=legacy-global
PASS: render failed closed without explicit legacy opt-in

make demo-up DEMO_MODE=plugin_c_synthetic
PASS: breaker regression, real Rust PluginClient mTLS/SVID integration,
and Python reference plugin SVID fail-closed checks ran successfully

helm template ... clientCertId=45-byte-string
PASS: render failed closed with `clientCertId must match ^[A-Za-z0-9_-]{1,44}$`
```
