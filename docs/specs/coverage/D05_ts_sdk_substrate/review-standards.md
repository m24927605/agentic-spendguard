# D05 — Review Standards

Use this checklist with `superpowers:code-reviewer` on every D05 slice. R1 runs the full checklist; R2-R5 focus only on findings still open from the previous round. Findings are categorised P0 / P1 / P2 / Polish; P0 + P1 are blockers.

## 1. Public-surface lock (P0 — blocker)

The substrate is the contract D04 / D06 / D08 / D29 build against. Any change to the public surface after `design.md` is merged requires a re-spec.

| Check | Pass condition |
|---|---|
| 1.1 | Every symbol listed in `design.md` §4.1 is exported from `src/index.ts`. |
| 1.2 | Every TS signature matches `design.md` §4.2–§4.9 verbatim — function names, parameter names, optionality, return types. |
| 1.3 | Subpath exports in `package.json#exports` match `design.md` §4.1 subpath table. |
| 1.4 | `tests/_support/contractSnapshot.ts` typechecks against the v0.1.0 surface. |
| 1.5 | `reserve()` and `requestDecision` share an identical function reference (alias, not copy). |
| 1.6 | Naming convention enforced: **camelCase on the public surface; snake_case only in `src/_proto/`.** |
| 1.7 | No `default export` anywhere in `src/index.ts`. (Adapters import named symbols only.) |

If any of 1.1–1.7 fail → P0. Substrate is unsuitable for downstream consumption.

## 2. Cross-language determinism (P0 — blocker)

| Check | Pass condition |
|---|---|
| 2.1 | `computePromptHash(text, tenant)` byte-for-byte equals Python `spendguard.prompt_hash.compute` for every fixture entry. |
| 2.2 | `deriveIdempotencyKey({...})` byte-for-byte equals Python `derive_idempotency_key(**{...})` for every fixture entry. |
| 2.3 | `defaultCallSignature(messages, settings)` matches Python `default_call_signature(messages, model_settings)` for the canonicalised messages in the fixture. |
| 2.4 | UUID-shaped tenants are lowercased; non-UUID tenants are used verbatim. |
| 2.5 | The shared fixture file `sdk/fixtures/cross-language/v1.json` is committed; the Python + Rust suites consume it; the TS suite consumes it. |
| 2.6 | `pnpm run test tests/crossLanguage.test.ts` runs ≥ 20 vectors and they all pass. (Amended 2026-06-07 in SLICE 9 R1 spec-hygiene followup: the original ≥ 64 floor assumed a 3-function corpus including `defaultCallSignature`; per design.md §11 line 502 that signature is "slightly relaxed; uses its own framework's message types" and has no TS impl yet. SLICE 9 ships 20 fixtures across `derive_idempotency_key` (8), `compute_prompt_hash` (8), `derive_uuid_from_signature` (4). v1.json is appendable per the README runbook; the floor will rise organically as new fixtures land.) |
| 2.7 | When a fixture vector fails, the test prints a clear diff (expected vs actual) — not just `false === true`. |

Drift here breaks audit-chain rule dedup across the Python + Rust + TS estate. This is THE invariant. P0 — blocker.

## 3. ESM-only + tree-shakeability (P1)

| Check | Pass condition |
|---|---|
| 3.1 | `"type": "module"` in `package.json`. |
| 3.2 | No CJS build artefact in `dist/`. |
| 3.3 | `"sideEffects": false` in `package.json`. |
| 3.4 | `tests/treeShaking.test.ts` confirms `import { newUuid7 } from "@spendguard/sdk/ids"` does NOT pull `@grpc/grpc-js`. |
| 3.5 | tsup config produces ESM-only output (`format: ["esm"]`). |
| 3.6 | Each subpath has a `.js` + `.d.ts` pair in `dist/`. |

## 4. Bundle-size budget (P1)

| Check | Pass condition |
|---|---|
| 4.1 | `pnpm run size` script exists and is wired into `prepack`. |
| 4.2 | `dist/index.js` minified ≤ 120 KB. |
| 4.3 | `dist/index.js` gzipped ≤ 35 KB. |
| 4.4 | Generated proto tree ≤ 250 KB unminified. |
| 4.5 | A budget breach is a build failure, not a warning. |

## 5. Error hierarchy parity with Python (P1)

| Check | Pass condition |
|---|---|
| 5.1 | Every Python error class in `errors.py` has a TS counterpart in `src/errors.ts`. |
| 5.2 | Class names use the **TS convention with PascalCase** but otherwise mirror Python (e.g. Python `DecisionStopped` → TS `DecisionStopped`, Python `ApprovalDeniedError` → TS `ApprovalDeniedError`). |
| 5.3 | `SidecarUnavailable.statusCode === 503`; `DecisionDenied.statusCode === 403`. Both are `as const` literal numbers, not `number`. |
| 5.4 | `instanceof` chains are correct: `ApprovalRequired instanceof DecisionDenied instanceof SpendGuardError instanceof Error`. |
| 5.5 | `ApprovalRequired.resume(client)` delegates to `client.resumeAfterApproval` and returns its result. |
| 5.6 | Each error class preserves its `name` field across `JSON.stringify` and `Error.captureStackTrace`. |
| 5.7 | `cause` is forwarded when the constructor receives it (`new SidecarUnavailable("...", { cause: err })`). |

## 6. UDS transport correctness (P1)

| Check | Pass condition |
|---|---|
| 6.1 | `unix:` URI scheme used to connect via `@grpc/grpc-js`. |
| 6.2 | `grpc.default_authority` channel option set to `"localhost"` (per Python SDK precedent at `client.py:240-251`). |
| 6.3 | Channel options expose `grpc.max_receive_message_length` ≥ 4 MiB (covers v1 message ceiling). |
| 6.4 | Channel close on `close()` is idempotent and does not throw on double-close. |
| 6.5 | `[Symbol.asyncDispose]` is implemented and tested. |

## 7. Idempotency + retry safety (P1)

| Check | Pass condition |
|---|---|
| 7.1 | Retry policy is bounded to 2 attempts with 25 ms + jitter [0..25 ms]. |
| 7.2 | Only `UNAVAILABLE`, `DEADLINE_EXCEEDED`, `CANCELLED` are retried; `INVALID_ARGUMENT` / `FAILED_PRECONDITION` / `PERMISSION_DENIED` are not. |
| 7.3 | A retry requires an `idempotencyKey` on the originating request; without one, retry is a no-op even on retriable codes. |
| 7.4 | The in-process `DecisionCache` collapses same-`idempotencyKey` `reserve()` calls. |
| 7.5 | Cache is per-client (different `SpendGuardClient` instances do not share). |
| 7.6 | `release()` is idempotent — same `(reservationId, idempotencyKey)` returns the original outcome. |

## 8. Run plan (Signal 3) correctness (P2)

| Check | Pass condition |
|---|---|
| 8.1 | `withRunPlan` uses `node:async_hooks` `AsyncLocalStorage`. |
| 8.2 | Nested usage: outer wins (per `run_plan.py` spec). |
| 8.3 | Plan visible across `await`, `Promise.all`, `setImmediate`, `process.nextTick`. |
| 8.4 | `plannedCalls` / `plannedTools` validated at decorator time (non-int / negative → `TypeError`). |
| 8.5 | `reserve()` sends `plannedStepsHint = plannedCalls + plannedTools` when a plan is active; `0` otherwise. |
| 8.6 | `currentRunPlan()` returns `null` when no plan is active. |

## 9. OTel hook semantics (P2)

| Check | Pass condition |
|---|---|
| 9.1 | `otelTracer` and `onSpan` are mutually exclusive — both set → `SpendGuardConfigError`. |
| 9.2 | When neither is set, the client emits zero spans (no allocation, no work). |
| 9.3 | When `otelTracer` is set, spans use names `spendguard.<rpc>` with the documented attribute set (`design.md` §6.4). |
| 9.4 | `@opentelemetry/api` is a `peerDependencyMeta.optional: true`; importing without it installed must not break the build. |
| 9.5 | OTel span errors are recorded with `recordException` + `setStatus({ code: ERROR })`. |

## 10. Proto codegen pipeline (P2)

| Check | Pass condition |
|---|---|
| 10.1 | `scripts/proto.ts` is the single source of truth; running it twice produces a bit-identical tree. |
| 10.2 | The generated tree under `src/_proto/` is committed (not gitignored). |
| 10.3 | CI gate `pnpm run proto && git diff --exit-code src/_proto` is wired in the publish workflow. |
| 10.4 | The script reads from `proto/spendguard/` only — no other proto roots. |
| 10.5 | Generated files include both `.ts` and the corresponding service stub types. |
| 10.6 | Proto field names that begin with a TypeScript reserved word (`new`, `default`, …) are escaped, not renamed. |

## 11. Documentation completeness (P2)

| Check | Pass condition |
|---|---|
| 11.1 | `sdk/typescript/README.md` includes a 30-line working `reserve → commitEstimated → release` snippet. |
| 11.2 | Every public method on `SpendGuardClient` has JSDoc with a `@throws` block enumerating typed exceptions. |
| 11.3 | The JSDoc on `queryBudget()` documents that it throws "not yet wired" in v0.1.x. |
| 11.4 | `SPENDGUARD_DISABLE` env var is documented as **test-only** in JSDoc on the constructor — a forgotten production setting must not silently lose enforcement. |
| 11.5 | `CHANGELOG.md` 0.1.0 entry calls out: "first public release; mirrors `spendguard-sdk` (Python) v0.5.1". |
| 11.6 | `LICENSE_NOTICES.md` lists every dep license; specifically `@grpc/grpc-js` (Apache-2.0), `@protobuf-ts/runtime` (Apache-2.0), `@noble/hashes` (MIT). |
| 11.7 | `docs/site/docs/integrations/typescript-sdk.md` exists. |

## 12. Publish pipeline (P1)

| Check | Pass condition |
|---|---|
| 12.1 | `.github/workflows/sdk-ts-publish.yml` exists. |
| 12.2 | Triggered on `release` event + `workflow_dispatch`. |
| 12.3 | `if: startsWith(github.ref, 'refs/tags/ts-sdk-v')` gates the publish job. |
| 12.4 | `permissions: id-token: write` is set on the publish job for OIDC. |
| 12.5 | `npm publish --provenance --access public` is the publish command. |
| 12.6 | The workflow runs `proto`, `lint`, `typecheck`, `test`, `build`, `size` before `publish`. |
| 12.7 | Lockfile-frozen install (`pnpm install --frozen-lockfile`). |
| 12.8 | No secrets in `pyproject`-equivalent; OIDC trusted publisher only (the `NODE_AUTH_TOKEN` fallback is documented as a temporary measure during the npm Trusted Publisher rollout window). |

## 13. Security checklist (P1)

| Check | Pass condition |
|---|---|
| 13.1 | No `eval`, `new Function`, or `Function.prototype.constructor` anywhere in `src/`. |
| 13.2 | No string-interpolation into shell commands in `scripts/`. |
| 13.3 | Crypto primitives use `node:crypto` or `@noble/hashes` only — no roll-your-own HMAC. |
| 13.4 | `computePromptHash` never logs the prompt text (only the hash). |
| 13.5 | `decisionContextJson` is treated as opaque (forwarded verbatim, never schemaful-merged into other adapter state). |
| 13.6 | `release()` does not log the `idempotencyKey` at INFO level (key is opaque but in dev tools it can leak request IDs). |
| 13.7 | `npm audit --omit=dev` reports 0 high/critical advisories at publish time. |

## 14. Performance gates (P2)

| Check | Pass condition |
|---|---|
| 14.1 | A `reserve()` call against the in-memory mock sidecar completes in ≤ 5 ms p95 on Node 22 LTS. |
| 14.2 | Cold `connect() + handshake()` completes in ≤ 50 ms on the mock sidecar. |
| 14.3 | The `withRunPlan` overhead (vs no plan) is ≤ 1 µs per call (benchmark in `tests/runPlan.test.ts`). |
| 14.4 | The pure-TS modules (ids, errors, pricing, promptHash, runPlan) load cold in ≤ 25 ms on Node 22 LTS. |

## 15. Slice-specific checks

Each slice's R1 review additionally verifies the slice doc's anti-scope is honored:

| Slice | Anti-scope check |
|---|---|
| `COV_S05_01_d05_package_init` | No source files under `src/` yet beyond placeholder index. No tests beyond a sanity import. |
| `COV_S05_02_d05_proto_codegen` | No client logic in this slice — only the codegen script + generated tree. |
| `COV_S05_03_d05_client_skeleton` | RPCs are stubbed (throw "see slice S05_04") if not yet implemented; no half-implementation. |
| `COV_S05_04_d05_handshake_reserve_commit` | No `release()` or `queryBudget()` work in this slice. |
| `COV_S05_05_d05_release_query` | No handshake / reserve changes; only release + queryBudget. |
| `COV_S05_06_d05_ids_prompt_hash_pricing` | No client changes; pure utility modules. |
| `COV_S05_07_d05_run_plan` | No client changes beyond reading `currentRunPlan()` in `buildDecisionRequest`. |
| `COV_S05_08_d05_otel_retry_idempotency` | No new RPC surface — only behavior wrappers. |
| `COV_S05_09_d05_test_matrix` | No src/ changes (except a possible `src/_internal/testing.ts` for the mock sidecar's shared types). |
| `COV_S05_10_d05_publish_pipeline` | No src/ changes; only `.github/`, `README.md`, `CHANGELOG.md`, `LICENSE_NOTICES.md`. |

## 16. Findings categorisation

| Category | Definition | R1 action |
|---|---|---|
| **P0** | Public-surface drift, cross-language drift, security finding, sidecar-incompatible wire change. | Block. Fix before re-run. |
| **P1** | Spec gate failure, missing test, missing documentation, wrong error class, retry / idempotency bug. | Block. Fix before re-run. |
| **P2** | Stylistic, minor JSDoc gap, non-critical perf, polish. | Track as residual; may merge with note. |
| **Polish** | Naming preferences, comment wording, etc. | Track as residual; do not block. |

## 17. R1-R5 escalation rules

- Same finding category appearing in **two consecutive rounds** without progress → escalate to Staff+ panel arbitration per build-plan §1.3.
- A P0 finding open at R5 → automatic Staff+ panel arbitration.
- A residual P2 / Polish that the reviewer agreed to defer is filed as a `gh issue` referenced from the slice doc.

## 18. Residual triage template

When a finding cannot be fixed in-slice and is deferred to a follow-up, the reviewer creates a GitHub issue with the template:

```
Title: [D05 residual] <one-line summary>

Body:
- Slice: COV_S05_<NN>_<short>
- Round: R<n>
- Category: P<0|1|2>|Polish
- Spec ref: design.md §<n>, tests.md §<n>, acceptance.md §<n>
- Repro: <minimal command sequence>
- Why deferred: <one line>
- Suggested follow-up slice: <name or "TBD post-D05">
```

Residuals are tracked alongside the existing `#85`-`#177` pattern; the issue list at D05 completion time is added to the memory file per build-plan §8.

## 19. Sign-off

The reviewer signs off only when:
- Every P0 + P1 in §1–§14 is green.
- Slice-specific anti-scope in §15 is honored.
- All R<=5 findings are either resolved or filed as residuals.
- The acceptance gates in `acceptance.md` §10 are green.

If any of those fail → slice does not pass R review → loop continues.
