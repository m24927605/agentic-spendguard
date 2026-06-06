# D01 — Envoy AI Gateway ExtProc Sidecar — Acceptance Gates

**Companion to:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md)
**Build plan §3 contract:** every gate is runnable in the current repo state. No third-party-blocking dependency.

A reviewer can re-run any gate below with no privileged access beyond what's in the repo.

---

## §1. Build gates (per slice; cumulative at end)

1. `cargo build -p spendguard-provider-routing --release` exits 0. (SLICE 1)
2. `cargo build -p spendguard-envoy-extproc --release` exits 0. (SLICE 1)
3. `cargo build -p spendguard-egress-proxy --release` still exits 0 after `routing.rs` extraction. (SLICE 1)
4. `cargo build --workspace --release` (existing default) still exits 0. (SLICE 1)
5. The new service container image builds via `docker build -f services/envoy_extproc/Dockerfile .` and image size < 80 MiB. (SLICE 6)

## §2. Unit + integration test gates

6. `cargo test -p spendguard-provider-routing` — all tests in `tests.md` §1.1 pass. (SLICE 1)
7. `cargo test -p spendguard-envoy-extproc --lib` — all unit tests in `tests.md` §1.2 through §1.7 pass. (SLICE 1-4)
8. `cargo test -p spendguard-envoy-extproc --test '*'` — all integration tests in `tests.md` §2 pass. (SLICE 1-4)
9. `cargo test -p spendguard-envoy-extproc --test conformance` — all conformance tests in `tests.md` §2.1 pass against committed golden fixtures. (SLICE 5)
10. `cargo test -p spendguard-egress-proxy` — no regression in existing egress_proxy tests after the routing-table extraction. (SLICE 1)

## §3. Demo gates

11. `make demo-up DEMO_MODE=envoy_extproc` brings up the topology, runs the demo client, and exits 0 within 180s. (SLICE 7)
12. `psql ... -f deploy/demo/verify_step_envoy_extproc.sql` returns the expected ≥ 5 audit_decision + audit_outcome rows with `runtime_kind = 'envoy-ai-gateway'`. (SLICE 7)
13. `verify-chain` regression at end of demo run reports 0 failures across all `envoy_extproc`-produced rows. (SLICE 7)
14. `make demo-up DEMO_MODE=decision` still passes (regression). (SLICE 1)
15. `make demo-up DEMO_MODE=proxy` still passes (regression). (SLICE 1)
16. `make demo-up DEMO_MODE=multi_provider_usd` still passes (regression after `routing.rs` extraction). (SLICE 1)
17. `make demo-up DEMO_MODE=approval` still passes (regression). (SLICE 1)
18. `make demo-down` cleans up; no leftover containers or volumes. (SLICE 7)

## §4. Helm gates

19. `helm lint charts/spendguard` exits 0 after the new `envoy_extproc.yaml` template is added. (SLICE 6)
20. `helm template charts/spendguard --set envoyExtproc.enabled=true --set tenant_id=test` renders without missing-required-value errors. (SLICE 6)
21. `helm template charts/spendguard` (envoyExtproc.enabled defaults to false) does NOT render the new Deployment — proves the chart is additive. (SLICE 6)
22. The rendered NetworkPolicy includes an ingress rule for port 8443 from pods labeled `app.kubernetes.io/name: envoy-ai-gateway`. (SLICE 6)
23. The rendered SVID volume mount uses the same `csi.spiffe.io` driver as `output_predictor_plugin_svid.yaml`. (SLICE 6)
24. Kind-cluster Helm install (existing `ci/helm_kind_validate.sh` style) of the chart with `envoyExtproc.enabled=true` reports all pods Ready within 120s. (SLICE 6)

## §5. Benchmark gates

25. `cargo bench -p spendguard-envoy-extproc bench_token_counting_openai_p99` reports p99 < 1ms. (SLICE 5)
26. `cargo bench -p spendguard-envoy-extproc bench_token_counting_anthropic_p99` reports p99 < 1ms. (SLICE 5)
27. `cargo bench -p spendguard-envoy-extproc bench_full_extproc_roundtrip_p99` reports p99 < 50ms (Contract §14 hot-path budget). (SLICE 5)

## §6. Security gates

28. `cargo audit -p spendguard-envoy-extproc` reports no Critical / High advisories. (SLICE 1)
29. `trivy image $(docker build -q services/envoy_extproc)` reports no Critical / High vulnerabilities. (SLICE 6)
30. The compiled binary contains no UDS-style code paths (`grep -rn "SocketAddr::Unix\|/run/spendguard/sidecar.sock" services/envoy_extproc/src/` returns 0 matches) — confirms §3.3 mTLS-over-TCP decision is enforced. (SLICE 1)
31. Fail-closed posture: a test that points `SPENDGUARD_EXTPROC_SIDECAR_URI` at an unresponsive address returns 503 within `SPENDGUARD_EXTPROC_REQUEST_TIMEOUT_MS + 50ms`, never 200. (SLICE 3)
32. SVID cert SPIFFE URI SAN pinning enforced — a sidecar presenting `spiffe://other-tenant/sidecar` is rejected (per `tests.md` §1.7 `tls_pins_sidecar_spiffe_uri`). (SLICE 1)

## §7. Conformance gates (per design §3.5 scope)

33. The committed golden fixture `tests/conformance/envoy_v06_token_counting.yaml.json` byte-equals the response stream produced when the harness replays the Envoy AI Gateway v0.6 reference `token_counting.yaml` ExtProc trace. (SLICE 5)
34. The committed golden fixture `tests/conformance/envoy_v06_budget.yaml.json` byte-equals the response stream produced when the harness replays the Envoy AI Gateway v0.6 reference `budget.yaml` ExtProc trace. (SLICE 5)
35. ExtProc TRAILERS phase is unimplemented — calling it returns `Status::unimplemented` with an explicit message (matches design §3.5 anti-scope). (SLICE 1)

## §8. Audit chain gates

36. Every ExtProc-driven `audit_decision` row carries `runtime_kind = 'envoy-ai-gateway'` (asserted by SQL in gate 12). (SLICE 4)
37. Every ExtProc-driven `audit_outcome` row's `decision_id` matches an existing `audit_decision` row (referential integrity — asserted in gate 12). (SLICE 4)
38. No ExtProc code path writes to `audit_outbox` directly; all audit emission flows through the existing sidecar adapter `EmitTraceEvents` RPC. (verified by `grep -rn "audit_outbox\|canonical_events" services/envoy_extproc/src/` returns 0 matches). (SLICE 1)
39. The `cloudevent_payload_signature` on ExtProc-driven rows verifies via existing `verify-chain` script unchanged. (SLICE 4)

## §9. Observability gates

40. `curl http://localhost:9090/metrics` against a running `envoy-extproc` pod exposes:
    - `envoy_extproc_handler_total{handler,outcome}` counters for each ExtProc phase
    - `envoy_extproc_request_decision_latency_seconds` histogram
    - `envoy_extproc_token_count_latency_seconds` histogram
    - `envoy_extproc_sidecar_unreachable_total` counter
    (SLICE 3, 6)
41. `curl http://localhost:9090/readyz` returns 200 only when the sidecar handshake has succeeded; 503 otherwise. (SLICE 1)
42. `curl http://localhost:9090/livez` returns 200 as long as the process is running, even when the sidecar is unreachable. (SLICE 1)
43. Structured JSON logs include `tenant_id`, `session_id`, `decision_id`, `run_id` on every decision and outcome line. (SLICE 3, 4)

## §10. Documentation gates

44. `docs/site/docs/integrations/envoy-ai-gateway.md` exists and shows an Envoy `ExternalProcessor` config snippet pointing at the new service. (SLICE 7)
45. `README.md`'s `## Adapter integrations` table has a new row: `Envoy AI Gateway | ExtProc sidecar | Tier 1 | docs/site/docs/integrations/envoy-ai-gateway.md`. (SLICE 7)
46. `CHANGELOG.md` has an entry for the new deliverable under the next unreleased version. (SLICE 7)

## §11. Definition-of-done summary

The deliverable is shipped when:

- All 46 gates above run green from the merge commit
- Memory write-back per build plan §8 created at `~/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/project_coverage_D01_shipped.md`
- A 1-paragraph entry exists in `README.md` `## Adapter integrations` table (gate 45)
- The 7 slices listed in [`design.md`](design.md) §4 have all merged to main with R1-passing reviews
