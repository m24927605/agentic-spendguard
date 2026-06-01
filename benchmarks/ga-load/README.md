# GA Load Harness

`benchmarks/ga-load/run.sh` drives the GA_08 real-stack scale smoke gate against the demo compose stack. It is not the Contract §14 latency certification gate; `spendguard-predictor-upgrade-benchmarks` owns that SLO.

The local scenario uses the single demo tenant that has a signed bundle and SVID identity in compose, then fans out 100 logical tenant workloads through distinct run, agent, model, provider, and prompt-class buckets. That keeps the path real-stack without fabricating tenant identities the sidecar would correctly reject.

## Merge Gate

```bash
benchmarks/ga-load/run.sh --scenario benchmarks/ga-load/scenarios/local-100-tenants.yaml
```

The harness:

- boots the compose stack with tokenizer, output_predictor, run_cost_projector, sidecar, canonical_ingest, stats_aggregator, and outbox_forwarder
- runs the Python SDK inside the demo container over the sidecar UDS
- calls tokenizer and output_predictor gRPC directly on the compose network before every sidecar decision
- fills the Strategy C audit mirror with a conservative synthetic value because the local compose stack does not mount a customer predictor plugin
- records p50/p95/p99/max latency for tokenizer, output_predictor, sidecar decision, publish confirmation, trace emit, and end-to-end request time
- verifies zero operation errors, outbox drain, canonical audit integrity, and live service metric counters
- writes evidence under `docs/reviews/ga-readiness/GA_08_scale_performance_slo_proof/`

Use `--no-reset` only for targeted debugging. Merge evidence must come from a clean source commit and a fresh stack.
