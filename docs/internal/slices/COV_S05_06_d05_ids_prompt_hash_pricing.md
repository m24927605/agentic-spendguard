# COV_S05_06 — D05 TS SDK substrate: ids + promptHash + pricing

> **Deliverable**: D05 TS SDK substrate
> **Slice**: 6 of 10 (M)
> **Spec set**: [`docs/specs/coverage/D05_ts_sdk_substrate/`](../../specs/coverage/D05_ts_sdk_substrate/)

## Scope

Lands the 3 helper modules that adapters import:
1. `ids.ts` — `newUuid7()` + `deriveIdempotencyKey()` (mirrors Python `spendguard.ids`)
2. `promptHash.ts` — HMAC-SHA256 tenant-keyed prompt hash (mirrors Python `spendguard.prompt_hash::compute`)
3. `pricing.ts` — USD-micros computation + `PricingLookup` interface (mirrors Python `spendguard.pricing`)

Plus the embedded demo pricing snapshot subpath: `@spendguard/sdk/pricing/demo` → `DEMO_PRICING` constant (the YAML content of `deploy/demo/init/pricing/seed.yaml` materialized as a TS literal). Bundle budget: snapshot < 50 KB; total bundle still ≤ 120 KB after SLICE 5's 119.40 KB baseline (deviation #1 — SLICE 6 likely exceeds; mitigation via subpath export so adapters tree-shake).

Closes the M-3 P2 residual from SLICE 4 R1: `req.promptText` no longer silently discarded — `buildRuntimeMetadataStruct` now calls `computePromptHash(req.promptText, this.cfg.tenantId)` after the decisionContextJson loop and writes `runtime_metadata.prompt_hash` per implementation.md §4 line 720.

Concretely:

### 1. `sdk/typescript/src/ids.ts` — NEW
```ts
export function newUuid7(): string {
  // RFC 9562 §5.7 UUIDv7 implementation; time-ordered + 80 bits randomness
}

export function deriveIdempotencyKey(
  tenantId: string,
  runId: string,
  stepId: string,
  llmCallId: string,
  decisionId: string,
): string {
  // SHA-256 fold of tenant_id + run_id + step_id + llm_call_id + decision_id
  // Mirrors Python spendguard.ids.derive_idempotency_key
}
```

### 2. `sdk/typescript/src/promptHash.ts` — NEW
```ts
export function computePromptHash(promptText: string, tenantId: string): string {
  // HMAC-SHA256(key=tenantId, data=promptText), hex-encoded
  // Mirrors Python spendguard.prompt_hash.compute
  // MUST produce byte-identical output to Python implementation for the same inputs
}
```

### 3. `sdk/typescript/src/pricing.ts` — NEW
```ts
export interface PricingLookup {
  lookup(model: string, providerHint?: string): PricingEntry | undefined;
}

export interface PricingEntry {
  inputTokensPerUsdMicros: bigint;  // NUMERIC(38,0) as bigint
  outputTokensPerUsdMicros: bigint;
  pricingVersion: string;
}

export function computeUsdMicros(
  inputTokens: bigint,
  outputTokens: bigint,
  entry: PricingEntry,
): bigint {
  // (input_tokens / input_per_usd_micros) + (output_tokens / output_per_usd_micros)
  // Mirrors Python spendguard.pricing.compute_usd_micros
}

export class StaticPricingLookup implements PricingLookup {
  constructor(private entries: Map<string, PricingEntry>) {}
  lookup(model: string, providerHint?: string): PricingEntry | undefined {
    return this.entries.get(model);
  }
}
```

### 4. `sdk/typescript/src/pricing/demo.ts` — NEW
- Embed the content of `deploy/demo/init/pricing/seed.yaml` as a TS literal
- Export `DEMO_PRICING: StaticPricingLookup` ready to use
- Size budget: < 50 KB

### 5. `sdk/typescript/src/client.ts` — modify `buildRuntimeMetadataStruct`
- Closes SLICE 4 R1 M-3 residual: when `req.promptText` is set, call `computePromptHash(req.promptText, this.cfg.tenantId)` and add `runtime_metadata.prompt_hash` to the Struct (as a stringValue).
- Add `import { computePromptHash } from "./promptHash.js"` (or wherever the ESM path resolves).

### 6. `sdk/typescript/src/index.ts` — barrel updates
- Re-export `newUuid7`, `deriveIdempotencyKey` from `./ids.js`
- Re-export `computePromptHash` from `./promptHash.js`
- Re-export `PricingLookup`, `PricingEntry`, `computeUsdMicros`, `StaticPricingLookup` from `./pricing.js`
- Do NOT re-export `DEMO_PRICING` from the main index (subpath export keeps bundle lean)

### 7. `sdk/typescript/package.json` — subpath exports
- Add `"./pricing/demo": "./dist/pricing/demo.js"` to exports field
- Add corresponding "./ids", "./promptHash", "./pricing" subpath exports per design §3.1 (if not already)

### 8. Tests
- `sdk/typescript/tests/ids.test.ts` — NEW
  - UUIDv7 timestamp ordering: 100 generated UUIDs are time-monotonic to ms precision
  - UUIDv7 RFC 9562 §5.7 byte layout (version nibble = 7; variant bits = 10)
  - deriveIdempotencyKey determinism: same inputs → same output
  - deriveIdempotencyKey domain separation: different tenant_id → different output

- `sdk/typescript/tests/promptHash.test.ts` — NEW
  - HMAC-SHA256 output matches Python `spendguard.prompt_hash.compute()` for ≥3 known fixtures (provide test vectors via subprocess `python -c "..."` if available; or hard-code the known-good outputs)
  - Empty prompt: returns a deterministic hash
  - Long prompt: handles arbitrary length

- `sdk/typescript/tests/pricing.test.ts` — NEW
  - computeUsdMicros for OpenAI gpt-4o pricing yields expected micros for a 1000-input / 500-output call
  - StaticPricingLookup.lookup returns undefined for unknown model
  - PricingEntry bigint serialization round-trip

- `sdk/typescript/tests/pricingDemo.test.ts` — NEW
  - DEMO_PRICING from `@spendguard/sdk/pricing/demo` is importable
  - Snapshot covers ≥10 models (OpenAI + Anthropic + Gemini common ones)
  - Snapshot version matches `deploy/demo/init/pricing/seed.yaml` `pricing_version` field

- Extend `sdk/typescript/tests/handshake-reserve-commit.test.ts`:
  - New test: when `req.promptText` is set, `runtime_metadata.prompt_hash` appears in the wire-shape with the HMAC-SHA256 hex output (closes SLICE 4 R1 M-3)

- Extend `sdk/typescript/tests/locked-surface.test.ts`:
  - Assert `ids.ts`, `promptHash.ts`, `pricing.ts` exports are reachable from main barrel

### 9. Bundle size mitigation
- Bundle budget is ≤ 120 KB minified for the main `index.js`. SLICE 5 baseline = 119.40 KB.
- New code likely adds ~3-5 KB to main bundle if all symbols are barrel-re-exported. 
- Mitigation: keep `DEMO_PRICING` ONLY accessible via subpath (`./pricing/demo`) — never re-exported from main `index.ts`. Verify with build-size check.
- If main bundle exceeds 120 KB after SLICE 6 → restructure: move `pricing.ts` to subpath `./pricing` (not in main barrel), document migration in slice doc. Adapters import via `@spendguard/sdk/pricing` going forward.

## Files touched

| File | Why |
|------|-----|
| `sdk/typescript/src/ids.ts` | NEW |
| `sdk/typescript/src/promptHash.ts` | NEW |
| `sdk/typescript/src/pricing.ts` | NEW |
| `sdk/typescript/src/pricing/demo.ts` | NEW — embedded snapshot |
| `sdk/typescript/src/client.ts` | wire computePromptHash into buildRuntimeMetadataStruct (M-3 closure) |
| `sdk/typescript/src/index.ts` | barrel updates (ids, promptHash, pricing — NOT demo) |
| `sdk/typescript/package.json` | subpath exports field |
| `sdk/typescript/tests/ids.test.ts` | NEW |
| `sdk/typescript/tests/promptHash.test.ts` | NEW |
| `sdk/typescript/tests/pricing.test.ts` | NEW |
| `sdk/typescript/tests/pricingDemo.test.ts` | NEW |
| `sdk/typescript/tests/handshake-reserve-commit.test.ts` | M-3 promptText wire test |
| `sdk/typescript/tests/locked-surface.test.ts` | new exports surface check |

## Test/verification plan

1. `pnpm run typecheck` clean.
2. `pnpm run test` — 162 + ~22 new = ~184 passing.
3. `pnpm run build` clean.
4. `dist/index.js` minified ≤ 120 KB; report headroom.
5. `dist/pricing/demo.js` ≤ 50 KB.
6. Key new tests:
   - UUIDv7 layout + monotonicity
   - HMAC-SHA256 cross-language parity (Python fixture)
   - Pricing micros computation
   - Demo snapshot version assertion
   - promptText → prompt_hash wire mapping (SLICE 4 R1 M-3 closure)

## Anti-scope

- No `withRunPlan` — SLICE 7.
- No OTel / retry / idempotency cache — SLICE 8.
- No release dance / NPM publish — SLICE 10.
- No proto bump for LLM_CALL_OUTCOME — cross-component slice (SLICE 5 deviation #1 residual).

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D05_ts_sdk_substrate/design.md) §4.9 PricingLookup, §3 module layout, §8 slice 6 row, §13 bundle constraints
- SLICE 5 R1 residual M-3: promptText silent discard closes here
- SLICE 5: [`COV_S05_05_d05_release_query.md`](COV_S05_05_d05_release_query.md)
