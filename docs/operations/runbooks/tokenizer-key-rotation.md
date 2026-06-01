# Tokenizer Provider Key Rotation Runbook

Scope: Tier 1 shadow provider credentials for tokenizer verification.

Credential sources:

- `SPENDGUARD_TOKENIZER_ANTHROPIC_API_KEY`
- `SPENDGUARD_TOKENIZER_GEMINI_API_KEY`

The tokenizer loads provider API keys at process start. There is no
in-process hot reload path for these credentials; rotate by updating the
Kubernetes Secret and rolling the tokenizer Deployment. The Helm chart
renders the Deployment as `<release>-spendguard-tokenizer`.

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
   kubectl -n <namespace> rollout restart deployment/<release>-spendguard-tokenizer
   kubectl -n <namespace> rollout status deployment/<release>-spendguard-tokenizer
   ```

3. Watch the restarted tokenizer pods. Current builds do not expose a
   queue-lag histogram or a provider-auth-failure counter; use the
   exported shadow counters plus logs until those metrics are added.

   ```sh
   kubectl -n <namespace> logs deployment/<release>-spendguard-tokenizer --since=5m
   kubectl -n <namespace> port-forward deployment/<release>-spendguard-tokenizer 9099:9099 &
   PF_PID=$!
   trap 'kill "${PF_PID}" 2>/dev/null || true' EXIT
   sleep 2
   curl -fsS http://127.0.0.1:9099/metrics | grep -E 'spendguard_tokenizer_shadow_(dropped_full|worker_dead)_total|spendguard_tokenizer_provider_count_tokens_schema_drift_total'
   kill "${PF_PID}" 2>/dev/null || true
   trap - EXIT
   ```

4. Verify Tier 1 shadow resumes without hot-path impact:

   - `spendguard_tokenizer_shadow_worker_dead_total` is not increasing.
   - `spendguard_tokenizer_shadow_dropped_full_total` is not increasing
     continuously after the rollout settles.
   - `spendguard_tokenizer_provider_count_tokens_schema_drift_total` is
     not increasing after the new keys are loaded.
   - Logs do not contain `schema-drift or auth error` after the restart
     window.
   - Tier 2 Tokenize latency remains within the normal p99 envelope.

## Rollback

Re-apply the prior Secret version and repeat the rollout restart. The
hot path remains Tier 2/Tier 3 only; Tier 1 shadow outages must not block
reservations.
