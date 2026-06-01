# Customer Output Predictor Plugin Onboarding

This guide is the customer-facing path for taking a Strategy C output
predictor plugin from a fork of the reference template to production
traffic. It intentionally keeps SpendGuard's safety invariant front and
center: plugin failure never blocks enforcement. SpendGuard falls back
to Strategy B and records metrics/audit evidence.

## Preconditions

- A SpendGuard tenant UUID provisioned by the control plane.
- A fork of `contrib/output_predictor_template/`.
- A trained or stub-replaced model that implements the template model
  surface.
- A Kubernetes namespace or equivalent runtime that can mount TLS
  secrets read-only.
- Access to the SpendGuard control plane endpoint registration API.
- `python3`, `pytest`, `grpcio`, and `cryptography` for local
  certification.

## Integration Sequence

1. Fork the reference template and pin the SpendGuard commit SHA used
   for the fork.
2. Regenerate the plugin proto bindings with `bash gen_proto.sh` if the
   `_proto/` directory is not already committed in your fork.
3. Replace `model_predictor_stub.py` with your trained model adapter.
   Keep the same `predict_one`, `confidence`, `sample_size`, and
   `MODEL_VERSION` semantics.
4. Run the conformance suite locally:

   ```bash
   cd contrib/output_predictor_template
   python3 -m pytest conformance_test.py -q
   ```

5. Mint the plugin server certificate in your environment. SpendGuard
   does not issue the plugin server identity.
6. Download or receive the SpendGuard predictor-client CA bundle and
   mount it into the plugin container.
7. Configure the plugin to require mTLS client certificates and exact
   tenant SVID matching:

   ```bash
   PREDICTOR_TENANT_ID="${TENANT_ID}"
   PREDICTOR_TLS_SERVER_CERT=/certs/server/tls.crt
   PREDICTOR_TLS_SERVER_KEY=/certs/server/tls.key
   PREDICTOR_TLS_CLIENT_CA=/trust/spendguard-ca.pem
   PREDICTOR_REQUIRE_CLIENT_SVID=true
   ```

   The reference template image defaults to `CMD ["--insecure"]` for
   local development. Production deployments must override container
   args so `--insecure` is not passed. For Kubernetes, set explicit
   args such as:

   ```yaml
   args:
     - --port
     - "50054"
   ```

   The TLS environment variables above are then loaded by
   `predictor_server.py`, and startup fails closed if any required TLS
   path is missing.

8. Register the endpoint and server certificate fingerprint with the
   SpendGuard control plane.
9. Deploy at least two plugin replicas before enabling production
   Strategy C traffic.
10. Observe plugin call metrics, health checks, and audit events for one
    low-risk tenant slice before widening traffic.

## Security Contract

The plugin must validate both the certificate chain and the client SVID
URI SAN. The only accepted SpendGuard client identity for tenant
`${TENANT_ID}` is:

```text
spiffe://spendguard.platform/predictor-client/${TENANT_ID}
```

The plugin must reject:

- missing client certificates,
- non-SPIFFE or common-name-only identities,
- more than one URI SAN identity,
- a valid SpendGuard SVID for a different tenant,
- plaintext production traffic,
- shared multi-tenant endpoints.

SpendGuard renders per-tenant client SVID material under
`outputPredictor.pluginClientSvid.bindings[]`. A production deployment
must not use the legacy global client certificate mode unless an
explicit migration exception is configured and time-bounded.

## Runtime Expectations

SpendGuard calls `Predict` inside the hot path. The plugin must keep the
following budgets:

| Budget | Requirement |
|---|---|
| Predict deadline | 50 ms hard cap. Late responses are timeout failures. |
| TLS connect/handshake | 500 ms cap before the endpoint is considered unreachable. |
| Health probe | Should answer within 2 seconds and return `SERVING`, `DEGRADED`, or `NOT_SERVING`. |
| Retry discipline | The plugin must be idempotent by `spendguard_call_id`; SpendGuard does not rely on retries to make a hot-path prediction succeed. |
| Circuit breaker | Repeated failures open the SpendGuard-side breaker and skip Strategy C until recovery probes pass. |

The plugin must return bounded values:

- `predicted_output_tokens > 0`
- `predicted_output_tokens <= model context window`
- `0.0 <= confidence <= 1.0`
- non-empty `plugin_version`
- non-empty `feature_hash`

Invalid responses are treated as plugin failures and fall back to
Strategy B.

## Registration Flow

Register one endpoint per tenant. The endpoint URL and SHA-256 server
certificate fingerprint are stored by the control plane and forwarded to
the output predictor endpoint cache. `client_cert_id` must equal the
matching `outputPredictor.pluginClientSvid.bindings[].clientCertId`
value, which selects the SpendGuard-issued client SVID directory mounted
under `/etc/spendguard/plugin-client-svid/${CLIENT_CERT_ID}`.

```bash
curl --fail -X POST \
  --header "Authorization: Bearer ${SPENDGUARD_API_TOKEN}" \
  --header "Content-Type: application/json" \
  --data @- \
  https://control-plane.spendguard.example/v1/predictor/plugins <<EOF
{
  "tenant_id": "${TENANT_ID}",
  "endpoint_url": "https://predictor.example.com:50054",
  "server_cert_fingerprint": "${SHA256_FINGERPRINT}",
  "client_cert_id": "${CLIENT_CERT_ID}"
}
EOF
```

The registration emits a signed `spendguard.audit.plugin_registered.v1alpha1`
event. Endpoint changes emit `spendguard.audit.plugin_updated.v1alpha1`.
Force reset operations emit `spendguard.audit.plugin_force_reset.v1alpha1`.

## Certification Evidence

Before production cutover, keep this evidence bundle with the customer
change record:

- SpendGuard commit SHA and plugin fork commit SHA.
- `python3 -m pytest conformance_test.py -q` output.
- `grpc_health_probe` output against the deployed endpoint.
- `openssl x509 -in /etc/spendguard/plugin-client-svid/${CLIENT_CERT_ID}/tls.crt -noout -ext subjectAltName`
  output from the SpendGuard-issued client SVID certificate showing the
  exact predictor-client SVID.
- Server certificate SHA-256 fingerprint used at registration.
- Backtest report showing held-out calibration and model version.
- Screenshot or export of plugin metrics for at least one low-risk
  traffic window.
- Audit query showing plugin registration/update events landed.

## Rollout And Rollback

Start with a single tenant and low traffic. Watch
`customer_predictor_call_total`, `customer_predictor_failure_mode_total`,
and `customer_predictor_tenant_isolation_violation_total`. If failures
rise or p99 latency exceeds the service budget, disable the endpoint in
the control plane. SpendGuard falls back to Strategy B without changing
the enforcement decision path.

Rollback does not require deleting audit rows. Keep plugin lifecycle
events intact and register a new endpoint or fingerprint when the fixed
plugin is ready.
