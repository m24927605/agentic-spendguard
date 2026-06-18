# HARDEN_D05_UR_S01 — TS SDK substrate broadening

> **Pass**: HARDEN_D05_UR
> **Slice**: 1 of 4 (S — small, surgical)
> **Spec set**: [`docs/specs/harden_d05_unit_ref/`](../../specs/harden_d05_unit_ref/)

## Scope

The SUBSTRATE-only slice. After this slice ships:
- TS SDK `UnitRef` interface has `unitId?: string` optional field
- `mapUnitRef` threads `unit.unitId ?? ""` to the wire
- Locked-surface tests assert the new shape
- Wire-shape tests assert the threading
- Backward-compat tests assert no breaking change

NO adapter changes (those land in SLICE 2 — D04+D06+D08+D29 TS adapters; SLICE 3 — Python adapters; SLICE 4 — demo overlays + verify SQL restoration + memory + sign-off).

## Files touched

| File | Why |
|------|-----|
| `sdk/typescript/src/client.ts` | `UnitRef.unitId?: string` + `mapUnitRef` thread-through + comment rewrite |
| `sdk/typescript/tests/locked-surface.test.ts` | U-LS-01 + U-LS-02 |
| `sdk/typescript/tests/unit-id-wire.test.ts` | NEW — U-WS-01..06 + XA-01 |
| `sdk/typescript/CHANGELOG.md` | Unreleased entry |

## Test plan

Per [`tests.md`](../../specs/harden_d05_unit_ref/tests.md) §1.1, §1.2, §1.3, §3:
- **U-LS-01** `UnitRef.unitId` is OPTIONAL string at type level
- **U-LS-02** 2-field shape (no `unitId`) still assignable
- **U-WS-01** `unitId` provided → threads verbatim
- **U-WS-02** `unitId` omitted → wire shows ""
- **U-WS-03** multi-claim case — all claims independently
- **U-WS-04** `projectedUnit` path also threads
- **U-WS-05** `commitEstimated` wire path also threads
- **U-WS-06** explicit `""` is identity-preserved
- **XA-01** Cross-adapter smoke against MockSidecar

Plus regression: ≥263 existing tests pass.

## Anti-scope

- ❌ Adapter options changes (SLICE 2/3)
- ❌ Demo overlay changes (SLICE 4)
- ❌ Python SDK changes (none needed — already correct)
- ❌ .NET SDK changes (SLICE 2 if D07 needs it; verify first)
- ❌ Verify SQL changes (SLICE 4)
- ❌ Memory updates (SLICE 4)

## Acceptance gates (per [`acceptance.md`](../../specs/harden_d05_unit_ref/acceptance.md) §1)

1. `pnpm run typecheck` clean
2. `pnpm run lint` clean (biome)
3. `pnpm run build` clean; dist/index.js minified delta < 200 bytes
4. `pnpm run test` — ≥271 tests pass (263 baseline + ≥8 new)
5. Identity invariant: `reserve === requestDecision === true`
6. Bundle still ≤ 120 KB minified main bundle
7. No Python SDK regression (Python tests still pass)

## Reviewer

Per [`review-standards.md`](../../specs/harden_d05_unit_ref/review-standards.md) §8, dispatch Claude Code CLI reviewer (`superpowers:code-reviewer` subagent) with the LOCKED prompt template.

## Backlinks

- Spec set: [`design.md`](../../specs/harden_d05_unit_ref/design.md) §1.1, §1.2, §2.1, §2.2; [`implementation.md`](../../specs/harden_d05_unit_ref/implementation.md) §1
- Memory: [[project_coverage_phase_b]] — single open cross-slice gap
- Memory: [[feedback_reviewer_claude_code_only]] — Claude Code CLI reviewer mandate
