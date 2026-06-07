"""SpendGuardLLM — the Dify ``LargeLanguageModel`` adapter.

Bridges Dify's synchronous ``_invoke`` SDK contract to the SpendGuard
async reservation lifecycle (``_DifyReservation``). Per review-standards.md
cross-cutting "Async / sync mixing" row, the plugin daemon SDK calls
``_invoke`` synchronously; we MUST bridge to async via a daemon-scoped
event loop, NOT ``asyncio.run()`` per call (which would create+destroy
an event loop per call and break gRPC channel reuse).

v1 scope (SLICE 4): non-streaming OpenAI forwarder. Streaming lands in
SLICE 6 (the ``_stream_generate`` path raises ``InvokeError`` in v1).

Lifecycle per call:
    1. Build DifyCallContext from credentials + prompt_messages.
    2. ``_DifyReservation.reserve`` -> ALLOW / DENY / DEGRADE.
       DENY -> raise ``InvokeAuthorizationError`` (HTTP 403 via Dify).
       DEGRADE -> raise ``InvokeServerUnavailableError`` (HTTP 503).
    3. ``OpenAIUpstream.generate`` -> upstream HTTP.
    4. ``_DifyReservation.commit_success`` with real usage.
    5. On any upstream error: ``_DifyReservation.release_failure``;
       re-raise the translated ``InvokeError``.
"""

from __future__ import annotations

import asyncio
import logging
import threading
from collections.abc import Generator
from typing import Any

from dify_plugin.entities.model.llm import LLMResult, LLMResultChunk
from dify_plugin.entities.model.message import (
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
        """Reserve -> forward -> commit/release. v1 = non-streaming only."""
        if stream:
            # SLICE 6 will replace this with _stream_generate. Until then
            # we fail-fast so the user sees an actionable error rather
            # than silently coercing to non-streaming.
            raise InvokeError(
                "streaming is not supported in SpendGuard plugin v1.0; "
                "set stream=False or wait for v1.1 (SLICE 6).",
            )
        ctx = self._build_call_context(
            model=model,
            credentials=credentials,
            prompt_messages=prompt_messages,
            stream=stream,
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
        from ``_invoke`` into the unified hierarchy. Our v1 plugin already
        catches openai.* in ``_upstream/openai.py`` and re-raises as
        ``InvokeError`` subclasses directly, so the mapping mostly
        documents the contract; the SDK's fallback path uses it.
        """
        import openai

        return {
            InvokeAuthorizationError: [openai.AuthenticationError],
            InvokeRateLimitError: [openai.RateLimitError],
            InvokeConnectionError: [openai.APIConnectionError],
            InvokeBadRequestError: [openai.BadRequestError],
            InvokeServerUnavailableError: [openai.APIStatusError],
            InvokeError: [openai.APIError, RuntimeError],
        }

    # ------------------------------------------------------------------
    # get_num_tokens — fallback estimate
    # ------------------------------------------------------------------

    def get_num_tokens(
        self,
        model: str,
        credentials: dict,
        prompt_messages: list[PromptMessage],
        tools: list[PromptMessageTool] | None = None,
    ) -> int:
        """Return a rough token count.

        SLICE 5 will route this through the sidecar ``count_tokens`` UDS
        RPC (review-standards.md 5.4). For v1 (SLICE 4), we use the same
        chars/4 heuristic the egress proxy uses pre-tokenizer-upgrade
        (decision.rs:277-295 legacy path) so the plugin daemon is
        self-contained without bundling tiktoken just for this method.
        """
        total_chars = 0
        for msg in prompt_messages:
            if hasattr(msg, "get_text_content"):
                total_chars += len(msg.get_text_content() or "")
            elif hasattr(msg, "content") and isinstance(msg.content, str):
                total_chars += len(msg.content)
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

    @staticmethod
    def _release_on_error(
        loop: _DaemonLoop,
        reservation: _DifyReservation,
        handle: ReservationHandle,
    ) -> None:
        """Fire-and-forget release on error paths.

        Release errors are swallowed by ``_DifyReservation.release_failure``
        itself (review-standards.md 3.6); we still guard against the
        loop.run timeout / unexpected exceptions here so the original
        upstream error reaches the caller.
        """
        try:
            loop.run(
                reservation.release_failure(handle, "upstream_failure"),
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
