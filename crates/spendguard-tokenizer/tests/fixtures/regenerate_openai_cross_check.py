#!/usr/bin/env python3
"""Verify OPENAI_TIKTOKEN cross-check fixture vectors with Python tiktoken.

This is the reference regeneration helper for issue #109. It intentionally
checks the committed JSON rather than printing a detached table, so reviewers
can rerun it after tiktoken upgrades and see the exact case that drifted.

Install the pinned reference dependency with:
    python3 -m pip install -r requirements-openai-cross-check.txt
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import tiktoken


FIXTURE_PATH = Path(__file__).with_name("cross_check.json")
EXPECTED_TIKTOKEN_VERSION = "0.12.0"


def main() -> int:
    actual_version = getattr(tiktoken, "__version__", None)
    if actual_version != EXPECTED_TIKTOKEN_VERSION:
        print(
            "Python tiktoken reference version mismatch: "
            f"expected {EXPECTED_TIKTOKEN_VERSION}, got {actual_version!r}. "
            "Install tests/fixtures/requirements-openai-cross-check.txt.",
            file=sys.stderr,
        )
        return 2

    manifest = json.loads(FIXTURE_PATH.read_text(encoding="utf-8"))
    cases = manifest["kinds"]["OPENAI_TIKTOKEN"]["cases"]
    failures: list[str] = []

    for case in cases:
        encoder_name = case["encoder"]
        encoder = tiktoken.get_encoding(encoder_name)
        actual_ids = encoder.encode(case["input"])
        expected_ids = case["expected_token_ids"]
        expected_count = case["expected_token_count"]

        if actual_ids != expected_ids:
            failures.append(
                f"{case['case_id']} ids drifted for {encoder_name}: "
                f"expected={expected_ids} actual={actual_ids}"
            )
        if len(actual_ids) != expected_count:
            failures.append(
                f"{case['case_id']} count drifted for {encoder_name}: "
                f"expected={expected_count} actual={len(actual_ids)}"
            )

    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1

    print(
        f"verified {len(cases)} OPENAI_TIKTOKEN fixture cases "
        f"with Python tiktoken {actual_version}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
