"""Cost Advisor P0.5 prompt_hash cross-language test vectors.

These five vectors MUST byte-equal the Rust side's SHARED_VECTORS in
``services/sidecar/src/prompt_hash.rs``. Any drift between Python and
Rust here is a P0 bug for cost_advisor's run-scope dedup — the rules
group LLM call retries by ``(run_id, prompt_hash)`` and a Python adapter
computing a different hash than the Rust sidecar would expects to make
the rule silently miss retries.

Run via:
    cd sdk/python && pytest tests/test_prompt_hash.py -v
"""

from __future__ import annotations

import pytest

from spendguard.prompt_hash import compute

# Mirror the Rust SHARED_VECTORS test_tenant value.
TEST_TENANT = "00000000-0000-4000-8000-000000000001"

# Each (input, expected_hex) pair was verified via
#   printf '%s' "$input" | openssl dgst -sha256 -hmac "$TEST_TENANT"
# at vector creation time. Pinned here so a regression in either Rust
# or Python is caught immediately.
SHARED_VECTORS = [
    # 1. Empty string.
    ("", "f35cfe956f859804e9c85f0f9b7ab40f754518045f0af59d5d0da0906f000a08"),
    # 2. Simple ASCII prompt.
    (
        "What is the capital of France?",
        "fcc518b02824c4728ab70e698328685894e07a6f1fa1b19886407188425af723",
    ),
    # 3. Leading + trailing whitespace stripped to same hash as 2.
    (
        "  What is the capital of France?\n",
        "fcc518b02824c4728ab70e698328685894e07a6f1fa1b19886407188425af723",
    ),
    # 4. Internal whitespace preserved (DISTINCT from 2).
    (
        "What is  the capital of France?",
        "b0c13ce5053c66c6d3883662db65c6ce2034920e3bc4544ff370f070e9ed5bf4",
    ),
    # 5. Unicode prompt.
    (
        "Réponds en français.",
        "9a8c1201d05402bad1cb9eea3a6c09ffc6e905aab7dceaa787b5d6e05dadce0e",
    ),
]


@pytest.mark.parametrize("input_text,expected", SHARED_VECTORS)
def test_shared_vector_matches_pinned_hash(input_text: str, expected: str) -> None:
    """Cross-language hash byte-equality against the Rust pinned set."""
    got = compute(input_text, TEST_TENANT)
    assert got == expected, (
        f"prompt_hash drift: input={input_text!r} "
        f"expected={expected} got={got}"
    )


def test_trim_collapses_outer_whitespace() -> None:
    canonical = compute("hello", TEST_TENANT)
    assert compute("  hello", TEST_TENANT) == canonical
    assert compute("hello\n", TEST_TENANT) == canonical
    assert compute("\t hello \r\n", TEST_TENANT) == canonical


def test_internal_whitespace_preserved() -> None:
    assert compute("hello world", TEST_TENANT) != compute("hello  world", TEST_TENANT)
    assert compute("hello\nworld", TEST_TENANT) != compute("hello world", TEST_TENANT)


def test_output_is_lowercase_hex() -> None:
    h = compute("anything", TEST_TENANT)
    assert len(h) == 64
    assert all(c in "0123456789abcdef" for c in h)


def test_different_tenants_produce_different_hashes() -> None:
    """Codex P0.5 r1 P2 fix: cross-tenant linkability defeated."""
    same_prompt = "What is the capital of France?"
    tenant_a = "11111111-1111-4111-8111-111111111111"
    tenant_b = "22222222-2222-4222-8222-222222222222"
    assert compute(same_prompt, tenant_a) != compute(same_prompt, tenant_b)


def test_tenant_id_canonicalization_collapses_case_and_format() -> None:
    """Codex P0.5 r2 P2 fix: same UUID in different string forms hashes equal."""
    prompt = "What is the capital of France?"
    dashed_lower = "00000000-0000-4000-8000-000000000001"
    dashed_upper = dashed_lower.upper()
    simple_no_dashes = "00000000000040008000000000000001"
    assert compute(prompt, dashed_lower) == compute(prompt, dashed_upper)
    assert compute(prompt, dashed_lower) == compute(prompt, simple_no_dashes)


def test_non_uuid_tenant_falls_back_to_raw_string() -> None:
    """Degraded path: non-UUID tenant_id still works deterministically."""
    prompt = "hello"
    assert compute(prompt, "test-tenant") == compute(prompt, "test-tenant")
    assert compute(prompt, "test-tenant-a") != compute(prompt, "test-tenant-b")
