# GA_05 Observability Dashboards Evidence

Slice: `docs/internal/slices/GA_05_observability_dashboards.md`

## Staff+ Implementation Decisions

| Role | Decision |
|---|---|
| SRE/Operations Architect | Dashboard panels must use emitted metrics, not aspirational SLO names. |
| Backend Architect | Predictor and projector p99 panels require service-owned histograms, not static gauges. |
| Security Engineer | SVID failures are represented through bounded Strategy C `tls_error` failure mode, without tenant labels. |
| Database Optimizer | Audit lag is an every-pod oldest-pending-row gauge over `audit_outbox`, with leader count shown separately so no-leader states do not mask backlog growth. |
| Product/Domain Expert | Replay dedup and drift alerts are first-class dashboard panels because they map to customer-visible audit-chain trust. |
| Staff+ Arbitration | R5 rustfmt finding is fixed anyway; audit lag polling must be backed by `audit_outbox_pending_age_idx` and verified by EXPLAIN before merge. |

## Acceptance Evidence

Concrete command results are captured in `command-results.md`. The acceptance set includes dashboard JSON parsing, metric inventory validation, Rust builds/tests for affected services, Helm demo/production rendering, clean-state `make demo-up DEMO_MODE=default`, live scrape checks for canonical ingest, outbox forwarder, and run cost projector metrics, and EXPLAIN evidence for the audit lag index.
