# Tokenizer Provider Key Rotation Runbook

Scope: Tier 1 shadow provider credentials for tokenizer verification.

Credential sources:

- `SPENDGUARD_TOKENIZER_ANTHROPIC_API_KEY`
- `SPENDGUARD_TOKENIZER_GEMINI_API_KEY`

The tokenizer loads provider API keys at process start. There is no
in-process hot reload path for these credentials; rotate by updating the
Kubernetes Secret and rolling the tokenizer Deployment.

## Rotation

1. Update the Secret that backs the tokenizer Deployment. Do not put key
   material in Helm values, logs, tickets, or this runbook.

   ```sh
   kubectl -n <namespace> create secret generic <tokenizer-provider-secret> \
     --from-literal=anthropic-api-key='<redacted>' \
     --from-literal=gemini-api-key='<redacted>' \
     --dry-run=client -o yaml | kubectl apply -f -
   ```

2. Restart tokenizer pods so the new environment values are loaded.

   ```sh
   kubectl -n <namespace> rollout restart deployment/<release>-tokenizer
   kubectl -n <namespace> rollout status deployment/<release>-tokenizer
   ```

3. Watch the shadow queue drain. The operational expectation inherited
   from SLICE_05 is sample queue lag p99 below 30 seconds; allow a
   60-second drain window before declaring the rotation unhealthy.

   ```sh
   kubectl -n <namespace> logs deployment/<release>-tokenizer --since=5m
   ```

4. Verify Tier 1 shadow resumes without hot-path impact:

   - `tokenizer_shadow_queue_lag_seconds` returns below the 30-second p99
     objective.
   - `tokenizer_provider_auth_failures_total` does not increase after the
     restart window.
   - Tier 2 Tokenize latency remains within the normal p99 envelope.

## Rollback

Re-apply the prior Secret version and repeat the rollout restart. The
hot path remains Tier 2/Tier 3 only; Tier 1 shadow outages must not block
reservations.
