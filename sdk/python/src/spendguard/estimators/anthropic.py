"""Anthropic Claude 3 / 3.5 token estimator using vendored BPE.

Spec ref ``tokenizer-service-spec-v1alpha1.md`` §3.1 (Anthropic row) +
§7.4 (asset signature dual-layer for vendored BPE).

Uses the ``tokenizers`` Python package (HuggingFace) to load the
vendored Claude 3 tokenizer asset shipped at
``spendguard/data/anthropic_claude3_tokenizer.json``. The vendored
asset MUST byte-equal the Rust crate's vendored copy (
``crates/spendguard-tokenizer/data/anthropic-claude3/tokenizer.json``)
so the SDK estimator and the server-side tokenizer service produce
identical token counts.

The asset is verified at first use:

* Layer A — sha256 of the JSON bytes matches the pinned value in
  ``LICENSE_NOTICES.md`` for the Anthropic row. Mismatch ⇒ raise at
  estimator construction (fail-fast; an asset swap is a wire-protocol
  violation per spec §7.4.1).
* Layer B — handled server-side via the cross-check fixture; the
  SDK does not run a Layer B fixture (we trust the asset that ships
  in the wheel since the wheel itself is signed via PyPI Trusted
  Publisher per project_asp_standards_push memory).

Per-message overhead: Anthropic does NOT publish a definitive per-message
overhead the way OpenAI does. The community heuristic is ~5 tokens per
message for the role/content wrapping. We use that value; the 1% drift
threshold (spec §4.2) absorbs the approximation.
"""

from __future__ import annotations

import hashlib
import importlib.resources as importlib_resources
import threading
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from tokenizers import Tokenizer


# Pinned sha256 of the vendored asset. MUST match the value in
# crates/spendguard-tokenizer/LICENSE_NOTICES.md (SLICE_04 row).
# Bumped when the asset is refreshed (per spec §7.3 quarterly cadence).
_ASSET_SHA256_HEX = (
    "c241737df24b4e7f7c9af4fdcee29a0ca903dcb288a8b753bc346a3092911767"
)

_ASSET_RESOURCE_NAME = "anthropic_claude3_tokenizer.json"

# Anthropic per-message overhead (community heuristic).
_PER_MESSAGE_OVERHEAD = 5
_REPLY_PRIMING_OVERHEAD = 3

# Default context window for Claude 3 family (matches Anthropic API
# docs — 200K tokens for the 3.x family). Used by Strategy A when
# the caller passes ``max_tokens=None``.
_DEFAULT_CONTEXT_WINDOW = 200_000


# Module-level cache: the Tokenizer is expensive to construct (~1.7MB
# of BPE merges parsed once). Cached under a lock so concurrent first
# calls don't repeat the work.
_TOKENIZER_CACHE: "Tokenizer | None" = None
_TOKENIZER_LOCK = threading.Lock()


def _load_tokenizer() -> "Tokenizer":
    """Load + verify the vendored Anthropic tokenizer asset.

    Performs Layer A sha256 check at first call. Subsequent calls
    return the cached singleton (locked construction).
    """
    global _TOKENIZER_CACHE
    if _TOKENIZER_CACHE is not None:
        return _TOKENIZER_CACHE

    with _TOKENIZER_LOCK:
        if _TOKENIZER_CACHE is not None:
            return _TOKENIZER_CACHE  # raced; another thread won

        try:
            from tokenizers import Tokenizer
        except ImportError as exc:  # pragma: no cover — gated by missing-extra
            raise RuntimeError(
                "spendguard.estimators.anthropic requires the "
                "`tokenizers` package. Install via "
                "`pip install 'spendguard-sdk[anthropic]'` or "
                "`pip install tokenizers`."
            ) from exc

        # Resolve the vendored asset via importlib.resources so it works
        # in both editable installs and zipped wheels.
        try:
            resource = (
                importlib_resources.files("spendguard.data")
                / _ASSET_RESOURCE_NAME
            )
            asset_bytes = resource.read_bytes()
        except (FileNotFoundError, ModuleNotFoundError) as exc:
            raise RuntimeError(
                "spendguard.estimators.anthropic: vendored tokenizer "
                f"asset `{_ASSET_RESOURCE_NAME}` is missing from the "
                "installed package. Reinstall spendguard-sdk; if the "
                "issue persists this is a packaging bug — please file "
                "an issue with `pip show spendguard-sdk`."
            ) from exc

        actual_hash = hashlib.sha256(asset_bytes).hexdigest()
        if actual_hash != _ASSET_SHA256_HEX:
            raise RuntimeError(
                "spendguard.estimators.anthropic: vendored tokenizer "
                f"asset sha256 mismatch. Expected "
                f"{_ASSET_SHA256_HEX!r}, got {actual_hash!r}. The "
                "asset has been tampered with or the wheel build was "
                "corrupted. Refusing to load (per spec §7.4.1)."
            )

        # Tokenizer.from_str does not exist in all versions; use
        # from_buffer if available, else write to a tmp + from_file.
        try:
            tk = Tokenizer.from_str(asset_bytes.decode("utf-8"))
        except AttributeError:  # pragma: no cover — older versions
            import tempfile

            with tempfile.NamedTemporaryFile(
                mode="wb", suffix=".json", delete=False
            ) as f:
                f.write(asset_bytes)
                tmp_path = f.name
            tk = Tokenizer.from_file(tmp_path)

        _TOKENIZER_CACHE = tk
        return tk


def _encode_one(text: str) -> int:
    """Count tokens for one text string via the vendored encoder."""
    tk = _load_tokenizer()
    return len(tk.encode(text, add_special_tokens=False).ids)


def _extract_text(message: object) -> str:
    """Coerce a message of unknown shape to a single text string."""
    if isinstance(message, str):
        return message
    if isinstance(message, dict):
        content = message.get("content", "")
        if isinstance(content, str):
            return content
        if isinstance(content, list):
            parts = []
            for blk in content:
                if isinstance(blk, dict) and "text" in blk:
                    parts.append(str(blk["text"]))
            return "\n".join(parts)
        return str(content)
    content_attr = getattr(message, "content", None)
    if isinstance(content_attr, str):
        return content_attr
    if content_attr is not None:
        return repr(content_attr)
    return repr(message)


def count_input_tokens(messages: list, _model: str) -> int:
    """Estimate input tokens for an Anthropic call.

    Per-message overhead is the community heuristic 5 tokens (role
    wrapping). 1% drift threshold per spec §4.2 absorbs the
    approximation gap with the official Anthropic count_tokens API.
    """
    total = _REPLY_PRIMING_OVERHEAD
    for msg in messages or []:
        text = _extract_text(msg)
        total += _PER_MESSAGE_OVERHEAD + _encode_one(text)
    return max(1, total)


def count_output_tokens_max(max_tokens: int | None, _model: str) -> int:
    """Strategy A reservation for Anthropic models.

    Per Anthropic docs the default ``max_tokens`` for the Messages API
    is required (unlike OpenAI's optional). We still honor None by
    capping at the family default 200K context window.
    """
    if max_tokens is not None and max_tokens > 0:
        return max_tokens
    return _DEFAULT_CONTEXT_WINDOW


__all__ = [
    "count_input_tokens",
    "count_output_tokens_max",
]
