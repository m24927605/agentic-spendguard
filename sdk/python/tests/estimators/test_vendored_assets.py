"""Unit tests for vendored asset loading + sha256 verification.

SLICE_12 Phase B — every estimator that loads a vendored BPE asset
MUST verify the sha256 hash matches the pinned value. Mismatch ⇒
``RuntimeError`` at first call (fail-fast per spec §7.4.1).

These tests cover:
* Asset file is present in the installed package.
* sha256 of the shipped asset matches the pinned value in the
  estimator module (parity check; if the file is regenerated but the
  hash isn't bumped this test catches the drift).
* The estimator loads + tokenises a sample without error.
* Asset signature mismatch ⇒ RuntimeError (simulated via monkey-patch).

Also verifies the LICENSE_NOTICES.md pinned hashes match the shipped
files so the legal documentation stays in sync with the wire bytes.
"""

from __future__ import annotations

import hashlib
import importlib.resources as importlib_resources
import re
from pathlib import Path

import pytest


# Pin the expected SHA256 directly here (independent of the source).
# This is a parity / wire-protocol check: if either the estimator
# pinned value OR the shipped asset drifts, the test fails.
EXPECTED_ANTHROPIC_SHA256 = (
    "c241737df24b4e7f7c9af4fdcee29a0ca903dcb288a8b753bc346a3092911767"
)
EXPECTED_GEMINI_SHA256 = (
    "05e97791a5e007260de1db7e1692e53150e08cea481e2bf25435553380c147ee"
)


class TestAssetSignaturesShipped:
    """Verify vendored asset bytes match pinned sha256 in source."""

    def test_anthropic_asset_shipped_with_correct_sha256(self) -> None:
        resource = importlib_resources.files("spendguard.data") / "anthropic_claude3_tokenizer.json"
        asset_bytes = resource.read_bytes()
        actual = hashlib.sha256(asset_bytes).hexdigest()
        assert actual == EXPECTED_ANTHROPIC_SHA256, (
            f"Anthropic asset sha256 mismatch — re-download the asset "
            f"from the URL in LICENSE_NOTICES.md or bump the pinned "
            f"hash if intentional. Expected {EXPECTED_ANTHROPIC_SHA256}, "
            f"got {actual}."
        )

    def test_gemini_asset_shipped_with_correct_sha256(self) -> None:
        resource = importlib_resources.files("spendguard.data") / "gemini_1_5_tokenizer.json"
        asset_bytes = resource.read_bytes()
        actual = hashlib.sha256(asset_bytes).hexdigest()
        assert actual == EXPECTED_GEMINI_SHA256, (
            f"Gemini asset sha256 mismatch — see anthropic test. "
            f"Expected {EXPECTED_GEMINI_SHA256}, got {actual}."
        )


class TestEstimatorAssetPinning:
    """Verify the estimator modules' _ASSET_SHA256_HEX constants match
    the shipped files. Catches the case where someone regenerates the
    asset but forgets to bump the pinned hash."""

    def test_anthropic_estimator_pinned_hash(self) -> None:
        from spendguard.estimators.anthropic import _ASSET_SHA256_HEX as pinned

        assert pinned == EXPECTED_ANTHROPIC_SHA256

    def test_gemini_estimator_pinned_hash(self) -> None:
        from spendguard.estimators.gemini import _ASSET_SHA256_HEX as pinned

        assert pinned == EXPECTED_GEMINI_SHA256


class TestLicenseNoticesParity:
    """Verify LICENSE_NOTICES.md pinned hashes match shipped files.

    Source of truth is `LICENSE_NOTICES.md` (legal docs). If the
    estimator's _ASSET_SHA256_HEX drifts from this file the operator
    cannot verify the asset matches what we claim to ship.
    """

    @staticmethod
    def _read_license_notices() -> str:
        # Locate LICENSE_NOTICES.md relative to this test file. In
        # editable installs it's at sdk/python/LICENSE_NOTICES.md;
        # in wheel-installed envs it ships at the package root via
        # the sdist `include` rule.
        test_dir = Path(__file__).resolve().parent
        candidates = [
            test_dir.parent.parent / "LICENSE_NOTICES.md",  # sdk/python/LICENSE_NOTICES.md
        ]
        for c in candidates:
            if c.exists():
                return c.read_text()
        pytest.skip(f"LICENSE_NOTICES.md not found at {candidates}")
        return ""  # unreachable

    def test_license_notices_anthropic_hash(self) -> None:
        content = self._read_license_notices()
        # Match the row format `Asset sha256 | `\`HASH\`` | `
        match = re.search(
            r"Anthropic Claude.*?Asset sha256.*?\|\s*`([a-f0-9]{64})`",
            content,
            re.DOTALL,
        )
        assert match, "LICENSE_NOTICES.md missing Anthropic sha256 row"
        assert match.group(1) == EXPECTED_ANTHROPIC_SHA256

    def test_license_notices_gemini_hash(self) -> None:
        content = self._read_license_notices()
        match = re.search(
            r"Google Gemini.*?Asset sha256.*?\|\s*`([a-f0-9]{64})`",
            content,
            re.DOTALL,
        )
        assert match, "LICENSE_NOTICES.md missing Gemini sha256 row"
        assert match.group(1) == EXPECTED_GEMINI_SHA256


class TestAssetVerificationFailFast:
    """Asset tampering ⇒ RuntimeError at first estimator call.

    Simulated by monkey-patching the estimator's pinned sha256 to a
    value that intentionally doesn't match the shipped asset. The
    estimator MUST raise rather than silently proceed (per
    `tokenizer-service-spec-v1alpha1.md` §7.4.1).
    """

    def test_anthropic_sha256_mismatch_raises(
        self, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        import spendguard.estimators.anthropic as mod

        # Reset cache so the load happens fresh under the bad hash.
        monkeypatch.setattr(mod, "_TOKENIZER_CACHE", None)
        monkeypatch.setattr(mod, "_ASSET_SHA256_HEX", "0" * 64)

        with pytest.raises(RuntimeError, match="sha256 mismatch"):
            mod.count_input_tokens([{"role": "user", "content": "x"}], "claude-3-5-sonnet")

    def test_gemini_sha256_mismatch_raises(
        self, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        import spendguard.estimators.gemini as mod

        monkeypatch.setattr(mod, "_TOKENIZER_CACHE", None)
        monkeypatch.setattr(mod, "_ASSET_SHA256_HEX", "1" * 64)

        with pytest.raises(RuntimeError, match="sha256 mismatch"):
            mod.count_input_tokens([{"role": "user", "content": "x"}], "gemini-1.5-flash")


class TestEstimatorEndToEnd:
    """Sanity: estimator loads and counts tokens correctly."""

    def test_anthropic_counts_nonzero(self) -> None:
        from spendguard.estimators.anthropic import count_input_tokens

        # Reset cache so we exercise the load path
        import spendguard.estimators.anthropic as mod

        mod._TOKENIZER_CACHE = None

        result = count_input_tokens(
            [{"role": "user", "content": "Hello, Claude!"}], "claude-3-5-sonnet"
        )
        assert result > 0
        assert result < 100  # sanity upper bound

    def test_gemini_counts_nonzero(self) -> None:
        from spendguard.estimators.gemini import count_input_tokens

        import spendguard.estimators.gemini as mod

        mod._TOKENIZER_CACHE = None

        result = count_input_tokens(
            [{"role": "user", "content": "Hello, Gemini!"}], "gemini-1.5-flash"
        )
        assert result > 0
        assert result < 100

    def test_anthropic_output_strategy_a(self) -> None:
        from spendguard.estimators.anthropic import count_output_tokens_max

        assert count_output_tokens_max(500, "claude-3-5-sonnet") == 500
        assert count_output_tokens_max(None, "claude-3-5-sonnet") == 200_000
        assert count_output_tokens_max(0, "claude-3-5-sonnet") == 200_000

    def test_gemini_output_strategy_a(self) -> None:
        from spendguard.estimators.gemini import count_output_tokens_max

        assert count_output_tokens_max(500, "gemini-1.5-flash") == 500
        assert count_output_tokens_max(None, "gemini-1.5-flash") == 1_000_000

    def test_anthropic_via_estimator_for_model(self) -> None:
        from spendguard.estimators import estimator_for_model

        e = estimator_for_model("claude-3-5-sonnet-20240620")
        assert e.encoder_name == "anthropic-v3-bpe"
        count = e.count_input_tokens(
            [{"role": "user", "content": "What is 2+2?"}], "claude-3-5-sonnet-20240620"
        )
        assert count > 0

    def test_gemini_via_estimator_for_model(self) -> None:
        from spendguard.estimators import estimator_for_model

        e = estimator_for_model("gemini-1.5-pro")
        assert e.encoder_name == "gemini-1.5-bpe"
        count = e.count_input_tokens(
            [{"role": "user", "content": "What is 2+2?"}], "gemini-1.5-pro"
        )
        assert count > 0

    def test_anthropic_tokenizer_cached_across_calls(self) -> None:
        from spendguard.estimators.anthropic import _load_tokenizer

        tk1 = _load_tokenizer()
        tk2 = _load_tokenizer()
        # Same singleton instance returned on second call
        assert tk1 is tk2
