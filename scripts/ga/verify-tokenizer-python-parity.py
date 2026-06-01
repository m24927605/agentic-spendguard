#!/usr/bin/env python3
"""Verify vendored tokenizer.json assets against the Python tokenizers runtime.

POST_GA_03 / #117: Rust tokenizers crate tests catch in-process drift, but
the release train also needs an independent Python oracle because Python is
the most common customer-side tokenizer integration surface.
"""

from __future__ import annotations

from pathlib import Path
import sys

try:
    from tokenizers import Tokenizer
except Exception as exc:  # pragma: no cover - exercised in missing-dep envs
    raise SystemExit(f"python tokenizers package is required: {exc}") from exc


ROOT = Path(__file__).resolve().parents[2]
DATA = ROOT / "crates" / "spendguard-tokenizer" / "data"
DEFAULT_FIXTURE = "spendguard-cross-check-fixture-v1alpha1"
LLAMA_FIXTURE = "spendguard-llama-cross-check-v1alpha1 \u4f60\u597d llama-\u00f1"


CASES = [
    (
        "anthropic-claude3",
        DATA / "anthropic-claude3" / "tokenizer.json",
        DEFAULT_FIXTURE,
        [39995, 17973, 17, 9258, 17, 1584, 17, 12488, 17, 90, 21, 2741, 21],
    ),
    (
        "gemini-1.5",
        DATA / "gemini-1.5" / "tokenizer.json",
        DEFAULT_FIXTURE,
        [120479, 14413, 235290, 16100, 235290, 3534, 235290, 35693, 235290, 235272, 235274, 4705, 235274],
    ),
    (
        "cohere-command-r",
        DATA / "cohere-command-r" / "tokenizer.json",
        DEFAULT_FIXTURE,
        [221325, 37315, 20, 32374, 20, 7399, 20, 61774, 20, 93, 24, 21159, 24],
    ),
    (
        "llama-3.1",
        DATA / "llama-3.1" / "tokenizer.json",
        LLAMA_FIXTURE,
        [2203, 408, 27190, 12, 657, 3105, 77529, 16313, 8437, 16, 7288, 16, 118195, 53901, 94776, 12, 5771],
    ),
]


def main() -> int:
    failures: list[str] = []
    for label, path, fixture, expected in CASES:
        tokenizer = Tokenizer.from_file(str(path))
        actual = tokenizer.encode(fixture, add_special_tokens=False).ids
        if actual != expected:
            failures.append(
                f"{label}: expected {expected[:8]}... len={len(expected)}, "
                f"got {actual[:8]}... len={len(actual)}"
            )
        else:
            print(f"ok {label} len={len(actual)}")

    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
