# Output Predictor Plugin Certification Checklist

This checklist defines the pass/fail bar for a customer Strategy C
plugin. A plugin is certified for a tenant only when every required item
has evidence.

## Certification States

| State | Meaning |
|---|---|
| Not certified | Missing required evidence or failed conformance. Strategy C remains disabled. |
| Provisionally certified | Conformance and mTLS pass in staging; production traffic is limited to a low-risk tenant slice. |
| Certified | Staging and production smoke evidence pass, audit events are present, and rollback has been rehearsed. |
| Suspended | A regression, SVID mismatch, severe latency, or model quality issue requires disabling Strategy C. |

## Required Evidence

| Area | Requirement | Evidence |
|---|---|---|
| Template baseline | Fork is pinned to a SpendGuard commit SHA. | Fork metadata or release record. |
| Proto compatibility | Generated bindings match `proto/spendguard/output_predictor_plugin/v1/plugin.proto`. | `bash gen_proto.sh` output or committed generated files. |
| Conformance | Full corpus passes. | `python3 -m pytest conformance_test.py -q`. |
| Tenant binding | Different tenant IDs are rejected. | `test_tenant_mismatch_rejected` and SVID mismatch tests pass. |
| mTLS | Plaintext production traffic is impossible. | Deployment manifest has server cert/key, client CA, and an explicit `command` or `args` override that prevents the reference image default `CMD ["--insecure"]`; alternatively, runtime evidence proves the process did not start with `--insecure`. |
| SVID subject | Client cert URI SAN exactly equals `spiffe://spendguard.platform/predictor-client/<tenant_id>`. | `openssl x509 -ext subjectAltName` output. |
| Server fingerprint | Control plane registration pins the plugin server certificate fingerprint. | Registration request and response. |
| Timeout | Predict completes inside the 50 ms budget under expected load. | Load or soak result with p99. |
| Circuit breaker | Plugin failures fall back to Strategy B. | Metric sample for `customer_predictor_call_total{outcome="fall_to_b"}` or a controlled failure test. |
| Audit | Registration/update/reset lifecycle events land in canonical audit. | Query or verify-chain evidence. |
| Model quality | Held-out calibration is documented. | Backtest output with model version and sample size. |
| Operations | Health check and rollback commands are known to on-call. | Runbook link and rehearsal log. |

## Hard Fail Conditions

Certification fails if any of these are true:

- The plugin accepts a SpendGuard client certificate without the exact
  tenant SVID URI SAN.
- A single endpoint serves more than one tenant.
- Production deployment uses plaintext or `--insecure`.
- A reference-image deployment omits an explicit `command` or `args`
  override that disables the image default `CMD ["--insecure"]`, unless
  runtime proof shows the process started in mTLS mode.
- `Predict` returns zero, negative, overflowing, or unbounded token
  predictions.
- `confidence` is outside `[0.0, 1.0]`.
- The plugin lacks a non-empty `plugin_version` or `feature_hash`.
- Health check returns `SERVING` while the model cannot answer requests.
- The customer cannot produce conformance test output for the deployed
  plugin version.

## Local Certification Command

Run from the template directory:

```bash
python3 -m pytest conformance_test.py -q
```

The current template suite covers:

- 50 happy-path requests across prompt classes and model families,
- timeout,
- gRPC internal error,
- zero or negative prediction,
- overflow,
- invalid confidence,
- deserialization or semantic input error,
- TLS handshake failure,
- `NOT_SERVING` health,
- tenant ID mismatch,
- SVID mismatch,
- common-name-only identity rejection,
- missing SVID fail-closed behavior,
- oversized request field rejection,
- concurrency smoke.

## Production Readiness Review

Before enabling broad traffic, the SpendGuard operator and customer
owner should sign off on:

- exact tenant UUID and endpoint mapping,
- server certificate renewal process,
- SpendGuard predictor-client CA refresh process,
- rollback command for disabling the endpoint,
- alert routing for plugin latency and failure modes,
- backtest acceptance threshold and retrain owner,
- audit evidence retention location.

The plugin remains customer-owned. SpendGuard certifies the wire,
security, and operational behavior, not the customer's internal model
training process.
