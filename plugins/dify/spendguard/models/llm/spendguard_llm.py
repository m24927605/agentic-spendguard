"""SpendGuardLLM — the Dify ``LargeLanguageModel`` adapter.

Bridges Dify's synchronous ``_invoke`` SDK contract to the SpendGuard
async reservation lifecycle (``_DifyReservation``). Per review-standards.md
cross-cutting "Async / sync mixing" row, the plugin daemon SDK calls
``_invoke`` synchronously; we MUST bridge to async via a daemon-scoped
event loop, NOT ``asyncio.run()`` per call (which would create+destroy
an event loop per call and break gRPC channel reuse).

Slice coverage:
- SLICE 4 (shipped): OpenAI non-streaming forwarder.
- SLICE 5 (this slice): Anthropic non-streaming forwarder +
  ``get_num_tokens`` routed through the sidecar ``/v1/tokenize`` HTTP
  companion (falls back to chars/4 on companion failure).
- SLICE 6 (this slice): SSE streaming via ``_stream_generate`` for both
  OpenAI and Anthropic, with end-of-stream commit on SUCCESS and
  release on FAILURE / CANCELLED.

Lifecycle per call:
    1. Build DifyCallContext from credentials + prompt_messages.
    2. ``_DifyReservation.reserve`` -> ALLOW / DENY / DEGRADE.
       DENY -> raise ``InvokeAuthorizationError`` (HTTP 403 via Dify).
       DEGRADE -> raise ``InvokeServerUnavailableError`` (HTTP 503).
    3. Non-streaming: ``UpstreamClient.generate`` -> single response;
       commit_success with real usage.
       Streaming: ``UpstreamStream.stream`` -> chunk generator; each
       chunk yielded as ``LLMResultChunk``; final commit at
       end-of-stream with accumulated usage.
    4. On any upstream error: ``_DifyReservation.release_failure``;
       re-raise the translated ``InvokeError``.
"""

from __future__ import annotations

import asyncio
import logging
import os
import threading
from collections.abc import Generator
from typing import Any

from dify_plugin.entities.model.llm import (
    LLMResult,
    LLMResultChunk,
    LLMResultChunkDelta,
    LLMUsage,
)
from dify_plugin.entities.model.message import (
    AssistantPromptMessage,
    PromptMessage,
    PromptMessageTool,
)
from dify_plugin.errors.model import (
    InvokeAuthorizationError,
    InvokeBadRequestError,
    InvokeConnectionError,
    InvokeError,
    InvokeRateLimitError,
    InvokeServerUnavailableError,
)
from dify_plugin.interfaces.model.large_language_model import LargeLanguageModel
from spendguard.errors import (
    DecisionDenied,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardError,
)

from ._DifyReservation import (
    DifyCallContext,
    ReservationHandle,
    _DifyReservation,
)
from ._upstream import build_upstream_client
from ._upstream.openai import UpstreamResponse

log = logging.getLogger("spendguard.dify_plugin.llm")


# ---------------------------------------------------------------------------
# Daemon-scoped event loop (sync/async bridge)
# ---------------------------------------------------------------------------

class _DaemonLoop:
    """Lazy-initialised event loop running on a background thread.

    The Dify plugin daemon SDK calls ``_invoke`` synchronously, but the
    SpendGuard SDK is async-only (gRPC.aio over UDS). Per
    review-standards.md cross-cutting "Async / sync mixing": we MUST
    bridge via a daemon-scoped loop, NOT ``asyncio.run()`` per call.

    The loop is started once on first use and reused for the daemon's
    lifetime. ``run`` blocks the caller thread until the coroutine
    completes.

    Test-injection seam: ``set_test_instance`` allows tests to inject a
    synchronous stub so unit tests don't need the background thread
    (which interferes with pytest-asyncio's per-test runner under
    gevent-monkey-patched dify_plugin imports).
    """

    _instance: _DaemonLoop | None = None
    _instance_lock = threading.Lock()

    @classmethod
    def get(cls) -> _DaemonLoop:
        if cls._instance is not None:
            return cls._instance
        with cls._instance_lock:
            if cls._instance is None:
                cls._instance = _DaemonLoop()
        return cls._instance

    @classmethod
    def set_test_instance(cls, instance: _DaemonLoop | None) -> None:
        """Override the singleton for tests. Pass ``None`` to reset."""
        with cls._instance_lock:
            cls._instance = instance

    def __init__(self, *, _skip_thread: bool = False) -> None:
        self._loop: asyncio.AbstractEventLoop | None = None
        self._thread: threading.Thread | None = None
        self._ready = threading.Event()
        if not _skip_thread:
            self._start()

    def _start(self) -> None:
        def _runner() -> None:
            loop = asyncio.new_event_loop()
            asyncio.set_event_loop(loop)
            self._loop = loop
            self._ready.set()
            try:
                loop.run_forever()
            finally:
                loop.close()

        self._thread = threading.Thread(
            target=_runner, name="spendguard-dify-loop", daemon=True,
        )
        self._thread.start()
        # Wait for the loop to be ready; bounded so misconfig surfaces
        # quickly instead of hanging the plugin daemon boot.
        if not self._ready.wait(timeout=5.0):
            raise SpendGuardConfigError(
                "spendguard plugin daemon loop failed to start within 5s",
            )

    def run(self, coro: Any, *, timeout: float = 90.0) -> Any:
        """Submit ``coro`` to the loop; block until it returns or raises.

        ``timeout`` upper-bounds the per-call wait. The default is loose
        because upstream calls (OpenAI chat completion) can take 60s+.
        """
        if self._loop is None:  # pragma: no cover — _start guarantees
            raise SpendGuardConfigError("daemon loop not initialised")
        future = asyncio.run_coroutine_threadsafe(coro, self._loop)
        return future.result(timeout=timeout)


class _SyncLoopStub(_DaemonLoop):
    """Test-only: runs coroutines synchronously via ``asyncio.run``.

    Used in unit tests to avoid the background thread loop (which fights
    with pytest-asyncio's per-test runner under gevent-monkey-patched
    imports). NOT for production use.
    """

    def __init__(self) -> None:
        super().__init__(_skip_thread=True)

    def run(self, coro: Any, *, timeout: float = 90.0) -> Any:
        del timeout  # synchronous runner ignores timeout
        return asyncio.new_event_loop().run_until_complete(coro)


# ---------------------------------------------------------------------------
# Sidecar tokenize companion (SLICE 5)
# ---------------------------------------------------------------------------

def _sidecar_tokenize(
    *,
    text: str,
    provider: str,
    model: str,
    sidecar_url: str | None = None,
    timeout_s: float = 0.5,
) -> int | None:
    """Call the sidecar HTTP companion ``/v1/tokenize`` and return count.

    Returns ``None`` on any failure (network error, non-200 response,
    timeout, missing companion URL) so the caller can fall back to the
    chars/4 heuristic. Failures are logged at WARN so operators can
    diagnose companion outages without breaking the get_num_tokens
    contract.

    Provider/model are forwarded so the companion can route to the
    correct tokenizer tier (T1 for Anthropic count_tokens shadow, T2 for
    OpenAI tiktoken / Anthropic BPE, etc. — see tokenizer-spec-v1alpha1.md).

    Review-standards.md 5.4: ``get_num_tokens`` SHOULD route through the
    sidecar tokenize companion when ``SPENDGUARD_SIDECAR_HTTP_URL`` is
    set on the plugin daemon. When unset, fall back to chars/4.
    """
    if not sidecar_url:
        sidecar_url = os.environ.get(
            "SPENDGUARD_SIDECAR_HTTP_URL", "",
        ).strip()
    if not sidecar_url:
        return None
    try:
        # Lazy import — operators with no companion don't pay the import
        # cost at plugin daemon boot.
        import httpx
    except ImportError:  # pragma: no cover
        log.warning(
            "spendguard: httpx not installed; cannot reach sidecar "
            "tokenize companion. get_num_tokens falls back to chars/4.",
        )
        return None
    endpoint = sidecar_url.rstrip("/") + "/v1/tokenize"
    body = {
        "text": text,
        "provider": provider,
        "model": model,
        # The companion currently echoes the caller's tier hint when no
        # real tokenizer is wired (see services/sidecar/src/http_companion).
        # We leave these blank so the companion picks its server-side
        # default — the plugin should NOT pretend to know the live
        # tokenizer tier.
        "tokenizer_tier": "",
        "tokenizer_version_id": "",
    }
    try:
        with httpx.Client(timeout=timeout_s) as client:
            resp = client.post(endpoint, json=body)
        if resp.status_code != 200:
            log.warning(
                "spendguard: sidecar tokenize companion returned %d for "
                "provider=%s model=%s; falling back to chars/4.",
                resp.status_code, provider, model,
            )
            return None
        payload = resp.json()
        token_count = payload.get("token_count")
        if token_count is None:
            return None
        return int(token_count)
    except Exception as exc:
        # Companion errors are non-fatal: the get_num_tokens contract
        # only promises a rough count, so we degrade quietly.
        log.warning(
            "spendguard: sidecar tokenize companion call failed "
            "(provider=%s model=%s): %r; falling back to chars/4.",
            provider, model, exc,
        )
        return None


def _classify_upstream(model: str, credentials: dict) -> tuple[str, str]:
    """Return ``(provider, upstream_model)`` for sidecar tokenize routing.

    The ``provider`` is the Dify credentials selection; the
    ``upstream_model`` strips the ``spendguard/`` prefix so the sidecar
    can route to the correct tokenizer vocab.
    """
    provider = str(credentials.get("upstream_provider") or "openai").strip().lower()
    upstream_model = model.removeprefix("spendguard/")
    return provider, upstream_model


# ---------------------------------------------------------------------------
# Streaming accumulator (SLICE 6)
# ---------------------------------------------------------------------------

class _StreamingAccumulator:
    """Collects content + usage across SSE chunks; commits at end-of-stream.

    Review-standards.md 6.3: streaming MUST commit the same shape as
    non-streaming. The accumulator tracks prompt_tokens, completion_tokens,
    final content, provider_event_id, and the upstream's terminal usage
    block. On end-of-stream, the caller fires
    ``reservation.commit_success`` with the accumulated values.

    Design:
    - OpenAI streams include a final ``usage`` chunk when the client
      requests it (``stream_options={"include_usage": True}``); we set
      that flag unconditionally so commit always has real usage.
    - Anthropic streams include ``message_start`` with input_tokens and
      ``message_delta`` with output_tokens (no separate "usage" chunk);
      we accumulate from both events.
    - If the upstream OMITS usage (rare; some compatibility layers), we
      fall back to a chars-based estimate of the accumulated content
      (review-standards.md 6.4) and emit a WARN at commit time.
    """

    def __init__(self) -> None:
        self.content: list[str] = []
        self.prompt_tokens: int = 0
        self.completion_tokens: int = 0
        self.provider_event_id: str = ""
        self.had_usage: bool = False

    def append_text(self, text: str) -> None:
        if text:
            self.content.append(text)

    def update_usage(
        self,
        *,
        prompt_tokens: int | None = None,
        completion_tokens: int | None = None,
    ) -> None:
        if prompt_tokens is not None:
            self.prompt_tokens = int(prompt_tokens)
            self.had_usage = True
        if completion_tokens is not None:
            # Anthropic delta usage is cumulative-or-final; OpenAI usage
            # is final. Both fit a "last write wins" semantic on
            # completion_tokens.
            self.completion_tokens = int(completion_tokens)
            self.had_usage = True

    def fallback_estimate(self) -> None:
        """Fill prompt/completion when upstream omitted usage.

        Review-standards.md 6.4: chars/4 fallback when streaming usage
        is missing. Conservative: we never under-count.
        """
        if self.had_usage:
            return
        joined = "".join(self.content)
        approx = max(1, len(joined) // 4)
        self.completion_tokens = approx
        # prompt_tokens stays 0 — the upstream didn't tell us. The
        # estimator-snapshot reservation amount picked up the prompt
        # cost at reserve time.

    def build_llm_result(self, *, model: str) -> LLMResult:
        return LLMResult(
            model=model,
            prompt_messages=[],
            message=AssistantPromptMessage(content="".join(self.content)),
            usage=LLMUsage.empty_usage().model_copy(update={
                "prompt_tokens": self.prompt_tokens,
                "completion_tokens": self.completion_tokens,
                "total_tokens": self.prompt_tokens + self.completion_tokens,
            }),
        )


# ---------------------------------------------------------------------------
# SpendGuardLLM — the Dify SDK adapter
# ---------------------------------------------------------------------------

class SpendGuardLLM(LargeLanguageModel):
    """Dify ``LargeLanguageModel`` that gates every call through SpendGuard.

    Composition: ``SpendGuardLLM`` adapts the Dify SDK signature;
    ``_DifyReservation`` (delegate, not parent) owns the SpendGuard
    reserve/commit/release lifecycle.
    """

    _reservation: _DifyReservation | None = None
    _reservation_lock = threading.Lock()

    def _get_reservation(self) -> _DifyReservation:
        """Lazy-init the reservation delegate.

        Reads ``SPENDGUARD_SIDECAR_UDS`` + ``SPENDGUARD_TENANT_ID`` from
        the plugin daemon env. The delegate raises ``SpendGuardConfigError``
        with the offending var named if either is missing
        (review-standards.md 3.2). Lazy init avoids importing the SDK at
        module-load (review-standards.md 1.5 — no outbound on import).
        """
        if self._reservation is not None:
            return self._reservation
        with type(self)._reservation_lock:
            if type(self)._reservation is None:
                type(self)._reservation = _DifyReservation()
            self._reservation = type(self)._reservation
        return self._reservation

    # ------------------------------------------------------------------
    # _invoke — the Dify SDK entrypoint
    # ------------------------------------------------------------------

    def _invoke(
        self,
        model: str,
        credentials: dict,
        prompt_messages: list[PromptMessage],
        model_parameters: dict,
        tools: list[PromptMessageTool] | None = None,
        stop: list[str] | None = None,
        stream: bool = True,
        user: str | None = None,
    ) -> LLMResult | Generator[LLMResultChunk, None, None]:
        """Reserve -> forward -> commit/release. SLICE 6 adds streaming."""
        ctx = self._build_call_context(
            model=model,
            credentials=credentials,
            prompt_messages=prompt_messages,
            stream=stream,
            user=user,
        )
        if stream:
            return self._stream_generate(
                ctx=ctx,
                model_parameters=model_parameters,
                stop=stop,
                user=user,
            )
        return self._generate(
            ctx=ctx,
            model_parameters=model_parameters,
            stop=stop,
            user=user,
        )

    # ------------------------------------------------------------------
    # validate_credentials — install-time probe entrypoint
    # ------------------------------------------------------------------

    def validate_credentials(
        self,
        model: str,
        credentials: dict,
    ) -> None:
        """Surface a credentials probe to the Dify install flow.

        The real install-time roundtrip lives in
        ``provider/spendguard.py::SpendguardProvider.validate_provider_credentials``
        (INV-4 — 1-token reserve+release). This method is the Dify SDK's
        per-model hook; we delegate the deep probe to the provider class
        but still surface obvious shape problems (missing keys) so the
        SDK validation path doesn't silently no-op.
        """
        from dify_plugin.errors.model import CredentialsValidateFailedError

        for key in (
            "upstream_provider",
            "spendguard_budget_id",
            "spendguard_window_instance_id",
        ):
            if not str(credentials.get(key) or "").strip():
                raise CredentialsValidateFailedError(
                    f"credentials.{key} is missing",
                )
        upstream = str(credentials.get("upstream_provider", "")).strip().lower()
        if upstream == "openai" and not (
            credentials.get("openai_api_key")
            or credentials.get("upstream_api_key")
        ):
            raise CredentialsValidateFailedError(
                "credentials.openai_api_key is missing",
            )
        if upstream == "anthropic" and not (
            credentials.get("anthropic_api_key")
            or credentials.get("upstream_api_key")
        ):
            raise CredentialsValidateFailedError(
                "credentials.anthropic_api_key is missing",
            )
        if upstream not in ("openai", "anthropic", "gemini", "bedrock"):
            raise CredentialsValidateFailedError(
                f"upstream_provider {upstream!r} is not a supported value",
            )
        # NOTE: deep sidecar roundtrip is performed by the provider
        # class (provider/spendguard.py::validate_provider_credentials).

    # ------------------------------------------------------------------
    # _invoke_error_mapping — Dify SDK error translation table
    # ------------------------------------------------------------------

    @property
    def _invoke_error_mapping(self) -> dict[type[InvokeError], list[type[Exception]]]:
        """Map upstream exceptions to Dify ``InvokeError`` subclasses.

        Dify's SDK consults this dict to translate exception types raised
        from ``_invoke`` into the unified hierarchy. SLICE 5 extends the
        v1 OpenAI table with the anthropic.* family. The anthropic
        package is imported lazily so operators who never use Anthropic
        don't pay the import cost.
        """
        import openai

        mapping: dict[type[InvokeError], list[type[Exception]]] = {
            InvokeAuthorizationError: [openai.AuthenticationError],
            InvokeRateLimitError: [openai.RateLimitError],
            InvokeConnectionError: [openai.APIConnectionError],
            InvokeBadRequestError: [openai.BadRequestError],
            InvokeServerUnavailableError: [openai.APIStatusError],
            InvokeError: [openai.APIError, RuntimeError],
        }
        try:
            import anthropic
            mapping[InvokeAuthorizationError].append(
                anthropic.AuthenticationError,
            )
            mapping[InvokeRateLimitError].append(anthropic.RateLimitError)
            mapping[InvokeConnectionError].append(
                anthropic.APIConnectionError,
            )
            mapping[InvokeBadRequestError].append(anthropic.BadRequestError)
            mapping[InvokeServerUnavailableError].append(
                anthropic.APIStatusError,
            )
            mapping[InvokeError].append(anthropic.APIError)
        except ImportError:  # pragma: no cover
            # Anthropic not installed — operators only using OpenAI still
            # get a correct mapping for openai.* exceptions.
            pass
        return mapping

    # ------------------------------------------------------------------
    # get_num_tokens — sidecar companion + fallback (SLICE 5)
    # ------------------------------------------------------------------

    def get_num_tokens(
        self,
        model: str,
        credentials: dict,
        prompt_messages: list[PromptMessage],
        tools: list[PromptMessageTool] | None = None,
    ) -> int:
        """Return a token count for the prompt.

        SLICE 5 routes through the sidecar ``/v1/tokenize`` HTTP companion
        when ``SPENDGUARD_SIDECAR_HTTP_URL`` is set on the plugin daemon
        env (review-standards.md 5.4). Falls back to chars/4 when:
        - the companion URL isn't configured, OR
        - the companion is unreachable / times out / returns non-200, OR
        - httpx is not installed.

        The fallback matches the egress proxy's legacy heuristic
        (decision.rs:277-295 pre-tokenizer-upgrade path) so the plugin
        is self-contained without bundling tiktoken.
        """
        total_chars = 0
        for msg in prompt_messages:
            if hasattr(msg, "get_text_content"):
                total_chars += len(msg.get_text_content() or "")
            elif hasattr(msg, "content") and isinstance(msg.content, str):
                total_chars += len(msg.content)

        provider, upstream_model = _classify_upstream(model, credentials)
        # Build the canonical text once for the companion. The companion
        # currently echoes the caller-supplied text length; future
        # slices wire real tokenization upstream.
        text_chunks: list[str] = []
        for msg in prompt_messages:
            if hasattr(msg, "get_text_content"):
                text_chunks.append(msg.get_text_content() or "")
            elif hasattr(msg, "content") and isinstance(msg.content, str):
                text_chunks.append(msg.content)
        # Flatten message texts with newlines so the companion sees
        # boundary structure (consistent with the prompt_hash pre-image).
        flat_text = "\n".join(text_chunks)

        # Try the sidecar companion first; fall back on any failure.
        sidecar_url = (
            credentials.get("spendguard_sidecar_http_url")
            or os.environ.get("SPENDGUARD_SIDECAR_HTTP_URL", "")
        )
        if sidecar_url:
            count = _sidecar_tokenize(
                text=flat_text,
                provider=provider,
                model=upstream_model,
                sidecar_url=str(sidecar_url),
            )
            if count is not None:
                return max(1, int(count))
        # Fallback heuristic: chars/4.
        return max(1, total_chars // 4)

    # ------------------------------------------------------------------
    # Internals — sync/async bridge
    # ------------------------------------------------------------------

    def _build_call_context(
        self,
        *,
        model: str,
        credentials: dict,
        prompt_messages: list[PromptMessage],
        stream: bool,
        user: str | None,
    ) -> DifyCallContext:
        return DifyCallContext(
            workspace_id=str(credentials.get("__dify_workspace_id") or ""),
            app_id=credentials.get("__dify_app_id"),
            model=model,
            prompt_messages=prompt_messages,
            stream=stream,
            credentials=credentials,
            user=user,
        )

    def _generate(
        self,
        *,
        ctx: DifyCallContext,
        model_parameters: dict,
        stop: list[str] | None,
        user: str | None,
    ) -> LLMResult:
        """Sync wrapper around the async reserve/forward/commit cycle."""
        reservation = self._get_reservation()
        upstream = build_upstream_client(ctx.credentials)

        loop = _DaemonLoop.get()
        handle: ReservationHandle | None = None
        try:
            try:
                handle = loop.run(reservation.reserve(ctx))
            except DecisionDenied as exc:
                # INV-1: DENY never hits the upstream provider.
                # review-standards.md 4.4 — no outbound HTTP on DENY.
                raise InvokeAuthorizationError(
                    f"SpendGuard denied the call (decision_id={exc.decision_id}): "
                    f"{exc}",
                ) from exc
            except SidecarUnavailable as exc:
                raise InvokeServerUnavailableError(
                    f"SpendGuard sidecar unavailable: {exc}",
                ) from exc
            except SpendGuardConfigError as exc:
                raise InvokeError(
                    f"SpendGuard configuration error: {exc}",
                ) from exc
            except SpendGuardError as exc:
                raise InvokeServerUnavailableError(
                    f"SpendGuard error: {exc}",
                ) from exc

            try:
                response: UpstreamResponse = upstream.generate(
                    model=ctx.model,
                    credentials=ctx.credentials,
                    prompt_messages=ctx.prompt_messages,
                    model_parameters=model_parameters,
                    stop=stop,
                    user=user,
                )
            except InvokeError:
                # Upstream raised a Dify error — release reservation and
                # re-raise. Release failures are swallowed
                # (review-standards.md 3.6).
                if handle is not None:
                    self._release_on_error(loop, reservation, handle)
                raise
            except BaseException:
                if handle is not None:
                    self._release_on_error(loop, reservation, handle)
                raise

            # Real usage from response.usage feeds commit (INV-5).
            real_amount = self._compute_real_amount_atomic(response)
            try:
                loop.run(reservation.commit_success(
                    handle,
                    real_amount_atomic=str(real_amount),
                    provider_event_id=response.provider_event_id,
                    actual_input_tokens=response.prompt_tokens,
                    actual_output_tokens=response.completion_tokens,
                ))
            except SpendGuardError as exc:
                # Commit failed — the LLM call already succeeded so we
                # MUST return the result. TTL sweep is the backstop. Log
                # WARN with no secret material (review-standards.md INV-6).
                log.warning(
                    "spendguard: commit failed for llm_call_id=%s err=%r; "
                    "reservation will TTL-sweep.",
                    handle.llm_call_id, exc,
                )
            return response.llm_result
        except InvokeError:
            raise
        except Exception:
            # Belt-and-braces release on unexpected exceptions.
            if handle is not None:
                self._release_on_error(loop, reservation, handle)
            raise

    # ------------------------------------------------------------------
    # _stream_generate — SSE proxy for OpenAI + Anthropic (SLICE 6)
    # ------------------------------------------------------------------

    def _stream_generate(
        self,
        *,
        ctx: DifyCallContext,
        model_parameters: dict,
        stop: list[str] | None,
        user: str | None,
    ) -> Generator[LLMResultChunk, None, None]:
        """SSE proxy that yields ``LLMResultChunk`` and commits at end.

        Review-standards.md 6.x summary:
        - reserve fires once before any upstream HTTP (INV-1).
        - Each upstream SSE event maps to one ``LLMResultChunk`` yield.
        - Accumulator captures content + usage across chunks.
        - On normal completion, commit_success fires with accumulated
          real usage (or chars/4 estimate when upstream omits usage).
        - On any error mid-stream (or caller cancellation), release_failure
          fires and the InvokeError propagates.
        - The upstream client is built per-call (review-standards.md 4.1).
        """
        reservation = self._get_reservation()
        loop = _DaemonLoop.get()
        provider = str(ctx.credentials.get("upstream_provider") or "openai").strip().lower()

        # Reserve before any upstream HTTP (INV-1).
        try:
            handle = loop.run(reservation.reserve(ctx))
        except DecisionDenied as exc:
            raise InvokeAuthorizationError(
                f"SpendGuard denied the call (decision_id={exc.decision_id}): "
                f"{exc}",
            ) from exc
        except SidecarUnavailable as exc:
            raise InvokeServerUnavailableError(
                f"SpendGuard sidecar unavailable: {exc}",
            ) from exc
        except SpendGuardConfigError as exc:
            raise InvokeError(
                f"SpendGuard configuration error: {exc}",
            ) from exc
        except SpendGuardError as exc:
            raise InvokeServerUnavailableError(
                f"SpendGuard error: {exc}",
            ) from exc

        accumulator = _StreamingAccumulator()
        try:
            # Dispatch on provider; per-provider streaming helper raises
            # translated InvokeError subclasses on upstream failure.
            if provider == "openai":
                yield from self._stream_openai(
                    ctx=ctx,
                    model_parameters=model_parameters,
                    stop=stop,
                    user=user,
                    accumulator=accumulator,
                )
            elif provider == "anthropic":
                yield from self._stream_anthropic(
                    ctx=ctx,
                    model_parameters=model_parameters,
                    stop=stop,
                    user=user,
                    accumulator=accumulator,
                )
            else:
                # Provider routing for gemini/bedrock raises InvokeError
                # at build_upstream_client; mirror that here for symmetry.
                build_upstream_client(ctx.credentials)
                # Unreachable: build_upstream_client raises before this.
                raise InvokeError(  # pragma: no cover
                    f"streaming not implemented for provider {provider!r}",
                )
        except (InvokeError, GeneratorExit, BaseException) as exc:
            # Stream errored or caller cancelled — release + re-raise.
            # GeneratorExit fires when the caller closes the generator
            # (Dify SSE cancellation). asyncio.CancelledError covers the
            # async cancellation path; the reservation delegate classifies
            # both as CANCELLED on its own.
            self._release_on_error(loop, reservation, handle, exc=exc)
            raise
        else:
            # Normal end-of-stream — commit with accumulated usage.
            accumulator.fallback_estimate()
            real_amount = accumulator.prompt_tokens + accumulator.completion_tokens
            try:
                loop.run(reservation.commit_success(
                    handle,
                    real_amount_atomic=str(real_amount),
                    provider_event_id=accumulator.provider_event_id,
                    actual_input_tokens=accumulator.prompt_tokens,
                    actual_output_tokens=accumulator.completion_tokens,
                ))
            except SpendGuardError as exc:
                log.warning(
                    "spendguard: streaming commit failed for "
                    "llm_call_id=%s err=%r; reservation will TTL-sweep.",
                    handle.llm_call_id, exc,
                )

    def _stream_openai(
        self,
        *,
        ctx: DifyCallContext,
        model_parameters: dict,
        stop: list[str] | None,
        user: str | None,
        accumulator: _StreamingAccumulator,
    ) -> Generator[LLMResultChunk, None, None]:
        """Stream OpenAI ``chat.completions`` with ``include_usage=True``.

        Each SSE event is a ``ChatCompletionChunk``. The final chunk
        carries ``usage`` when we set ``stream_options.include_usage=True``
        (review-standards.md 6.3 — required so we get real prompt/
        completion counts at end-of-stream).
        """
        import openai

        from ._upstream.openai import _prompt_messages_to_openai, _strip_model_prefix

        api_key = ctx.credentials.get("openai_api_key") or ctx.credentials.get(
            "upstream_api_key",
        )
        if not api_key:
            raise InvokeAuthorizationError(
                "credentials.openai_api_key is missing; configure it on "
                "the Dify provider form when upstream_provider=openai.",
            )
        base_url = (
            ctx.credentials.get("upstream_base_url")
            or ctx.credentials.get("openai_api_base")
            or None
        )
        upstream_model = _strip_model_prefix(ctx.model)
        oai = openai.OpenAI(
            api_key=str(api_key),
            base_url=str(base_url) if base_url else None,
            timeout=60.0,
        )
        kwargs: dict[str, Any] = {
            "model": upstream_model,
            "messages": _prompt_messages_to_openai(ctx.prompt_messages),
            "stream": True,
            # MUST include usage chunk so commit_success has real usage.
            "stream_options": {"include_usage": True},
        }
        if stop:
            kwargs["stop"] = stop
        if user:
            kwargs["user"] = user
        for key in ("temperature", "top_p", "max_tokens",
                    "frequency_penalty", "presence_penalty"):
            if key in model_parameters:
                kwargs[key] = model_parameters[key]

        try:
            stream = oai.chat.completions.create(**kwargs)
        except openai.AuthenticationError as exc:
            raise InvokeAuthorizationError(
                f"openai authentication failed: {exc}",
            ) from exc
        except openai.RateLimitError as exc:
            raise InvokeRateLimitError(f"openai rate limit: {exc}") from exc
        except openai.APIConnectionError as exc:
            raise InvokeConnectionError(
                f"openai connection error: {exc}",
            ) from exc
        except openai.BadRequestError as exc:
            raise InvokeBadRequestError(
                f"openai bad request: {exc}",
            ) from exc
        except openai.APIStatusError as exc:
            status = getattr(exc, "status_code", 500)
            if status in (502, 503, 504):
                raise InvokeServerUnavailableError(
                    f"openai upstream unavailable (status={status}): {exc}",
                ) from exc
            raise InvokeError(
                f"openai upstream error (status={status}): {exc}",
            ) from exc
        except openai.APIError as exc:
            raise InvokeError(f"openai upstream error: {exc}") from exc

        try:
            chunk_index = 0
            for chunk in stream:
                # End-of-stream usage chunk has empty choices but
                # carries response.usage.
                chunk_id = getattr(chunk, "id", "") or ""
                if chunk_id and not accumulator.provider_event_id:
                    accumulator.provider_event_id = str(chunk_id)
                usage = getattr(chunk, "usage", None)
                if usage is not None:
                    accumulator.update_usage(
                        prompt_tokens=getattr(usage, "prompt_tokens", None),
                        completion_tokens=getattr(usage, "completion_tokens", None),
                    )
                choices = getattr(chunk, "choices", None) or []
                if not choices:
                    continue
                choice = choices[0]
                delta = getattr(choice, "delta", None)
                delta_text = ""
                if delta is not None:
                    delta_text = getattr(delta, "content", None) or ""
                finish_reason = getattr(choice, "finish_reason", None)
                accumulator.append_text(delta_text)
                yield LLMResultChunk(
                    model=upstream_model,
                    prompt_messages=[],
                    delta=LLMResultChunkDelta(
                        index=chunk_index,
                        message=AssistantPromptMessage(content=delta_text),
                        finish_reason=finish_reason,
                    ),
                )
                chunk_index += 1
        except openai.APIError as exc:
            raise InvokeError(f"openai stream error: {exc}") from exc

    def _stream_anthropic(
        self,
        *,
        ctx: DifyCallContext,
        model_parameters: dict,
        stop: list[str] | None,
        user: str | None,
        accumulator: _StreamingAccumulator,
    ) -> Generator[LLMResultChunk, None, None]:
        """Stream Anthropic ``messages.stream`` SSE events.

        Anthropic's SSE event types:
        - ``message_start`` — carries ``message.id`` + ``usage.input_tokens``.
        - ``content_block_delta`` — incremental text delta.
        - ``message_delta`` — final ``usage.output_tokens``.
        - ``message_stop`` — terminator.

        We use the SDK's ``client.messages.stream(**kwargs)`` context
        manager which yields typed events.
        """
        import anthropic

        from ._upstream.anthropic import (
            _DEFAULT_MAX_TOKENS,
            _prompt_messages_to_anthropic,
            _strip_model_prefix,
        )

        api_key = ctx.credentials.get("anthropic_api_key") or ctx.credentials.get(
            "upstream_api_key",
        )
        if not api_key:
            raise InvokeAuthorizationError(
                "credentials.anthropic_api_key is missing; configure it on "
                "the Dify provider form when upstream_provider=anthropic.",
            )
        base_url = (
            ctx.credentials.get("upstream_base_url")
            or ctx.credentials.get("anthropic_api_url")
            or None
        )
        upstream_model = _strip_model_prefix(ctx.model)
        client = anthropic.Anthropic(
            api_key=str(api_key),
            base_url=str(base_url) if base_url else None,
            timeout=60.0,
        )
        system_prompt, messages = _prompt_messages_to_anthropic(ctx.prompt_messages)
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
            kwargs["metadata"] = {"user_id": user}
        for key in ("temperature", "top_p", "top_k"):
            if key in model_parameters:
                kwargs[key] = model_parameters[key]

        try:
            stream_ctx = client.messages.stream(**kwargs)
        except anthropic.AuthenticationError as exc:
            raise InvokeAuthorizationError(
                f"anthropic authentication failed: {exc}",
            ) from exc
        except anthropic.RateLimitError as exc:
            raise InvokeRateLimitError(
                f"anthropic rate limit: {exc}",
            ) from exc
        except anthropic.APIConnectionError as exc:
            raise InvokeConnectionError(
                f"anthropic connection error: {exc}",
            ) from exc
        except anthropic.BadRequestError as exc:
            raise InvokeBadRequestError(
                f"anthropic bad request: {exc}",
            ) from exc
        except anthropic.APIStatusError as exc:
            status = getattr(exc, "status_code", 500)
            if status in (502, 503, 504, 529):
                raise InvokeServerUnavailableError(
                    f"anthropic upstream unavailable (status={status}): {exc}",
                ) from exc
            raise InvokeError(
                f"anthropic upstream error (status={status}): {exc}",
            ) from exc
        except anthropic.APIError as exc:
            raise InvokeError(f"anthropic upstream error: {exc}") from exc

        chunk_index = 0
        try:
            with stream_ctx as stream:
                for event in stream:
                    event_type = getattr(event, "type", None)
                    if event_type == "message_start":
                        message = getattr(event, "message", None)
                        msg_id = getattr(message, "id", "") if message else ""
                        if msg_id and not accumulator.provider_event_id:
                            accumulator.provider_event_id = str(msg_id)
                        usage = getattr(message, "usage", None) if message else None
                        if usage is not None:
                            accumulator.update_usage(
                                prompt_tokens=getattr(usage, "input_tokens", None),
                            )
                        continue
                    if event_type == "content_block_delta":
                        delta = getattr(event, "delta", None)
                        delta_text = (
                            getattr(delta, "text", "") if delta is not None else ""
                        )
                        if delta_text:
                            accumulator.append_text(delta_text)
                            yield LLMResultChunk(
                                model=upstream_model,
                                prompt_messages=[],
                                delta=LLMResultChunkDelta(
                                    index=chunk_index,
                                    message=AssistantPromptMessage(
                                        content=delta_text,
                                    ),
                                    finish_reason=None,
                                ),
                            )
                            chunk_index += 1
                        continue
                    if event_type == "message_delta":
                        usage = getattr(event, "usage", None)
                        if usage is not None:
                            accumulator.update_usage(
                                completion_tokens=getattr(
                                    usage, "output_tokens", None,
                                ),
                            )
                        # Surface the stop_reason as the finish_reason on
                        # a terminal empty delta so Dify sees the close.
                        delta_attr = getattr(event, "delta", None)
                        stop_reason = (
                            getattr(delta_attr, "stop_reason", None)
                            if delta_attr is not None
                            else None
                        )
                        if stop_reason:
                            yield LLMResultChunk(
                                model=upstream_model,
                                prompt_messages=[],
                                delta=LLMResultChunkDelta(
                                    index=chunk_index,
                                    message=AssistantPromptMessage(content=""),
                                    finish_reason=stop_reason,
                                ),
                            )
                            chunk_index += 1
                        continue
                    # Other events (message_stop, content_block_start /
                    # _stop, ping) are no-ops for the SpendGuard side.
        except anthropic.APIError as exc:
            raise InvokeError(f"anthropic stream error: {exc}") from exc

    @staticmethod
    def _release_on_error(
        loop: _DaemonLoop,
        reservation: _DifyReservation,
        handle: ReservationHandle,
        *,
        exc: BaseException | str | None = None,
    ) -> None:
        """Fire-and-forget release on error paths.

        Release errors are swallowed by ``_DifyReservation.release_failure``
        itself (review-standards.md 3.6); we still guard against the
        loop.run timeout / unexpected exceptions here so the original
        upstream error reaches the caller.
        """
        try:
            loop.run(
                reservation.release_failure(handle, exc or "upstream_failure"),
                timeout=10.0,
            )
        except Exception as rel_exc:
            log.warning(
                "spendguard: release loop submission failed for "
                "llm_call_id=%s err=%r; reservation will TTL-sweep.",
                handle.llm_call_id, rel_exc,
            )

    @staticmethod
    def _compute_real_amount_atomic(response: UpstreamResponse) -> int:
        """Compute the atomic ledger amount from real upstream usage.

        v1 plugin treats the budget unit as ``atomic.usd.micro`` (default
        in ``_build_binding_from_credentials``); the projector picks up
        real micro-USD pricing via the sidecar pricing snapshot. For v1
        we feed total tokens through as a proxy — the sidecar will
        re-evaluate against pricing on commit. Future slices replace this
        with explicit pricing math when the operator overrides the
        binding.
        """
        return int(response.prompt_tokens + response.completion_tokens)
