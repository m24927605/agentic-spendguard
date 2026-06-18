# Contributing to SpendGuard

Thanks for your interest in SpendGuard. This is an Apache-2.0, single-maintainer
project (development status: Beta). Contributions, issues, and questions are
welcome.

Because SpendGuard sits in the LLM egress path and makes money-affecting
ALLOW/DENY decisions, the bar for changes that touch the decision path, the
ledger, the audit chain, or the wire protocol is high. When in doubt, open an
issue first.

## Ground rules

- **Fail-closed always.** Any change to the decision, reservation, commit, or
  release path must preserve fail-closed behavior: on any error, timeout, or
  ambiguity, refuse spend — never allow it. A change that can turn a `DENY`
  into an `ALLOW` will not be merged.
- **Wire and schema are append-only.** `proto/` and the SQL `migrations/`
  trees evolve by *adding* backwards-compatible changes. Never edit an
  already-applied migration; add a new one (`CREATE OR REPLACE` for functions).
  Open an issue before touching `proto/` or any `migrations/` directory.
- **Minimal, surgical diffs.** Match the surrounding code's style, naming, and
  error-handling idioms. Avoid unrelated refactors in a functional change.

## Repository layout

| Path | What's there |
|---|---|
| `services/` | Rust services, each its **own independent Cargo project** |
| `crates/` | Shared Rust libraries (tokenizer, prediction mirror, provider routing) |
| `sdk/python`, `sdk/typescript` | Client SDKs |
| `plugins/`, `integrations/` | Gateway plugins and framework integrations |
| `proto/` | gRPC/Protobuf wire definitions (append-only) |
| `charts/` | Helm chart |
| `deploy/demo/` | Docker Compose demo stack + demo modes |
| `docs/specs/` | Authoritative versioned specs (source of truth) |
| `benchmarks/`, `scripts/`, `tests/` | Benchmarks, verification scripts, E2E |

## Prerequisites

- **Rust** (stable, 2021 edition) with `cargo`.
- **Node 20+** and **pnpm** (TypeScript SDK / integrations).
- **Python 3.10+** (Python SDK).
- **Docker** + Docker Compose (demo and integration tests).
- **PostgreSQL 16** for ledger work (the demo provisions this for you).

## Building and testing

Each Rust service builds independently — there is no shared workspace, so
build the one you changed:

```bash
cd services/<service> && cargo check --tests   # fast type/compile check
cd services/<service> && cargo test             # unit tests (some need Postgres)
```

SDKs and integrations build via their own package scripts:

```bash
# TypeScript
pnpm --filter @spendguard/sdk build
pnpm --filter @spendguard/sdk typecheck

# Python
cd sdk/python && python -m pytest      # and: ruff check src/
```

Helper verification scripts live in `scripts/` (e.g.
`verify-cargo-workspace.sh`, `helm-validate-kind.sh`,
`verify-migrations-postgres16.sh`).

## The demo is a quality gate

SpendGuard treats the end-to-end demo as a first-class test. A change to the
core flow is not "done" until at least the default demo passes:

```bash
make demo-up DEMO_MODE=deny      # hard-cap → STOP, the headline fail-closed path
make demo-down                   # always tear down (removes named volumes)
```

Many integrations ship their own `DEMO_MODE=<name>` gate (see the README's
demo-modes list). If you add or change an integration, add or update its demo
gate and confirm it is green.

> Tip: run `make demo-down` before bringing the demo back up, to wipe stale
> volumes from a previous run.

## Commit and PR conventions

- Use clear, imperative commit subjects (e.g. `fix(ledger): enforce
  BUDGET_EXHAUSTED hard cap in reserve path`). Conventional-commit prefixes
  (`feat`, `fix`, `docs`, `chore`) are encouraged.
- Keep a PR focused on one logical change. Describe *what* changed and *why*,
  and note any wire/migration impact.
- Make sure the affected component builds and its demo gate (if any) passes
  before requesting review.

## Reporting bugs and proposing features

- Bugs and feature requests: open a GitHub issue using the templates in
  `.github/ISSUE_TEMPLATE/`.
- **Security issues: do not open a public issue.** Follow
  [`SECURITY.md`](SECURITY.md).

## Code of conduct

By participating you agree to abide by our
[Code of Conduct](CODE_OF_CONDUCT.md).
