# @spendguard/sdk (TypeScript)

> **Status:** PRE-RELEASE substrate. `version: 0.0.0`. First published release
> (`0.1.0`) ships in slice `COV_S05_10`.

This is the TypeScript half of the SpendGuard SDK — the runtime safety layer
client for AI agent frameworks. It mirrors the Python `spendguard-sdk`
(currently v0.5.1 on PyPI) so D04 / D06 / D08 / D29 adapter packages
(`@spendguard/langchain`, `@spendguard/vercel-ai`, `@spendguard/openai-agents`,
`@spendguard/inngest-agentkit`) have a single shared substrate to build against.

## What lives here

This directory hosts the npm package `@spendguard/sdk`. The public surface,
architecture decisions, slice plan, and review standards are tracked in the
spec set:

- [`docs/specs/coverage/D05_ts_sdk_substrate/design.md`](../../docs/specs/coverage/D05_ts_sdk_substrate/design.md)
  — public surface contract (§4), architecture (§6), locked decisions (§9),
  bundle-size budget (§10).
- [`docs/specs/coverage/D05_ts_sdk_substrate/implementation.md`](../../docs/specs/coverage/D05_ts_sdk_substrate/implementation.md)
  — repo layout, module skeletons, codegen pipeline.
- [`docs/specs/coverage/D05_ts_sdk_substrate/review-standards.md`](../../docs/specs/coverage/D05_ts_sdk_substrate/review-standards.md)
  — review checklist applied to every D05 slice.

The slice index for this deliverable lives at
[`docs/slices/COV_S05_*.md`](../../docs/slices/).

## Slice progress

| Slice | Status |
|---|---|
| `COV_S05_01_d05_package_init` | This slice — package skeleton lands. |
| `COV_S05_02` – `COV_S05_10` | Pending. See spec set §8. |

## Why this slice is mostly empty

Per the slice doc anti-scope, `COV_S05_01` lays the package skeleton only.
No `src/*.ts` files, no `tests/`, no codegen, no publish workflow. Those
arrive in slices `COV_S05_02` through `COV_S05_10`. The npm scripts in
`package.json` are placeholder no-ops that exit `0` so workspace-level CI
gates pass; real wiring lands per the table above.

## Locked decisions (design.md §9)

These are not re-litigated by slice authors:

1. **ESM-only.** No CJS. `"type": "module"`.
2. **`reserve()` is the canonical method name**; `requestDecision` is an alias.
3. **camelCase on the public surface, snake_case on the wire.**
4. **`queryBudget()` ships as a public method with placeholder implementation**
   in v0.1.x — sidecar wire is a follow-up.
5. **Biome over ESLint+Prettier.**
6. **protobuf-ts over ts-proto** (stability over feature velocity).
7. **OTel as `peerDependencyMeta.optional`** — adapters that never enable OTel
   pay zero deps cost.
8. **Tokenizer is OUT of v0.1.x scope.**
9. **Pricing snapshot embedded** under `@spendguard/sdk/pricing/demo`.
10. **No browser** — UDS transport only in v0.1.x.

## License

Apache-2.0 (same as the parent repo).
