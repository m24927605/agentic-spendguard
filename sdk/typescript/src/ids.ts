// SpendGuard SDK — ID helpers (SLICE 6 / COV_S05_06).
//
// Mirrors `sdk/python/src/spendguard/ids.py`. Two flavours of identity are
// minted here:
//
//   - Time-ordered IDs (`newUuid7`) for one-shot operations whose identity
//     does not need to survive retries (handshake `workload_instance_id`,
//     `RunContext.run_id` when the caller has not set one).
//   - **Content-derived** IDs for everything inside a single LLM call. A
//     retry of the SAME logical step within an agent framework's run loop
//     MUST reuse the same key so the sidecar's idempotency cache + the
//     ledger's `UNIQUE` constraint collapse onto the first decision.
//
// Spec refs:
//   - design.md §4.6 (LOCKED ID helper surface)
//   - implementation.md §6 (`src/ids.ts`)
//   - review-standards.md §1.5 (cross-language byte-equivalence)
//   - tests.md §5.3 (cross-language fixture matrix)
//
// Cross-language invariant: `deriveIdempotencyKey({...})` MUST produce the
// same string as Python `spendguard.ids.derive_idempotency_key(**kwargs)`
// for any byte-identical input. Same for `deriveUuidFromSignature`. Tested
// against shared fixtures in `tests/ids.test.ts`.

import { randomBytes } from "node:crypto";
import { blake2b } from "@noble/hashes/blake2";

/**
 * Mint a UUIDv7 per RFC 9562 §5.7.
 *
 * Layout (128 bits, big-endian):
 *   - 48 bits unix epoch ms
 *   - 4 bits version (0b0111)
 *   - 12 bits random
 *   - 2 bits variant (0b10)
 *   - 62 bits random
 *
 * Two calls within the same ms are time-ordered to ms precision but
 * randomised within the ms slot. Returned in canonical 36-char hex form
 * (`xxxxxxxx-xxxx-7xxx-yxxx-xxxxxxxxxxxx`).
 */
export function newUuid7(): string {
  const tsMs = BigInt(Date.now()) & ((1n << 48n) - 1n);
  // 12 bits of rand_a
  const randA = randomBytes(2).readUInt16BE(0) & 0x0fff;
  // 62 bits of rand_b + 2-bit variant overwrite
  const randB = randomBytes(8);
  // Overwrite top 2 bits of byte 8 with variant 0b10
  randB[0] = (randB[0]! & 0x3f) | 0x80;

  // bits 127..80  ts_ms (48)
  // bits 79..76   version 0x7
  // bits 75..64   rand_a (12)
  // bits 63..62   variant 0b10 (already set in randB)
  // bits 61..0    rand_b
  const hi = (tsMs << 16n) | BigInt(randA);
  const buf = Buffer.alloc(16);
  buf.writeBigUInt64BE(hi, 0);
  // Overwrite top nibble of byte 6 with version 7
  buf[6] = (buf[6]! & 0x0f) | 0x70;
  randB.copy(buf, 8);

  return [
    buf.toString("hex", 0, 4),
    buf.toString("hex", 4, 6),
    buf.toString("hex", 6, 8),
    buf.toString("hex", 8, 10),
    buf.toString("hex", 10, 16),
  ].join("-");
}

/**
 * Fixed canonical-form separator.
 *
 * Mirrors Python `derive_idempotency_key`: ASCII Unit Separator (\x1f) joins
 * the fields. The leading `"v1"` schema tag is included.
 */
const FIELD_SEP = "\x1f";

/**
 * Deterministic idempotency key for a trigger boundary.
 *
 * Same `(tenantId, sessionId, runId, stepId, llmCallId, trigger)` →
 * same key. A retry of the SAME logical step within an agent framework's
 * run loop MUST reuse this so the sidecar's cache short-circuits + the
 * ledger's `UNIQUE` returns Replay.
 *
 * Returns `"sg-"` + 32 hex chars (128-bit BLAKE2b digest).
 *
 * **Cross-language gate (P0 — review-standards §2.2)**: the TS output for
 * any given input is byte-identical to Python
 * `derive_idempotency_key(**kwargs)` (which uses `hashlib.blake2b(...,
 * digest_size=16)`). The implementation here calls the audited
 * `@noble/hashes` BLAKE2b primitive with `dkLen: 16` — the same 128-bit
 * personalised-output mode Python's hashlib exposes. Verified byte-for-byte
 * against `derive_idempotency_key(**kwargs)` for 7 fixtures in
 * `tests/ids.test.ts` (FX1–FX7).
 */
export function deriveIdempotencyKey(args: {
  tenantId: string;
  sessionId: string;
  runId: string;
  stepId: string;
  llmCallId: string;
  trigger: string;
}): string {
  const canonical = [
    "v1",
    args.tenantId,
    args.sessionId,
    args.runId,
    args.stepId,
    args.llmCallId,
    args.trigger,
  ].join(FIELD_SEP);
  // BLAKE2b-128 (16-byte digest) — byte-equivalent to Python
  // `hashlib.blake2b(..., digest_size=16).hexdigest()`. Locked by
  // review-standards §2.2 cross-language P0 gate.
  const digest = blake2b(new TextEncoder().encode(canonical), { dkLen: 16 });
  let hex = "";
  for (let i = 0; i < digest.length; i++) {
    hex += digest[i]!.toString(16).padStart(2, "0");
  }
  return `sg-${hex}`;
}

/**
 * Derive a stable UUID (v4-shaped) from a content signature + scope namespace.
 *
 * Used for content-derived `decision_id` / `llm_call_id` slots where a retry
 * of the SAME `Model.request()` MUST produce the same UUID across retries.
 * Scope ("decision_id", "llm_call_id", etc.) namespaces the output so
 * different identifier slots never collide for the same signature.
 *
 * Same `(signature, scope)` → same UUID. Byte-equivalent to Python
 * `spendguard.ids.derive_uuid_from_signature(signature, scope=scope)`:
 * BLAKE2b-128 over `f"{scope}|{signature}"`, with RFC 4122 v4 version (0x4)
 * and variant (0b10) nibbles overlaid on bytes 6 + 8. Verified against
 * 5 fixtures in `tests/ids.test.ts` (FXU1–FXU5).
 */
export function deriveUuidFromSignature(signature: string, args: { scope: string }): string {
  const canonical = `${args.scope}|${signature}`;
  const digest = blake2b(new TextEncoder().encode(canonical), { dkLen: 16 });
  const buf = Buffer.from(digest);
  // Version 4 + variant 10
  buf[6] = (buf[6]! & 0x0f) | 0x40;
  buf[8] = (buf[8]! & 0x3f) | 0x80;
  return [
    buf.toString("hex", 0, 4),
    buf.toString("hex", 4, 6),
    buf.toString("hex", 6, 8),
    buf.toString("hex", 8, 10),
    buf.toString("hex", 10, 16),
  ].join("-");
}

/**
 * Sidecar workload identity hint.
 *
 * The adapter asserts this in handshake; the sidecar verifies against
 * SO_PEERCRED + signed manifest. Reads `SPENDGUARD_WORKLOAD_INSTANCE_ID`
 * env var; returns empty string when unset.
 */
export function workloadInstanceId(): string {
  return process.env.SPENDGUARD_WORKLOAD_INSTANCE_ID ?? "";
}
