# D13 — `DEMO_MODE=subscription_meter`

Subscription-tier meter mode for Claude Code Pro / Codex on ChatGPT
Plus.  Three-step cap walk against the in-tree `subscription_meter`
Rust module.

## Run

```bash
make -C deploy/demo demo-up DEMO_MODE=subscription_meter
```

## What it does

The runner exercises three cap-decision scenarios:

| Step | current | delta | alert_at | hard | Expected            |
|------|--------:|------:|---------:|-----:|---------------------|
| 1    |       0 |   100 |    1 000 | 2 000| `Pass`              |
| 2    |     950 |    50 |    1 000 | 2 000| `SoftCapAlert`      |
| 3    |   1 900 |   200 |    1 000 | 2 000| `HardCapBlock` (429)|

The runner asserts:

1. Each cap evaluation returns the correct decision kind.
2. Hard-cap returns a Retry-After in `[1, 86_400]` seconds.
3. The counting-stub recorded **zero** upstream calls (meter never
   forwards on its own).
4. The synthetic 429 body shape carries `error.code =
   "spendguard_subscription_cap"` so CLIs can distinguish a
   SpendGuard-injected 429 from a vendor-injected one.

## Verification gates

After the runner exits, the Makefile drives
`deploy/demo/verify_step_subscription_meter.sql` which asserts the
**load-bearing invariants**:

* **Zero `ledger_entries`** rows under the meter tenant.  Subscription
  meter MUST NOT write to the BYOK ledger — that would double-count
  the flat fee as phantom dollars (design §4.3).

* **Zero `reservations`** rows under the meter tenant.  Same reason.

* `subscription_meters`, `subscription_alerts`, and
  `subscription_import_jobs` tables exist (proof that migrations
  0044 / 0045 / 0046 ran).

The runner's own stdout (`SUBSCRIPTION_METER_DEMO_OK`) is the
positive gate for the three-step cap walk.

## Tenant ID

The runner uses tenant `00000000-0000-4000-8000-00000000d013` for
all three steps.  This is fixed via
`SPENDGUARD_SUBSCRIPTION_METER_TENANT_ID` in `docker-compose.yaml`.

## Legal posture

This demo does NOT call api.anthropic.com or chatgpt.com — that
would charge real tokens against a real BYOK key.  The runner is
self-contained: it evaluates the cap math in pure Python (mirroring
the Rust module under test) and never touches the upstream provider.
The counting-stub is included to verify the **negative** invariant
that the meter never forwards.

## See also

* `docs/specs/coverage/D13_subscription_meter/design.md`
* `docs/specs/coverage/D13_subscription_meter/implementation.md`
* `services/sidecar/src/subscription_meter/` — authoritative impl
* `services/ledger/migrations/004{4,5,6}_subscription_*.sql`
* `services/ledger/src/subscription_importer/` — D14/D15/D16 stubs
