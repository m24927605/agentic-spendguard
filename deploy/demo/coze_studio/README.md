# D31 SLICE 3 — `DEMO_MODE=coze_studio_real`

End-to-end demo proving SpendGuard gates every Coze Studio model call
through the sidecar HTTP companion. Headline acceptance per
`docs/specs/coverage/D31_coze_studio/acceptance.md` §G6.

## What this exercises

| Step | Body | Asserts |
|------|------|---------|
| 1. ALLOW | small prompt that fits the demo budget | companion reserve precedes upstream, real-ish usage commit, **counting-stub hit count = 1**, INV-2 strict-order |
| 2. DENY | request fingerprint pre-seeded to deny | companion returns 502, **counting-stub hit count UNCHANGED**, INV-1 satisfied, DENY row in audit chain |
| 3. STREAM | SSE-shaped call | end-of-stream commit, `decision_context->>'stream' = 'true'`, INV-5 |

## Run

```bash
make demo-down
export OPENAI_API_KEY=sk-...   # any value works — the demo uses the
                               # counting-stub upstream, not real OpenAI
make demo-up DEMO_MODE=coze_studio_real
```

On success the runner prints the literal line:

```
[demo] coze_studio_real ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

and the verify gate prints:

```
D31_COZE OK: coze decisions=N (N >= 2)
```

The Makefile target then runs `demo-verify-coze-studio-real` which
executes `verify_step_coze_studio_real.sql` and the
outbox-closure check.

## What the overlay adds

5 new services on top of the base `compose.yaml` (all with
`spendguard-coze-` container name prefix):

- `counting-stub` — mock OpenAI provider. Provides `/v1/chat/completions`
  + `/_count` + `/healthz`.
- `coze-postgres` — separate Postgres for Coze metadata (INV-8: never
  co-tenants with `spendguard_ledger`). Volume `coze-postgres-data`.
- `coze-redis` — Coze Studio's job queue.
- `coze-studio` — Coze Studio container, pinned by SHA256 digest
  (INV-9). **Off by default** behind `profiles: [coze]`. Operators who
  want the full Coze UI run `COMPOSE_PROFILES=coze make demo-up
  DEMO_MODE=coze_studio_real`.
- `coze-seed` — one-shot init that idempotently seeds the demo tenant +
  budget + open window-instance in `spendguard_ledger`. See
  `seed_workspace.sql`.

## Why coze-studio is off by default

Coze Studio's image is ~1.5 GB. The demo headline assertions (INV-1
through INV-5) are proved at the companion contract boundary — that is
the same code path Coze Studio invokes internally when a workspace
executes a chat-flow that resolves to the SpendGuard-gated OpenAI
provider. Pulling the full Coze stack on every workstation run is the
wrong tradeoff for a 30s-boot demo.

This mirrors the dify_plugin overlay's deviation note: the upstream
plugin daemon path (`SpendGuardLLM._invoke`) is what proves the
integration; the Workspace HTTP frontend is downstream of that and out
of scope for D10's value proposition. Same reasoning here.

Operators who DO want the Coze UI:

```bash
COMPOSE_PROFILES=coze make demo-up DEMO_MODE=coze_studio_real
```

This activates the `coze-studio` container behind the `coze` profile.
The driver then exercises Coze's chat-flow API in addition to the
direct companion path. Note: Coze's API requires its own bootstrap
flow (admin user creation, workspace setup) that's vendor-specific —
the profile path is a starting point; full UI-driven tests stay
manual.

## Files

- `compose.override.yaml` — the overlay (loaded after the base
  `compose.yaml`).
- `seed_workspace.sql` — idempotent ledger seed.
- `README.md` (this file).

The verify SQL + Makefile branch + Python driver live in adjacent
parent locations:

- `../verify_step_coze_studio_real.sql` — ledger-DB assertions.
- `../Makefile` — `DEMO_MODE=coze_studio_real` branch.
- `../../examples/coze-studio/client.py` — 3-step matrix driver.

## Troubleshooting

### `coze-seed` fails with `relation "budgets" does not exist`

The base demo stack hasn't fully migrated yet. Run `make demo-down`
and then `make demo-up DEMO_MODE=coze_studio_real`; the seed depends
on `ledger` being healthy which gates on migrations.

### `coze-runner` exits 9

The runner driver hit a gate failure. Check `make demo-logs` for the
`[demo] coze_studio_real FAIL` line — the body identifies which step
(ALLOW / DENY / STREAM) regressed.

### `counting-stub` shows hits during DENY (INV-1 regression)

Worst correctness bug — DENY MUST NOT hit upstream. Re-check that the
companion's contract bundle has a deny rule for the demo tenant's
DENY-step fingerprint. The verify SQL `stub_hits` assertion would
also have failed.

### `coze-postgres` and `postgres` collide

Both want their default port. Compose isolates them by service name —
no host port mapping. If you've port-mapped one for inspection, the
other is still reachable inside the `spendguard-net` network. If you
see "container name already in use" errors, run `make demo-down -v`
then bring up again.

## See also

- `docs/specs/coverage/D31_coze_studio/acceptance.md` — full acceptance
  gates.
- `examples/coze-studio/README.md` — operator-facing install path.
- `examples/coze-studio/client.py` — demo driver source.
