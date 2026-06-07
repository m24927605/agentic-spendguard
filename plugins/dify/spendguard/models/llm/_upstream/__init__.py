"""Upstream provider forwarders for the SpendGuard Dify plugin.

v1 ships an OpenAI forwarder. Anthropic lands in SLICE 5; Gemini and
Bedrock are stubbed and raise ``InvokeError`` when selected
(review-standards.md 4.7) so the v1 form does not silently fall through.
"""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any

from dify_plugin.errors.model import InvokeError

from .openai import OpenAIUpstream, UpstreamResponse


def build_upstream_client(credentials: Mapping[str, Any]) -> UpstreamClient:
    """Factory: pick the upstream forwarder based on ``credentials``.

    Operator selects ``upstream_provider`` via the Dify provider form
    (``provider/spendguard.yaml``). Unknown / v1.1+ providers raise
    ``InvokeError`` with the explicit "not supported in this plugin
    version" message (review-standards.md 4.7).
    """
    upstream = str(credentials.get("upstream_provider") or "").strip().lower()
    if upstream == "openai":
        return OpenAIUpstream()
    if upstream in ("anthropic", "gemini", "bedrock"):
        raise InvokeError(
            f"upstream provider {upstream!r} not supported in this plugin "
            "version (v1 ships OpenAI only; Anthropic/Gemini/Bedrock land "
            "in v1.1).",
        )
    if not upstream:
        raise InvokeError(
            "credentials.upstream_provider is missing; pick one of "
            "[openai, anthropic, gemini, bedrock] on the Dify provider form.",
        )
    raise InvokeError(
        f"unknown upstream provider {upstream!r}; expected one of "
        "[openai, anthropic, gemini, bedrock].",
    )


# Re-export for callers
__all__ = ["OpenAIUpstream", "UpstreamClient", "UpstreamResponse", "build_upstream_client"]


# UpstreamClient is a structural protocol — duck-typed Python style.
# We declare a lightweight Protocol so IDEs can hint; runtime uses duck
# typing.
try:
    from typing import Protocol

    class UpstreamClient(Protocol):
        """Forwarder interface.

        Implementations build a per-call API client from ``credentials``
        (NOT cached at module level — multi-workspace safety, see
        review-standards.md 4.1) and translate the response into an
        ``UpstreamResponse`` carrying real usage counts.
        """

        def generate(  # pragma: no cover - protocol stub
            self,
            *,
            model: str,
            credentials: Mapping[str, Any],
            prompt_messages: list[Any],
            model_parameters: dict[str, Any],
            stop: list[str] | None,
            user: str | None,
        ) -> UpstreamResponse: ...
except ImportError:  # pragma: no cover
    UpstreamClient = object  # type: ignore[misc,assignment]
