"""OpenAI upstream forwarder.

SLICE 4 — non-streaming ``_invoke`` / ``_generate``. Builds an
``openai.OpenAI`` client from ``credentials.openai_api_key`` +
``credentials.upstream_base_url`` per-call (NOT cached at module level —
multi-workspace safety, review-standards.md 4.1).

Translates ``openai.*`` exception hierarchy into Dify ``InvokeError``
subclasses (review-standards.md 4.5):

- ``openai.AuthenticationError``    -> ``InvokeAuthorizationError``
- ``openai.RateLimitError``         -> ``InvokeRateLimitError``
- ``openai.APIConnectionError``     -> ``InvokeConnectionError``
- ``openai.APIError`` (default)     -> ``InvokeError``
"""

from __future__ import annotations

import logging
from collections.abc import Mapping
from dataclasses import dataclass
from typing import Any

from dify_plugin.entities.model.llm import LLMResult, LLMUsage
from dify_plugin.entities.model.message import AssistantPromptMessage
from dify_plugin.errors.model import (
    InvokeAuthorizationError,
    InvokeBadRequestError,
    InvokeConnectionError,
    InvokeError,
    InvokeRateLimitError,
    InvokeServerUnavailableError,
)

try:  # pragma: no cover — import-time only
    import openai
    from openai import (
        APIConnectionError,
        APIError,
        APIStatusError,
        AuthenticationError,
        BadRequestError,
        RateLimitError,
    )
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.dify_plugin._upstream.openai requires the openai "
        "package. Install with: pip install openai>=1.40",
    ) from exc

log = logging.getLogger("spendguard.dify_plugin.upstream.openai")


@dataclass(frozen=True, slots=True)
class UpstreamResponse:
    """Canonical response shape across upstream providers.

    Fields the reservation layer needs to commit the audit row:
    - ``llm_result`` for Dify to return to its caller.
    - ``prompt_tokens`` / ``completion_tokens`` for ``real_amount_atomic``
      (review-standards.md 4.3 / INV-5).
    - ``provider_event_id`` for cross-system correlation.
    """
    llm_result: LLMResult
    prompt_tokens: int
    completion_tokens: int
    provider_event_id: str


def _strip_model_prefix(model: str) -> str:
    """Strip the ``spendguard/`` prefix before forwarding to OpenAI.

    Review-standards.md 4.2: the upstream wire MUST receive the bare
    upstream model name.
    """
    return model.removeprefix("spendguard/")


def _prompt_messages_to_openai(prompt_messages: list[Any]) -> list[dict[str, Any]]:
    """Translate Dify ``PromptMessage`` list to OpenAI Chat Completions
    messages.

    Dify's prompt_messages are pydantic models. We use ``get_text_content``
    when available (handles multimodal stripping) and fall back to
    ``str(content)`` otherwise. Tool-call shape passthrough on
    ``AssistantPromptMessage.tool_calls`` is handled minimally; v1 plugin
    forwards text + role only — full tool-call shape lands when a v1.1
    slice adds tool-calling.
    """
    out: list[dict[str, Any]] = []
    for msg in prompt_messages:
        if hasattr(msg, "role") and hasattr(msg, "content"):
            role_val = msg.role
            role = role_val.value if hasattr(role_val, "value") else str(role_val)
            if hasattr(msg, "get_text_content"):
                content = msg.get_text_content()
            else:
                content = msg.content if isinstance(msg.content, str) else str(msg.content)
            entry: dict[str, Any] = {"role": role, "content": content}
            name = getattr(msg, "name", None)
            if name:
                entry["name"] = name
            out.append(entry)
        elif isinstance(msg, dict):
            out.append(msg)
        else:  # pragma: no cover
            out.append({"role": "user", "content": str(msg)})
    return out


class OpenAIUpstream:
    """Per-call OpenAI forwarder.

    Stateless — every call builds a fresh ``openai.OpenAI`` client from
    the call's ``credentials`` (review-standards.md 4.1). Logs MUST NOT
    leak ``openai_api_key`` (review-standards.md 4.8 + INV-6); we
    explicitly avoid passing credentials into any ``log.*`` call.
    """

    def generate(
        self,
        *,
        model: str,
        credentials: Mapping[str, Any],
        prompt_messages: list[Any],
        model_parameters: dict[str, Any],
        stop: list[str] | None = None,
        user: str | None = None,
    ) -> UpstreamResponse:
        api_key = credentials.get("openai_api_key") or credentials.get(
            "upstream_api_key",
        )
        if not api_key:
            raise InvokeAuthorizationError(
                "credentials.openai_api_key is missing; configure it on "
                "the Dify provider form when upstream_provider=openai.",
            )
        base_url = (
            credentials.get("upstream_base_url")
            or credentials.get("openai_api_base")
            or None
        )
        upstream_model = _strip_model_prefix(model)
        oai = openai.OpenAI(
            api_key=str(api_key),
            base_url=str(base_url) if base_url else None,
            timeout=60.0,
        )
        kwargs: dict[str, Any] = {
            "model": upstream_model,
            "messages": _prompt_messages_to_openai(prompt_messages),
        }
        if stop:
            kwargs["stop"] = stop
        if user:
            kwargs["user"] = user
        # Whitelist of model parameters; the Dify form ships
        # temperature/top_p/max_tokens by default.
        for key in ("temperature", "top_p", "max_tokens",
                    "frequency_penalty", "presence_penalty"):
            if key in model_parameters:
                kwargs[key] = model_parameters[key]

        try:
            response = oai.chat.completions.create(**kwargs)
        except AuthenticationError as exc:
            raise InvokeAuthorizationError(
                f"openai authentication failed: {exc}",
            ) from exc
        except RateLimitError as exc:
            raise InvokeRateLimitError(
                f"openai rate limit: {exc}",
            ) from exc
        except APIConnectionError as exc:
            raise InvokeConnectionError(
                f"openai connection error: {exc}",
            ) from exc
        except BadRequestError as exc:
            raise InvokeBadRequestError(
                f"openai bad request: {exc}",
            ) from exc
        except APIStatusError as exc:
            status = getattr(exc, "status_code", 500)
            if status in (502, 503, 504):
                raise InvokeServerUnavailableError(
                    f"openai upstream unavailable (status={status}): {exc}",
                ) from exc
            raise InvokeError(
                f"openai upstream error (status={status}): {exc}",
            ) from exc
        except APIError as exc:
            raise InvokeError(f"openai upstream error: {exc}") from exc

        return self._to_upstream_response(
            response=response, model=upstream_model,
        )

    @staticmethod
    def _to_upstream_response(
        *, response: Any, model: str,
    ) -> UpstreamResponse:
        """Translate ``openai.ChatCompletion`` -> ``UpstreamResponse``.

        Real usage is pulled from ``response.usage`` (review-standards.md
        4.3 / INV-5). If ``response.usage`` is None (rare; some
        compatibility layers omit it), defaults to 0 — the reservation
        layer will emit a WARN about estimator fallback at commit time.
        """
        provider_event_id = str(getattr(response, "id", "") or "")
        try:
            choice = response.choices[0]
            message = choice.message
            content = getattr(message, "content", None) or ""
        except (AttributeError, IndexError) as exc:
            raise InvokeError(
                f"openai response missing choices[0].message: {exc}",
            ) from exc

        usage = getattr(response, "usage", None)
        prompt_tokens = int(getattr(usage, "prompt_tokens", 0) or 0)
        completion_tokens = int(getattr(usage, "completion_tokens", 0) or 0)

        llm_result = LLMResult(
            model=model,
            prompt_messages=[],  # Dify deprecates this field; ship empty
            message=AssistantPromptMessage(content=content),
            usage=LLMUsage.empty_usage().model_copy(update={
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens,
            }),
            system_fingerprint=str(getattr(response, "system_fingerprint", "") or "") or None,
        )
        return UpstreamResponse(
            llm_result=llm_result,
            prompt_tokens=prompt_tokens,
            completion_tokens=completion_tokens,
            provider_event_id=provider_event_id,
        )
