// SpendGuard SDK — optional OTel span hook.
//
// Power-user opt-in: a caller passes an `@opentelemetry/api` `Tracer` via
// `SpendGuardClientConfig.otelTracer`. The client then wraps every RPC in a
// `spendguard.<rpc>` span (per design.md §6.4). When the tracer is unset,
// `withOtelSpan` short-circuits to `await fn()` with ZERO span overhead and
// ZERO `@opentelemetry/api` dep resolution at runtime (the dep is
// `peerDependenciesMeta.optional: true` per package.json + design §6.4 line
// 422 + locked decision #7).
//
// ── Spec lineage (LOCKED) ──────────────────────────────────────────────────
//
//   - design.md §6.4 lines 409-422 (span name + attribute table).
//   - implementation.md §12 (one-line skeleton: "Span wrapper").
//   - review-standards.md §1.5 P0 alias identity (preserves `requestDecision
//     === reserve` — we wrap from the OUTSIDE so the underlying method
//     reference is untouched).
//   - sdk/python/src/spendguard/client.py (no Python OTel parity yet — TS
//     leads here; Python catches up in a future slice).
//
// ── Why `peerDependenciesMeta.optional: true` ──────────────────────────────
//
// `@opentelemetry/api` is a 25 KB dep with its own transitive
// `@opentelemetry/context-async-hooks` chain. The vast majority of adapters
// never need OTel (`onSpan` covers their use case). Pinning OTel as a peer
// optional dep means:
//   1. Adapters that DO use OTel get a sane version-range warning if their
//      OTel version is incompatible.
//   2. Adapters that DON'T use OTel pay zero install + zero bundle cost.
//   3. The `Tracer` type is `import type`'d — TypeScript erases the import at
//      build time, so even type-only users don't drag the runtime in.
//
// The `cfg.otelTracer` field is typed via `import type { Tracer } from
// "@opentelemetry/api"` in `config.ts`; this file mirrors that pattern.

import type { Span, SpanStatusCode, Tracer } from "@opentelemetry/api";

/**
 * Frozen set of OTel attribute keys that `withOtelSpan` honors. The keys
 * match the table in design.md §6.4 verbatim — adapters that change them
 * break the span-attribute contract that observability dashboards consume.
 *
 * The values are intentionally lowercase + dotted to match OpenTelemetry
 * semantic-convention style (`<vendor>.<subsystem>.<field>`); they are
 * NOT prefixed with `attr_` or similar.
 */
export const SPENDGUARD_OTEL_ATTR = {
  TENANT_ID: "spendguard.tenant_id",
  DECISION_ID: "spendguard.decision_id",
  TRIGGER: "spendguard.trigger",
  OUTCOME_DECISION: "spendguard.outcome.decision",
  OUTCOME_REASON_CODES: "spendguard.outcome.reason_codes",
  SDK_VERSION: "spendguard.sdk.version",
  RESERVATION_ID: "spendguard.reservation_id",
  SCOPE_ID: "spendguard.scope_id",
} as const;

/**
 * Lazy SpanStatusCode shim: the OTel enum value `2` corresponds to ERROR
 * (and `1` to OK / `0` to UNSET), per the
 * `@opentelemetry/api/build/src/trace/status.d.ts` definition. Encoding the
 * raw integer here avoids importing the runtime value (which would force the
 * peer-optional dep into the runtime path even when no tracer is set).
 *
 * SLICE 9 adds a true `SpanStatusCode.ERROR` import once we have an integration
 * test that requires the runtime symbol — for v0.1.x, the integer value is the
 * stable OTel API contract.
 */
const SPAN_STATUS_ERROR = 2 satisfies SpanStatusCode;

/**
 * Attribute primitive set OTel accepts on `Span.setAttribute`.
 *
 * Mirrors `SpanAttributeValue` from `@opentelemetry/api` without importing
 * the runtime type (the type-only import path is already covered by the
 * function signature).
 */
export type OtelAttributeValue = string | number | boolean | string[] | number[] | boolean[];

/**
 * Span attributes for the `withOtelSpan` wrapper. Keys SHOULD use the
 * `spendguard.*` prefix per design.md §6.4 attribute table; values are
 * standard OTel attribute primitives.
 */
export type OtelAttributes = Readonly<Record<string, OtelAttributeValue | undefined>>;

/**
 * Wrap `fn()` in an OTel span named `spendguard.<rpcName>`. When `tracer`
 * is `undefined`, returns `await fn()` directly — no span, no attribute
 * encoding, no allocation overhead.
 *
 * Per design.md §6.4 (line 422), `@opentelemetry/api` is a
 * `peerDependenciesMeta.optional` dep; the type-only import above is erased
 * at build time so callers that never enable OTel pay zero install/runtime
 * cost.
 *
 * Span lifecycle:
 *   - `tracer.startSpan(name, { attributes })` opens the span.
 *   - Exceptions from `fn()` are recorded via `span.recordException(err)` +
 *     `span.setStatus({ code: ERROR, message })`, then re-thrown — the
 *     RPC failure is observable, the original throw semantics are preserved.
 *   - `span.end()` is always invoked from the `finally` block so a thrown
 *     exception does NOT leak the span (a leaked span would never appear
 *     in the trace export; adapters debugging missing spans waste hours).
 *
 * Span attribute encoding:
 *   - Undefined-valued attributes are skipped (no `null` propagation onto the
 *     span — OTel treats missing keys identically and omitting them keeps the
 *     wire payload tighter).
 *
 * @param tracer Optional OTel `Tracer`. When `undefined`, `fn()` runs
 *   unwrapped. When defined, MUST be an `@opentelemetry/api` v1.9+ tracer.
 * @param rpcName Bare RPC name; the span name is `spendguard.<rpcName>`.
 *   Caller MUST NOT prefix with `spendguard.` themselves (this function
 *   does it).
 * @param attributes Span attributes to set at start-time. Undefined values
 *   are filtered out.
 * @param fn The RPC implementation. Its return value is the function's
 *   return value; its throws are recorded and re-thrown.
 *
 * @returns The result of `fn()`. Rethrows whatever `fn()` throws.
 *
 * @example
 *   await withOtelSpan(cfg.otelTracer, "reserve", {
 *     [SPENDGUARD_OTEL_ATTR.TENANT_ID]: cfg.tenantId,
 *     [SPENDGUARD_OTEL_ATTR.DECISION_ID]: req.decisionId,
 *   }, async () => {
 *     return await client.requestDecision(...);
 *   });
 */
export async function withOtelSpan<T>(
  tracer: Tracer | undefined,
  rpcName: string,
  attributes: OtelAttributes,
  fn: () => Promise<T>,
): Promise<T> {
  if (tracer === undefined) return await fn();

  const spanName = `spendguard.${rpcName}`;
  // Filter undefined attribute values — OTel's `Attributes` map treats
  // `undefined` as the absent-key marker, not as a stored null.
  const filtered: Record<string, OtelAttributeValue> = {};
  for (const [k, v] of Object.entries(attributes)) {
    if (v !== undefined) filtered[k] = v;
  }
  const span: Span = tracer.startSpan(spanName, { attributes: filtered });
  try {
    const result = await fn();
    return result;
  } catch (err) {
    // recordException accepts unknown / Error / string; cast to satisfy the
    // overload set. We preserve the original throw — never silently swallow.
    if (err instanceof Error) {
      span.recordException(err);
      span.setStatus({ code: SPAN_STATUS_ERROR, message: err.message });
    } else {
      const message = typeof err === "string" ? err : String(err);
      span.recordException({ name: "SpendGuardError", message });
      span.setStatus({ code: SPAN_STATUS_ERROR, message });
    }
    throw err;
  } finally {
    span.end();
  }
}

/**
 * Set additional attributes on the active span after start-time. Used for
 * outcome-side attributes (`spendguard.outcome.decision` /
 * `spendguard.outcome.reason_codes`) that are only known after the RPC
 * returns. Silently no-ops when `tracer` is undefined.
 *
 * The active span lookup is intentionally NOT done via `trace.getActiveSpan()`
 * — that requires importing the OTel runtime, defeating the peer-optional
 * dep cost guarantee. Instead, callers thread the span explicitly via the
 * `withOtelSpan` callback (the span is in scope inside the callback closure).
 *
 * SLICE 9 may add a thread-local span variant if the egress proxy / projector
 * surface needs it; for v0.1.x, `withOtelSpan` is the only surface.
 *
 * @param span The active span. When `undefined`, this function is a no-op.
 * @param attributes Attributes to add. Undefined values are skipped.
 */
export function setOtelSpanAttributes(span: Span | undefined, attributes: OtelAttributes): void {
  if (span === undefined) return;
  for (const [k, v] of Object.entries(attributes)) {
    if (v !== undefined) span.setAttribute(k, v);
  }
}
