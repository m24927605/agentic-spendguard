"""Google Gemini token estimator (Gemma approximation).

Spec ref ``tokenizer-service-spec-v1alpha1.md`` §3.1 (Gemini row) +
SLICE_04 R2 M5 honest-disclosure rationale.

Google's official Gemini tokenizer is exposed only via the
``count_tokens`` REST API and is NOT vendorable. We use the open-source
Gemma tokenizer (Apache 2.0, vendored from the Xenova HF mirror) as
the closest publicly-available approximation. Spec §4.2 sets a 1%
drift threshold to accommodate the approximation gap; SLICE_05 shadow
worker quantifies the actual delta in production.

The vendored asset MUST byte-equal the Rust crate's vendored copy
(``crates/spendguard-tokenizer/data/gemini-1.5/tokenizer.json``) so
that the SDK estimator and server-side tokenizer service produce
identical token counts.

Per-message overhead: Gemini's Messages-API equivalent (the
``GenerateContent`` REST endpoint) wraps each turn with role tokens.
We use 3 tokens per message as the community heuristic; 1% drift
threshold absorbs the approximation.
"""

from __future__ import annotations

import hashlib
import importlib.resources as importlib_resources
import threading
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from tokenizers import Tokenizer


# Pinned sha256 of the vendored asset. MUST match
# crates/spendguard-tokenizer/LICENSE_NOTICES.md (SLICE_04 Gemini row).
_ASSET_SHA256_HEX = (
    "05e97791a5e007260de1db7e1692e53150e08cea481e2bf25435553380c147ee"
)

_ASSET_RESOURCE_NAME = "gemini_1_5_tokenizer.json"

# Gemini per-message overhead (community heuristic, 3 tokens per turn).
_PER_MESSAGE_OVERHEAD = 3
_REPLY_PRIMING_OVERHEAD = 2

# Default context window for Gemini 1.5 Pro: 2M tokens. Gemini 1.5
# Flash: 1M tokens. Use the smaller (Flash) value as a conservative
# default when ``max_tokens`` is None and the specific variant isn't
# obvious from the model string.
_DEFAULT_CONTEXT_WINDOW = 1_000_000


_TOKENIZER_CACHE: "Tokenizer | None" = None
_TOKENIZER_LOCK = threading.Lock()


def _load_tokenizer() -> "Tokenizer":
    """Load + verify the vendored Gemma/Gemini tokenizer asset."""
    global _TOKENIZER_CACHE
    if _TOKENIZER_CACHE is not None:
        return _TOKENIZER_CACHE

    with _TOKENIZER_LOCK:
        if _TOKENIZER_CACHE is not None:
            return _TOKENIZER_CACHE

        try:
            from tokenizers import Tokenizer
        except ImportError as exc:  # pragma: no cover — gated by missing-extra
            raise RuntimeError(
                "spendguard.estimators.gemini requires the `tokenizers` "
                "package. Install via "
                "`pip install 'spendguard-sdk[gemini]'` or "
                "`pip install tokenizers`."
            ) from exc

        try:
            resource = (
                importlib_resources.files("spendguard.data")
                / _ASSET_RESOURCE_NAME
            )
            asset_bytes = resource.read_bytes()
        except (FileNotFoundError, ModuleNotFoundError) as exc:
            raise RuntimeError(
                "spendguard.estimators.gemini: vendored tokenizer "
                f"asset `{_ASSET_RESOURCE_NAME}` is missing from the "
                "installed package. Reinstall spendguard-sdk; if the "
                "issue persists this is a packaging bug."
            ) from exc

        actual_hash = hashlib.sha256(asset_bytes).hexdigest()
        if actual_hash != _ASSET_SHA256_HEX:
            raise RuntimeError(
                "spendguard.estimators.gemini: vendored tokenizer "
                f"asset sha256 mismatch. Expected "
                f"{_ASSET_SHA256_HEX!r}, got {actual_hash!r}. The "
                "asset has been tampered with or the wheel build was "
                "corrupted. Refusing to load (per spec §7.4.1)."
            )

        try:
            tk = Tokenizer.from_str(asset_bytes.decode("utf-8"))
        except AttributeError:  # pragma: no cover
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
    tk = _load_tokenizer()
    return len(tk.encode(text, add_special_tokens=False).ids)


def _extract_text(message: object) -> str:
    """Coerce message to text (mirror of anthropic helper)."""
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
    """Estimate input tokens for a Gemini call (Gemma approximation)."""
    total = _REPLY_PRIMING_OVERHEAD
    for msg in messages or []:
        text = _extract_text(msg)
        total += _PER_MESSAGE_OVERHEAD + _encode_one(text)
    return max(1, total)


def count_output_tokens_max(max_tokens: int | None, _model: str) -> int:
    """Strategy A reservation for Gemini models.

    Gemini's ``GenerateContentRequest.generation_config.max_output_tokens``
    is optional. When None, cap at the Gemini 1.5 Flash 1M context
    window as a conservative default.
    """
    if max_tokens is not None and max_tokens > 0:
        return max_tokens
    return _DEFAULT_CONTEXT_WINDOW


__all__ = [
    "count_input_tokens",
    "count_output_tokens_max",
]
