# Output Predictor Plugin Error Taxonomy

This taxonomy maps Strategy C failures to SpendGuard metrics, fallback
behavior, and customer/operator action. It is written for on-call use and
customer certification reviews.

## Core Rule

Plugin failure never blocks SpendGuard enforcement. SpendGuard records
the failure, falls back to Strategy B where possible, and keeps the
deterministic decision path intact.

## SpendGuard Failure Modes

| Mode | Metric label | Typical trigger | SpendGuard behavior | Customer action | Operator action |
|---|---|---|---|---|---|
| Timeout | `timeout` | `Predict` exceeds 50 ms. | Fall to Strategy B; breaker failure count increases. | Profile model path and cache hot features. | Check latency dashboards; disable endpoint if sustained. |
| gRPC error | `grpc_error` | Plugin returns non-OK status not classified below. | Fall to Strategy B. | Inspect plugin logs for request id. | Confirm no rollout or dependency outage. |
| Invalid zero/negative | `invalid_zero_or_negative` | `predicted_output_tokens <= 0`. | Reject Strategy C result and fall to B. | Fix model adapter bounds. | Keep endpoint disabled until conformance passes. |
| Invalid overflow | `invalid_overflow` | Prediction exceeds model context window or integer bounds. | Reject Strategy C result and fall to B. | Clamp or retrain model. | Confirm audit rows show Strategy B fallback. |
| Invalid confidence | `invalid_confidence` | `confidence < 0`, `confidence > 1`, or NaN. | Reject Strategy C result and fall to B. | Fix confidence calibration. | Require conformance rerun. |
| Deserialization error | `deserialization_error` | Invalid wire format or semantic request decode failure. | Fall to Strategy B. | Regenerate proto bindings and redeploy. | Check template/proto version drift. |
| TLS error | `tls_error` | Chain failure, fingerprint mismatch, expired cert, or SVID mismatch. | Fall to Strategy B; tenant isolation metric may increment. | Rotate certs and validate SVID URI SAN. | Confirm control-plane fingerprint and CA bundle. |
| Not serving | `not_serving` | Health probe reports `NOT_SERVING`. | Skip Strategy C until health recovers. | Fix model or dependency state. | Keep breaker open and notify customer. |
| Not configured | `not_configured` | No enabled endpoint for tenant. | Use Strategy B without treating it as an incident. | Complete onboarding. | Verify registration record. |
| Breaker open | `breaker_open` | Failure threshold reached. | Skip calls until recovery probe succeeds or reset is requested. | Stabilize plugin before reset. | Use force reset only after root cause is corrected. |

Tenant isolation violations are also tracked through
`customer_predictor_tenant_isolation_violation_total`. Treat any increase
as a security incident until proven to be a synthetic test.

## Template gRPC Status Guidance

The reference template returns:

| Template condition | gRPC status |
|---|---|
| Missing tenant ID | `INVALID_ARGUMENT` |
| Tenant ID mismatch | `INVALID_ARGUMENT` |
| Oversized model/request field | `INVALID_ARGUMENT` |
| Missing required SVID when `require_client_svid=true` | `PERMISSION_DENIED` |
| Model exception | `INTERNAL` |
| Client deadline exceeded | `DEADLINE_EXCEEDED` observed by caller |
| Plaintext client to TLS server | `UNAVAILABLE` observed by caller |

Do not encode retryable customer dependency outages as successful
predictions. Return a clear gRPC error and let SpendGuard fall back.

## Retry And Circuit Breaker Rules

- Hot-path `Predict` must be idempotent by `spendguard_call_id`.
- The plugin should not perform unbounded internal retries inside the
  50 ms budget.
- SpendGuard does not require retries to make a prediction succeed.
- Repeated failures open the SpendGuard-side circuit breaker.
- Health recovery or operator force reset is required before normal
  Strategy C traffic resumes.

## Audit And Logs

Every plugin registration, update, and force reset is recorded as a
signed audit event. Prediction failures are counted in metrics; they do
not create one audit event per failed plugin call. Operators should
correlate:

- `customer_predictor_failure_mode_total`,
- `customer_predictor_call_total`,
- `customer_predictor_tenant_isolation_violation_total`,
- plugin logs keyed by `spendguard_call_id`,
- lifecycle audit events.

Never delete or mutate audit rows to hide plugin failures.
