"""Anthropic upstream forwarder.

SLICE 5 — non-streaming ``_invoke`` / ``_generate`` for Anthropic. Mirrors
``openai.py`` structure: per-call client build (NOT cached at module level
— multi-workspace safety, review-standards.md 4.1), exception translation
into Dify ``InvokeError`` subclasses (4.5), no secret material in logs
(4.8 / INV-6).

Message-format adapter: Dify's ``prompt_messages`` use the OpenAI shape
(``role`` ∈ ``{system, user, assistant}``). Anthropic's Messages API
takes a SEPARATE ``system`` parameter (top-level string) plus a
``messages`` list restricted to ``role`` ∈ ``{user, assistant}``. We
split the system messages out at translation time. Multiple system
prompts are joined with double-newlines (Anthropic's recommended
concatenation per their cookbook).

Exception translation:
- ``anthropic.AuthenticationError``  -> ``InvokeAuthorizationError``
- ``anthropic.RateLimitError``       -> ``InvokeRateLimitError``
- ``anthropic.APIConnectionError``   -> ``InvokeConnectionError``
- ``anthropic.BadRequestError``      -> ``InvokeBadRequestError``
- ``anthropic.APIStatusError``       -> ``InvokeServerUnavailableError`` (5xx)
                                      / ``InvokeError`` (others)
- ``anthropic.APIError``             -> ``InvokeError`` (default)

Real usage extraction: ``response.usage.input_tokens`` +
``response.usage.output_tokens`` (Anthropic's shape; OpenAI uses
``prompt_tokens`` + ``completion_tokens``). The ``UpstreamResponse`` is
normalised on the OpenAI shape (``prompt_tokens`` / ``completion_tokens``)
so the reservation commit path is provider-agnostic.
"""

from __future__ import annotations

import logging
from collections.abc import Mapping
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
    import anthropic
    from anthropic import (
        APIConnectionError,
        APIError,
        APIStatusError,
        AuthenticationError,
        BadRequestError,
        RateLimitError,
    )
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.dify_plugin._upstream.anthropic requires the anthropic "
        "package. Install with: pip install anthropic>=0.40",
    ) from exc

from .openai import UpstreamResponse

log = logging.getLogger("spendguard.dify_plugin.upstream.anthropic")

# Anthropic's API requires max_tokens on every request — they don't pick
# a sane default. Dify forms often omit it. We mirror what the egress
# proxy's anthropic adapter uses as a floor.
_DEFAULT_MAX_TOKENS = 1024


def _strip_model_prefix(model: str) -> str:
    """Strip the ``spendguard/`` prefix before forwarding to Anthropic.

    Review-standards.md 4.2 (provider-agnostic): upstream wire MUST
    receive the bare upstream model name (e.g. ``claude-3-5-sonnet-latest``).
    """
    return model.removeprefix("spendguard/")


def _prompt_messages_to_anthropic(
    prompt_messages: list[Any],
) -> tuple[str, list[dict[str, Any]]]:
    """Translate Dify ``PromptMessage`` list to Anthropic's split shape.

    Returns ``(system_prompt, messages_list)``. The ``system_prompt`` is
    the empty string when no system messages are present (Anthropic's
    Messages API accepts an absent or empty ``system`` parameter).

    Edge cases (review-standards.md 5.1):
    - Multiple system messages -> joined with ``\\n\\n``.
    - System messages MUST be elided from the ``messages`` list (the API
      rejects ``role=system`` rows there with a 400).
    - Tool-call passthrough is v1.1+; v1 forwards text + role only.
    """
    system_parts: list[str] = []
    messages: list[dict[str, Any]] = []
    for msg in prompt_messages:
        if hasattr(msg, "role") and hasattr(msg, "content"):
            role_val = msg.role
            role = role_val.value if hasattr(role_val, "value") else str(role_val)
            if hasattr(msg, "get_text_content"):
                content = msg.get_text_content()
            else:
                content = (
                    msg.content if isinstance(msg.content, str) else str(msg.content)
                )
            if role == "system":
                if content:
                    system_parts.append(content)
                continue
            entry: dict[str, Any] = {"role": role, "content": content}
            messages.append(entry)
        elif isinstance(msg, dict):
            if msg.get("role") == "system":
                content = msg.get("content")
                if content:
                    system_parts.append(str(content))
                continue
            messages.append(msg)
        else:  # pragma: no cover
            messages.append({"role": "user", "content": str(msg)})

    # Anthropic requires at least one message in the messages array; if
    # only a system prompt was provided, the upstream will return 400 —
    # we surface that as the BadRequest translation rather than mask.
    system_prompt = "\n\n".join(system_parts)
    return system_prompt, messages


class AnthropicUpstream:
    """Per-call Anthropic forwarder.

    Stateless — every call builds a fresh ``anthropic.Anthropic`` client
    from the call's ``credentials`` (review-standards.md 4.1). Logs MUST
    NOT leak ``anthropic_api_key`` (4.8 / INV-6); we never pass
    credentials to ``log.*``.
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
        api_key = credentials.get("anthropic_api_key") or credentials.get(
            "upstream_api_key",
        )
        if not api_key:
            raise InvokeAuthorizationError(
                "credentials.anthropic_api_key is missing; configure it on "
                "the Dify provider form when upstream_provider=anthropic.",
            )
        base_url = (
            credentials.get("upstream_base_url")
            or credentials.get("anthropic_api_url")
            or None
        )
        upstream_model = _strip_model_prefix(model)
        client = anthropic.Anthropic(
            api_key=str(api_key),
            base_url=str(base_url) if base_url else None,
            timeout=60.0,
        )

        system_prompt, messages = _prompt_messages_to_anthropic(prompt_messages)
        max_tokens = int(model_parameters.get("max_tokens", _DEFAULT_MAX_TOKENS))
        kwargs: dict[str, Any] = {
            "model": upstream_model,
            "messages": messages,
            "max_tokens": max(1, max_tokens),
        }
        if system_prompt:
            kwargs["system"] = system_prompt
        if stop:
            kwargs["stop_sequences"] = stop
        if user:
            # Anthropic uses metadata.user_id for end-user identification.
            kwargs["metadata"] = {"user_id": user}
        # Whitelist of model parameters; the Dify form ships
        # temperature/top_p by default. Anthropic does NOT accept
        # frequency_penalty / presence_penalty — silently drop them.
        for key in ("temperature", "top_p", "top_k"):
            if key in model_parameters:
                kwargs[key] = model_parameters[key]

        try:
            response = client.messages.create(**kwargs)
        except AuthenticationError as exc:
            raise InvokeAuthorizationError(
                f"anthropic authentication failed: {exc}",
            ) from exc
        except RateLimitError as exc:
            raise InvokeRateLimitError(
                f"anthropic rate limit: {exc}",
            ) from exc
        except APIConnectionError as exc:
            raise InvokeConnectionError(
                f"anthropic connection error: {exc}",
            ) from exc
        except BadRequestError as exc:
            raise InvokeBadRequestError(
                f"anthropic bad request: {exc}",
            ) from exc
        except APIStatusError as exc:
            status = getattr(exc, "status_code", 500)
            if status in (502, 503, 504, 529):
                # 529 is Anthropic's "Overloaded" status — fail-closed
                # server unavailable so Dify surfaces a retryable error.
                raise InvokeServerUnavailableError(
                    f"anthropic upstream unavailable (status={status}): {exc}",
                ) from exc
            raise InvokeError(
                f"anthropic upstream error (status={status}): {exc}",
            ) from exc
        except APIError as exc:
            raise InvokeError(f"anthropic upstream error: {exc}") from exc

        return self._to_upstream_response(
            response=response, model=upstream_model,
        )

    @staticmethod
    def _to_upstream_response(
        *, response: Any, model: str,
    ) -> UpstreamResponse:
        """Translate ``anthropic.Message`` -> ``UpstreamResponse``.

        Real usage is pulled from ``response.usage`` (review-standards.md
        4.3 / INV-5). Anthropic's shape uses ``input_tokens`` +
        ``output_tokens`` (not OpenAI's ``prompt_tokens`` +
        ``completion_tokens``); we map onto the OpenAI shape so the
        reservation commit path stays provider-agnostic.

        Anthropic ``Message.content`` is a LIST of content blocks
        (``TextBlock`` / ``ToolUseBlock`` / ...). v1 plugin extracts the
        first text block's ``.text`` field and concatenates additional
        text blocks; tool-use blocks are dropped (tool-calling lands in
        v1.1).
        """
        provider_event_id = str(getattr(response, "id", "") or "")
        # content is a list of typed blocks; extract text content
        content_blocks = getattr(response, "content", None) or []
        text_chunks: list[str] = []
        for block in content_blocks:
            # TextBlock has .type=='text' + .text
            block_type = getattr(block, "type", None)
            if block_type == "text":
                text = getattr(block, "text", "") or ""
                if text:
                    text_chunks.append(text)
        content = "".join(text_chunks)

        usage = getattr(response, "usage", None)
        # Anthropic shape: input_tokens / output_tokens (not OpenAI's
        # prompt/completion). Map onto the OpenAI shape so the commit
        # path is provider-agnostic.
        prompt_tokens = int(getattr(usage, "input_tokens", 0) or 0)
        completion_tokens = int(getattr(usage, "output_tokens", 0) or 0)

        llm_result = LLMResult(
            model=model,
            prompt_messages=[],  # Dify deprecates this field
            message=AssistantPromptMessage(content=content),
            usage=LLMUsage.empty_usage().model_copy(update={
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens,
            }),
            system_fingerprint=None,  # Anthropic has no system_fingerprint
        )
        return UpstreamResponse(
            llm_result=llm_result,
            prompt_tokens=prompt_tokens,
            completion_tokens=completion_tokens,
            provider_event_id=provider_event_id,
        )
