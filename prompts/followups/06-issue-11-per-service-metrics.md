# Followup #11 — Per-service /metrics endpoints (S23)

GitHub issue: https://github.com/m24927605/agentic-flow-cost-evaluation/issues/11

## Goal

Every Rust service should expose a Prometheus `/metrics` endpoint matching
`canonical_ingest`'s pattern. PR #2 S23 shipped
`deploy/observability/prometheus-rules.yaml` with 8 alert groups and
`docs/site/docs/operations/slos.md` with L1-L9 SLO targets, but only
canonical_ingest has actually wired metrics. The alert rules show NaN
otherwise.

This must land **before** issue #12 (per-drill runbooks) — the drill
rehearsals depend on these alerts firing.

## Files to read first

- `services/canonical_ingest/src/metrics.rs` — **reference impl**. Copy
  this pattern. Notes:
  - No `prometheus` crate dependency — uses `AtomicU64` + manual
    Prometheus text-format render. Keeps the dep graph lean
  - Counters per-route + per-quarantine-reason
  - Round 1 P2#3 added `unknown_key_admitted_total` +
    `invalid_signature_admitted_total` (non-strict-mode admit counters)
- `services/canonical_ingest/src/main.rs:serve_metrics` — hyper-based
  HTTP server on a separate port (default 9091)
- `deploy/observability/prometheus-rules.yaml` — alert names tell you
  what counters / histograms each service must emit
- `docs/site/docs/operations/slos.md` — L1-L9 numeric SLO targets

## Services that need metrics endpoints

| Service | Port | Key counters / histograms (derive from prometheus-rules.yaml) |
|---|---|---|
| ledger | 9092 | post_*_transaction_total, post_*_transaction_seconds, audit_outbox_lag_seconds |
| sidecar | 9093 | decision_total{kind}, decision_latency_seconds, contract_evaluator_seconds, fencing_lease_renew_total |
| webhook_receiver | 9094 | webhook_received_total{provider,event_kind}, webhook_dedupe_hit_total, webhook_hmac_failure_total |
| outbox_forwarder | 9095 | outbox_forwarded_total, outbox_pending, lease_state_total{state}, forward_seconds |
| ttl_sweeper | 9096 | sweep_cycles_total, sweep_released_total, lease_state_total{state} |
| control_plane | 9097 | api_requests_total{path,status}, api_seconds, approval_resolved_total{state} |
| dashboard | 9098 | api_requests_total{path,status}, api_seconds |
| usage_poller | 9099 | poll_cycles_total, fetched_total{provider}, inserted_total, deduped_total, http_seconds |

## Acceptance criteria

- Each service has a `src/metrics.rs` module mirroring canonical_ingest's
  shape: `IngestMetrics` (rename per service) with `Arc<Inner>`,
  `inc_*` methods, `render() -> String` that emits Prometheus text format
- Each `main.rs` spins up a `serve_metrics` task on the new port,
  separately from the gRPC / HTTP service port. No new heavy dep — reuse
  hyper + http-body-util that canonical_ingest already pulls in
- Helm chart updates:
  - `charts/spendguard/templates/<service>.yaml`: add the metrics port
    to `containerPort` + the matching `Service` block + `podMonitor`
    annotation `prometheus.io/scrape: "true"` and `prometheus.io/port:
    "<metrics_port>"`
  - `values.yaml`: per-service `metricsPort` knob with the documented
    default
- `compose.yaml`: same port mappings so demo docker compose can scrape
  via host (`localhost:909{2..9}/metrics`)
- One per-service smoke test: `curl -sS service:port/metrics | grep
  -F "# HELP "` returns a non-empty payload, the docs counter shows up
  with `... 0` baseline reading
- 8 unit tests (one per service) similar to canonical_ingest's
  `metrics::tests::counters_default_to_zero_in_render_output`
- Update `prometheus-rules.yaml` `runbook_url` annotations now that
  the alerts will actually fire — defer the per-runbook docs themselves
  to issue #12

## Pattern references

- canonical_ingest's metrics.rs is the canonical answer (no pun
  intended). The 4 round-9 admit counters added in PR #2 are good
  examples of how to evolve metrics over time without breaking the
  trait
- For Helm Service + ports, look at how the existing
  `canonical-ingest.yaml` exposes its 9091 metrics port post-round-1
  (the dup-ports merge fix from commit `a4dea4b`)

## Verification

```bash
# Compose demo
make demo-up DEMO_MODE=decision
for port in 9091 9092 9093 9094 9095 9096 9097 9098 9099; do
  echo "=== port $port ==="
  curl -sS localhost:$port/metrics | head -5 || echo "FAIL"
done

# Helm template smoke
helm template t charts/spendguard --set ... | grep -E "containerPort: 909[1-9]" | sort -u
```

## Commit + close

```
feat(s23): per-service /metrics endpoints (followup #11)

Eight services (ledger, sidecar, webhook_receiver, outbox_forwarder,
ttl_sweeper, control_plane, dashboard, usage_poller) each get a
Prometheus /metrics endpoint mirroring canonical_ingest's shape.

Helm + compose wire the new ports. prometheus-rules.yaml alerts
now have data to evaluate against; per-drill runbooks (issue #12)
can be authored against actually-firing alerts.

Tests: 8 unit tests + smoke curl across all 9 metrics ports under
DEMO_MODE=decision.
```

After merge: `gh issue close 11 --comment "Shipped in <commit-sha>"`.
