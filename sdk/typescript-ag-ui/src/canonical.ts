// canonicalEventJson — the design.md §7 cross-language byte-equivalence
// rule, LOCKED. Applied to the WHOLE envelope ({type, name, value,
// timestamp?}).
//
// Recursive key-sorted rebuild + JSON.stringify: stringify of a key-ordered
// object emits keys in insertion order, which we set to sorted order.
// `Object.keys(...).sort()` sorts by UTF-16 code units; ASCII-only keys
// (enforced below) make that provably identical to Python's code-point sort
// (design.md §7.4). Under the §7 constraints, JSON.stringify and Python's
// `json.dumps(obj, ensure_ascii=False, sort_keys=True, separators=(",",":"))`
// agree on the escape set: `"` `\` and the C0 controls (shorthand
// \b \t \n \f \r, \u00XX otherwise); all other characters pass through as
// raw UTF-8.
import { AgUiEventValidationError } from "./errors.js";
import type { SpendGuardAgUiEvent } from "./events.js";

/** Keys: printable ASCII only ([\x21-\x7e]), enforced — throw on violation
 *  (design.md §7.4). Mirrors Python `_ASCII_KEY_RE`. */
const ASCII_KEY_RE = /^[\x21-\x7e]+$/;

export function canonicalEventJson(event: SpendGuardAgUiEvent): string {
  return JSON.stringify(canonicalize(event));
}

function canonicalize(v: unknown): unknown {
  if (typeof v === "string") {
    assertWellFormed(v);
    return v;
  }
  if (typeof v === "boolean") {
    return v;
  }
  if (typeof v === "number") {
    assertCanonicalInt(v);
    return v;
  }
  if (Array.isArray(v)) {
    // Array order is preserved as given — arrays are caller-ordered,
    // e.g. reason_codes (design.md §7.6).
    return v.map(canonicalize);
  }
  if (v !== null && typeof v === "object") {
    const rec = v as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const k of Object.keys(rec).sort()) {
      assertAsciiKey(k);
      out[k] = canonicalize(rec[k]);
    }
    return out;
  }
  // null is forbidden — omit the key instead (design.md §7.5).
  throw new AgUiEventValidationError(
    "(value)",
    "null/undefined/unsupported type in canonical payload",
  );
}

/** Strings must be valid Unicode; unpaired surrogates are rejected
 *  (design.md §7.5). `String.prototype.isWellFormed()` — Node 20+. */
function assertWellFormed(s: string): void {
  if (!s.isWellFormed()) {
    throw new AgUiEventValidationError("(value)", "unpaired surrogate in canonical string value");
  }
}

/** Integers only: |n| <= 2^53 - 1 (Number.isSafeInteger — also rejects
 *  floats and non-finite numbers) and -0 forbidden (design.md §7.5). */
function assertCanonicalInt(n: number): void {
  if (!Number.isSafeInteger(n) || Object.is(n, -0)) {
    throw new AgUiEventValidationError(
      "(value)",
      "floats, non-finite numbers, -0, and unsafe integers are forbidden in canonical payload",
    );
  }
}

function assertAsciiKey(k: string): void {
  if (!ASCII_KEY_RE.test(k)) {
    throw new AgUiEventValidationError(
      "(key)",
      "object keys must be printable ASCII [\\x21-\\x7e]",
    );
  }
}
