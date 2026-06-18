// Cross-component contract test for the FAILED_PRECONDITION reason-code
// dispatch (D05 SLICE 5 hardening).
//
// The SDK's `readReasonCode` resolves IDEMPOTENCY_CONFLICT / BUDGET_EXCEEDED /
// BUNDLE_HOT_RELOADED by case-folded PREFIX-matching the gRPC Status message
// against `REASON_CODE_PREFIXES`. The production sidecar bakes the
// discriminator into the Status message string (it does NOT set the
// `x-spendguard-reason-code` trailer), so that string IS the contract.
//
// Risk this guards: a wording change to the Rust `DomainError::Display`
// (`#[error("...")]`) prefixes — e.g. reordering "idempotency conflict:" to
// "conflict (idempotency):" — would silently demote every conflict to the
// generic `MutationApplyFailed` default in production, breaking adapters that
// route on the specific subclass. There is no compile-time link between the
// Rust strings and the TS table, so we pin them here: this test fails CI when
// the TS prefixes drift from the Rust Display format documented inline.
//
// SOURCE OF TRUTH (kept in sync by this test):
//   - services/sidecar/src/domain/error.rs  `#[error("...")]` on the
//     FAILED_PRECONDITION cluster variants (the `: {0}` detail tail is
//     stripped — the prefix is anchored at the start, case-insensitive).
//   - services/sidecar/src/server/adapter_uds.rs:1423  `[BUNDLE_HOT_RELOADED]`
//     bracket prefix on the approval-resume path.
//
// If the Rust strings legitimately change, update BOTH the Rust source AND the
// `EXPECTED` table below in the same change.

import { describe, expect, it } from "vitest";

import { REASON_CODE_PREFIXES } from "../src/client.js";

// The exact Rust `DomainError::Display` prefixes (lowercased, detail tail
// dropped) that MUST map to each canonical reason code. Verified against
// services/sidecar/src/domain/error.rs and adapter_uds.rs:1423.
const EXPECTED: ReadonlyArray<readonly [string, string]> = [
  ["[bundle_hot_reloaded]", "BUNDLE_HOT_RELOADED"],
  ["bundle hot-reload", "BUNDLE_HOT_RELOADED"],
  ["idempotency conflict", "IDEMPOTENCY_CONFLICT"], // #[error("idempotency conflict: {0}")]
  ["reservation state conflict", "BUDGET_EXCEEDED"], // #[error("reservation state conflict: {0}")]
  ["reservation ttl expired", "BUDGET_EXCEEDED"], // #[error("reservation TTL expired: {0}")]
  ["pricing freeze mismatch", "BUDGET_EXCEEDED"], // #[error("pricing freeze mismatch: {0}")]
  ["overrun reservation", "BUDGET_EXCEEDED"], // #[error("overrun reservation: {0}")]
  ["multi-reservation commit deferred", "BUDGET_EXCEEDED"], // #[error("multi-reservation commit deferred: {0}")]
];

describe("FAILED_PRECONDITION reason-code prefix contract", () => {
  it("REASON_CODE_PREFIXES is byte-aligned with the Rust DomainError::Display prefixes", () => {
    // Exact list + ordering equality. Ordering matters: longer / more-specific
    // prefixes must come first so a shorter prefix cannot swallow a longer one.
    expect(REASON_CODE_PREFIXES.map((p) => [...p])).toEqual(EXPECTED.map((p) => [...p]));
  });

  it("every prefix is lowercase (readReasonCode case-folds the candidate before matching)", () => {
    for (const [prefix] of REASON_CODE_PREFIXES) {
      expect(prefix).toBe(prefix.toLowerCase());
    }
  });

  it("each canonical reason code is reachable from at least one prefix", () => {
    const codes = new Set(REASON_CODE_PREFIXES.map(([, code]) => code));
    expect(codes.has("IDEMPOTENCY_CONFLICT")).toBe(true);
    expect(codes.has("BUDGET_EXCEEDED")).toBe(true);
    expect(codes.has("BUNDLE_HOT_RELOADED")).toBe(true);
  });

  it("a representative Rust Display message resolves to the right code via prefix match", () => {
    // Simulates what the production sidecar emits: the Display prefix followed
    // by a per-call `: <detail>` tail. We assert prefix-startsWith semantics
    // (the same operation readReasonCode performs) for each cluster.
    const samples: ReadonlyArray<readonly [string, string]> = [
      [
        "idempotency conflict: key already committed for a different effect",
        "IDEMPOTENCY_CONFLICT",
      ],
      ["reservation state conflict: reservation already released", "BUDGET_EXCEEDED"],
      ["reservation TTL expired: reservation 9f.. expired at ..", "BUDGET_EXCEEDED"],
      ["pricing freeze mismatch: bundle pricing_version drifted", "BUDGET_EXCEEDED"],
      ["overrun reservation: committed amount exceeds reserved", "BUDGET_EXCEEDED"],
      ["multi-reservation commit deferred: 2 of 3 reservations pending", "BUDGET_EXCEEDED"],
      [
        "[BUNDLE_HOT_RELOADED] approval was issued under bundle hash A but ..",
        "BUNDLE_HOT_RELOADED",
      ],
    ];
    for (const [message, expectedCode] of samples) {
      const lower = message.toLowerCase();
      const match = REASON_CODE_PREFIXES.find(([prefix]) => lower.startsWith(prefix));
      expect(match, `no prefix matched ${JSON.stringify(message)}`).toBeDefined();
      expect(match?.[1]).toBe(expectedCode);
    }
  });
});
