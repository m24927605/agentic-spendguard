// Field validators — implementation.md §4.2, LOCKED rules.
//
// The rules AND the regexes are part of the cross-language contract: the
// Python `_validate.py` (slice COV_D39_02) mirrors them
// character-for-character. A string accepted by one language and rejected
// by the other is a fixture-level break (review-standards §4.5).
import { AgUiEventValidationError } from "./errors.js";

/** Non-negative atomic decimal string: no sign, no leading zeros. */
const ATOMIC_RE = /^(0|[1-9][0-9]*)$/;

/** RFC 3339 format gate — format check only, no date parsing libs. */
const RFC3339_RE = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/;

/** `typeof s === "string" && s.length > 0` (no trimming — exactness). */
export function requireNonEmpty(field: string, s: unknown): string {
  if (typeof s !== "string" || s.length === 0) {
    throw new AgUiEventValidationError(field, `field "${field}" must be a non-empty string`);
  }
  return s;
}

export function requireAtomic(field: string, s: unknown): string {
  if (typeof s !== "string" || !ATOMIC_RE.test(s)) {
    throw new AgUiEventValidationError(
      field,
      `field "${field}" must be a non-negative atomic decimal string (no sign, no leading zeros)`,
    );
  }
  return s;
}

export function requireRfc3339(field: string, s: unknown): string {
  if (typeof s !== "string" || !RFC3339_RE.test(s)) {
    throw new AgUiEventValidationError(field, `field "${field}" must be an RFC 3339 timestamp`);
  }
  return s;
}

/** Array of non-empty strings; `minLen` 0 or 1 per design §5. */
export function requireStringArray(
  field: string,
  a: unknown,
  opts: { minLen: 0 | 1 },
): readonly string[] {
  if (!Array.isArray(a) || a.length < opts.minLen) {
    throw new AgUiEventValidationError(
      field,
      `field "${field}" must be an array of non-empty strings (>= ${opts.minLen} entries)`,
    );
  }
  for (const entry of a) {
    if (typeof entry !== "string" || entry.length === 0) {
      throw new AgUiEventValidationError(
        field,
        `field "${field}" entries must be non-empty strings`,
      );
    }
  }
  return a as readonly string[];
}

export function requireSafeInteger(field: string, n: unknown): number {
  if (typeof n !== "number" || !Number.isSafeInteger(n) || Object.is(n, -0) || n < 0) {
    throw new AgUiEventValidationError(
      field,
      `field "${field}" must be a non-negative safe integer`,
    );
  }
  return n;
}

/** Returns `{ [field]: s }` when `s` is a non-empty string, else `{}`.
 *  Never throws. This is the design.md §6 omit-if-empty collapse: empty
 *  string and absent are the same thing and serialize identically —
 *  load-bearing for cross-language byte-equivalence (HARDEN_D05_UR). */
export function optionalEntry(field: string, s: unknown): Record<string, string> {
  if (typeof s === "string" && s.length > 0) {
    return { [field]: s };
  }
  return {};
}
