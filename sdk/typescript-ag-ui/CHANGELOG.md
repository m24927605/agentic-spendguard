# `@spendguard/ag-ui` Changelog

All notable changes to the SpendGuard AG-UI spend-event package.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This package adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The Python mirror — `spendguard.integrations.ag_ui` in
[`spendguard-sdk`](https://pypi.org/project/spendguard-sdk/) — is kept
byte-equivalent on the canonical serialization via the shared frozen fixture
corpus `sdk/fixtures/cross-language/ag_ui_v1.json`. See
[`docs/specs/coverage/D39_ag_ui/design.md`](../../docs/specs/coverage/D39_ag_ui/design.md)
for the locked vocabulary.

---

## [0.1.0] — 2026-06-10

First release (coverage deliverable D39, slice 1).

**Display-only**: these events are a presentation surface — SpendGuard
enforcement happens in the SpendGuard adapters and sidecar before the
provider call; the events report decisions already made and can neither grant
nor deny spend.

### Added

- The five-event `spendguard.*` AG-UI CUSTOM vocabulary
  (`SPENDGUARD_AG_UI_EVENT_NAMES`):
  `spendguard.budget.snapshot`, `spendguard.reservation.created`,
  `spendguard.reservation.committed`, `spendguard.reservation.released`,
  `spendguard.decision.denied`.
- Pure, clock-free builders for all five events
  (`buildBudgetSnapshot`, `buildReservationCreated`,
  `buildReservationCommitted`, `buildReservationReleased`,
  `buildDecisionDenied`) — every ID is an input; payload keys reuse ASP
  Draft-01 vocabulary verbatim where concepts overlap; every payload carries
  `schema_version: "1"`.
- `canonicalEventJson` — the locked cross-language canonical JSON rule
  (sorted keys, UTF-8, no whitespace, ASCII-only keys, integer-only numbers,
  null/float/-0/unpaired-surrogate rejection).
- `encodeSse` — `"data: " + canonicalEventJson(event) + "\n\n"` data-only SSE
  framing, plus the `AgUiEmit` transport callback type.
- `AgUiEventValidationError` with payload-key `field` attribution.
- Cross-language fixture corpus `sdk/fixtures/cross-language/ag_ui_v1.json`
  (22 vectors) minted by the reference generator `generate_ag_ui.mjs`;
  frozen at merge — the Python mirror consumes it byte-for-byte.
- Pinned `@ag-ui/core@0.0.56` compat suite (type assignability + runtime
  `CustomEventSchema` parse); `@ag-ui/core` is an optional peer
  (`>=0.0.27 <0.1.0`) and is never imported at runtime.
- Bundle budget enforced as a build failure: minified ≤ 8 KB, gzipped
  ≤ 3 KB, tarball ≤ 25 KB. Zero runtime dependencies; no `node:` imports
  (browser-safe).

### Known limitations

- `amount_atomic_observed` is reserved: builders accept and emit it when
  supplied (forward-compat), but SpendGuard's observed-amount commit lane is
  backlog — the only commit lane today is `CommitEstimated`
  (`amount_atomic_estimated`).
- `caps` (ASP `audit.reserve` when ALLOW_WITH_CAPS) is deliberately not in
  v0.1.0; the display story for `ALLOW_WITH_CAPS` is the decision value +
  `reason_codes`.
