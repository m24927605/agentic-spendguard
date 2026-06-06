"""Generate `sdk/fixtures/cross-language/v1.json` — the canonical cross-language
fixture corpus that both the Python and TypeScript test suites consume.

The Python implementation is the **reference**: every `expected_output` field
in the JSON is the byte exact string produced by calling the in-repo
``spendguard.ids.*`` / ``spendguard.prompt_hash.*`` functions on the inputs
recorded here. The TS suite then asserts the same outputs byte-for-byte. Drift
in either direction is a P0 review-standards §2 blocker.

Run::

    cd sdk/python
    PYTHONPATH=src python ../fixtures/cross-language/generate.py \
        > ../fixtures/cross-language/v1.json

(The output is also written when `--out` is passed.)

Invariants (cf. ``README.md``):
- v1.json MUST NEVER be edited in place once committed. Audit-chain
  immutability: a change in ``expected_output`` for an existing fixture means
  the underlying hash function semantics changed, which would silently
  invalidate every audit row produced before the change. Mint v2.json
  instead.
- New fixtures MAY be appended provided no existing fixture's
  ``id`` / ``fn`` / ``inputs`` change.
- A fixture's ``inputs`` must use the LOCKED named-arg shape from
  ``design.md`` §11 — both the Python and TS suites dispatch by ``fn`` and
  unpack ``inputs`` as kwargs.

Coverage targets (slice COV_S05_09):
- ``derive_idempotency_key``: 8 vectors (FX1-FX8) — covers ASCII, UUID
  tenant, empty trigger, all-empty, alternate trigger, multi-byte UTF-8,
  long IDs, Unit-Separator collision-safety probe.
- ``compute_prompt_hash``: 8 vectors (FXP1-FXP8) — empty prompt, ASCII,
  multi-byte UTF-8, BOM-prefixed, control chars, long 10KB+ prompt, mixed-
  case UUID canonicalisation, non-UUID tenant.
- ``derive_uuid_from_signature``: 4 vectors (FXU1-FXU4) — decision_id,
  llm_call_id, audit_chain, custom scope.

Total: 20 fixtures (≥20 required by COV_S05_09 slice doc).
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import sys
from typing import Any

from spendguard.ids import derive_idempotency_key, derive_uuid_from_signature
from spendguard.prompt_hash import compute as compute_prompt_hash


def _idempotency_vectors() -> list[dict[str, Any]]:
    """8 vectors exercising the LOCKED keyword-arg surface of
    ``derive_idempotency_key``. Every field is permuted at least once across
    the set; FX5/FX8 are edge cases (all-empty, Unit-Separator
    collision-safety)."""
    base = [
        # FX1: dense numeric ASCII — the live R1 drift case.
        {
            "id": "FX1",
            "fn": "derive_idempotency_key",
            "description": "ASCII numeric IDs, LLM_CALL_PRE trigger",
            "inputs": {
                "tenant_id": "t-1",
                "session_id": "s-1",
                "run_id": "r-1",
                "step_id": "step-1",
                "llm_call_id": "llm-1",
                "trigger": "LLM_CALL_PRE",
            },
        },
        # FX2: alternate numeric index.
        {
            "id": "FX2",
            "fn": "derive_idempotency_key",
            "description": "ASCII numeric IDs, alternate values",
            "inputs": {
                "tenant_id": "t-2",
                "session_id": "s-2",
                "run_id": "r-2",
                "step_id": "step-2",
                "llm_call_id": "llm-2",
                "trigger": "LLM_CALL_PRE",
            },
        },
        # FX3: production-shape UUID tenant.
        {
            "id": "FX3",
            "fn": "derive_idempotency_key",
            "description": "Canonical UUID tenant",
            "inputs": {
                "tenant_id": "00000000-0000-0000-0000-000000000001",
                "session_id": "sess-1",
                "run_id": "run-1",
                "step_id": "step-1",
                "llm_call_id": "llm-1",
                "trigger": "LLM_CALL_PRE",
            },
        },
        # FX4: empty trigger but other fields populated.
        {
            "id": "FX4",
            "fn": "derive_idempotency_key",
            "description": "Empty trigger field",
            "inputs": {
                "tenant_id": "tenant-abc",
                "session_id": "sess-1",
                "run_id": "run-1",
                "step_id": "step-1",
                "llm_call_id": "llm-1",
                "trigger": "",
            },
        },
        # FX5: all-empty (degraded but deterministic).
        {
            "id": "FX5",
            "fn": "derive_idempotency_key",
            "description": "All fields empty",
            "inputs": {
                "tenant_id": "",
                "session_id": "",
                "run_id": "",
                "step_id": "",
                "llm_call_id": "",
                "trigger": "",
            },
        },
        # FX6: AGENT_STEP_PRE boundary.
        {
            "id": "FX6",
            "fn": "derive_idempotency_key",
            "description": "AGENT_STEP_PRE trigger boundary",
            "inputs": {
                "tenant_id": "tenant-xyz",
                "session_id": "sess-42",
                "run_id": "run-42",
                "step_id": "step-7",
                "llm_call_id": "llm-7",
                "trigger": "AGENT_STEP_PRE",
            },
        },
        # FX7: multi-byte UTF-8 tenant id (CJK) — encoding gate.
        {
            "id": "FX7",
            "fn": "derive_idempotency_key",
            "description": "Multi-byte UTF-8 tenant id (CJK)",
            "inputs": {
                "tenant_id": "租户-甲",
                "session_id": "sess-1",
                "run_id": "run-1",
                "step_id": "step-1",
                "llm_call_id": "llm-1",
                "trigger": "LLM_CALL_PRE",
            },
        },
        # FX8: Unit-Separator (\x1f) joining collision-safety probe — two
        # inputs that would alias under naive concatenation but MUST NOT
        # collide. The fixture only carries the canonical form, but its
        # `expected_output` lets us cross-check the TS impl uses \x1f too.
        # We pick the second variant ("abcd" / "") of the standard probe.
        {
            "id": "FX8",
            "fn": "derive_idempotency_key",
            "description": "Unit-Separator probe (long tenant, empty session)",
            "inputs": {
                "tenant_id": "abcd",
                "session_id": "",
                "run_id": "x",
                "step_id": "y",
                "llm_call_id": "z",
                "trigger": "T",
            },
        },
    ]
    for entry in base:
        entry["expected_output"] = derive_idempotency_key(**entry["inputs"])
    return base


def _prompt_hash_vectors() -> list[dict[str, Any]]:
    """8 vectors exercising ``compute_prompt_hash`` edge cases.

    Includes empty / ASCII / UTF-8 multi-byte / BOM-prefixed / control char /
    long 10KB+ / mixed-case UUID canonicalisation / non-UUID tenant.
    """
    # 10KB prompt fixture (just above the "long" threshold). Repeated string is
    # fine — we only care that the hash byte-equivalence holds for long input.
    long_prompt = "a" * 10240

    base = [
        # FXP1: ASCII prompt + UUID tenant.
        {
            "id": "FXP1",
            "fn": "compute_prompt_hash",
            "description": "ASCII prompt + canonical UUID tenant",
            "inputs": {
                "prompt_text": "hello world",
                "tenant_id": "00000000-0000-0000-0000-000000000001",
            },
        },
        # FXP2: empty prompt — cold-start retry edge case.
        {
            "id": "FXP2",
            "fn": "compute_prompt_hash",
            "description": "Empty prompt + canonical UUID tenant",
            "inputs": {
                "prompt_text": "",
                "tenant_id": "00000000-0000-0000-0000-000000000001",
            },
        },
        # FXP3: ASCII-whitespace bordered prompt + non-UUID tenant.
        {
            "id": "FXP3",
            "fn": "compute_prompt_hash",
            "description": "Whitespace-padded prompt + non-UUID tenant (strip gate)",
            "inputs": {
                "prompt_text": "  trim me  ",
                "tenant_id": "tenant-abc",
            },
        },
        # FXP4: multi-byte UTF-8 prompt (CJK + ASCII punct).
        {
            "id": "FXP4",
            "fn": "compute_prompt_hash",
            "description": "Multi-byte UTF-8 prompt (CJK punctuation)",
            "inputs": {
                "prompt_text": "Hello, 世界!",
                "tenant_id": "00000000-0000-0000-0000-000000000042",
            },
        },
        # FXP5: BOM-prefixed prompt (U+FEFF) — must NOT be stripped by ASCII
        # whitespace strip. Defends against accidental Unicode-whitespace
        # trimming.
        {
            "id": "FXP5",
            "fn": "compute_prompt_hash",
            "description": "UTF-8 BOM-prefixed prompt (non-ASCII whitespace preserved)",
            "inputs": {
                "prompt_text": "﻿test prompt",
                "tenant_id": "00000000-0000-0000-0000-000000000007",
            },
        },
        # FXP6: control characters inside the prompt (NULL + bell). Must be
        # preserved (no special handling).
        {
            "id": "FXP6",
            "fn": "compute_prompt_hash",
            "description": "Embedded control characters NUL/BEL/VT preserved",
            "inputs": {
                "prompt_text": "before\x00\x07\x0bafter",
                "tenant_id": "tenant-control",
            },
        },
        # FXP7: long 10KB+ prompt — encoding-throughput edge case.
        {
            "id": "FXP7",
            "fn": "compute_prompt_hash",
            "description": "10KB ASCII prompt + non-UUID tenant",
            "inputs": {
                "prompt_text": long_prompt,
                "tenant_id": "tenant-long",
            },
        },
        # FXP8: mixed-case UUID tenant — canonicaliser must lowercase. Both
        # Python and TS impls MUST produce the SAME hash for the upper- and
        # lowercase forms of a UUID with a-f hex digits. The cross-language
        # gate asserts the byte equality of the FXP8 output across runtimes;
        # an additional in-test assertion lower-cases the tenant separately
        # and confirms FXP8 == the lowercase variant's hash (locked).
        {
            "id": "FXP8",
            "fn": "compute_prompt_hash",
            "description": "Mixed-case UUID tenant (canonicaliser lowercases hex)",
            "inputs": {
                "prompt_text": "hello world",
                "tenant_id": "ABCDEF12-3456-7890-ABCD-EF1234567890",
            },
        },
    ]
    for entry in base:
        entry["expected_output"] = compute_prompt_hash(
            entry["inputs"]["prompt_text"],
            entry["inputs"]["tenant_id"],
        )
    return base


def _uuid_from_signature_vectors() -> list[dict[str, Any]]:
    """4 vectors exercising ``derive_uuid_from_signature`` scope namespacing.

    Note: Python uses kw-only ``scope`` (``derive_uuid_from_signature(sig,
    scope=scope)``). The JSON ``inputs`` map carries both fields and is
    unpacked as kwargs after the positional ``signature``."""
    base = [
        # FXU1: live R1 drift case.
        {
            "id": "FXU1",
            "fn": "derive_uuid_from_signature",
            "description": "decision_id scope",
            "inputs": {"signature": "sig-abc", "scope": "decision_id"},
        },
        # FXU2: same signature, different scope — proves namespace separation.
        {
            "id": "FXU2",
            "fn": "derive_uuid_from_signature",
            "description": "llm_call_id scope (same sig as FXU1)",
            "inputs": {"signature": "sig-abc", "scope": "llm_call_id"},
        },
        # FXU3: audit_chain scope — used by audit row UUID derivation.
        {
            "id": "FXU3",
            "fn": "derive_uuid_from_signature",
            "description": "audit_chain scope",
            "inputs": {"signature": "audit-row-v1|tenant-a|2026-06-07", "scope": "audit_chain"},
        },
        # FXU4: custom scope (free-form string) — gate.
        {
            "id": "FXU4",
            "fn": "derive_uuid_from_signature",
            "description": "Custom scope string",
            "inputs": {"signature": "x" * 256, "scope": "custom-test-scope"},
        },
    ]
    for entry in base:
        sig = entry["inputs"]["signature"]
        scope = entry["inputs"]["scope"]
        entry["expected_output"] = str(derive_uuid_from_signature(sig, scope=scope))
    return base


def build() -> dict[str, Any]:
    return {
        "version": 1,
        "generated_at": _dt.date.today().isoformat(),
        "generated_with": {
            "python_reference": "spendguard.ids.* + spendguard.prompt_hash.compute",
            "note": (
                "Python implementation is the reference. TS asserts the SAME "
                "expected_output for the SAME inputs. Drift in either "
                "direction is a P0 review-standards §2 blocker."
            ),
        },
        "fixtures": [
            *_idempotency_vectors(),
            *_prompt_hash_vectors(),
            *_uuid_from_signature_vectors(),
        ],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=str,
        default=None,
        help="Path to write v1.json. Default: stdout.",
    )
    args = parser.parse_args(argv)

    corpus = build()
    payload = json.dumps(corpus, indent=2, ensure_ascii=False) + "\n"
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            fh.write(payload)
    else:
        sys.stdout.write(payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
