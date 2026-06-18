# ruff: noqa: ANN001, ANN201, ANN401, S108
# Rationale: test fixtures use ``monkeypatch`` (Any), and ``/tmp`` paths
# are appropriate for unit tests that never write to disk.
"""COV_D11_S1 — ``SpendGuardGuardrail`` skeleton tests.

Tier 1 unit tests per ``docs/internal/slices/COV_D11_S1_guardrail_class.md``
test plan + ``docs/specs/coverage/D11_litellm_proxy_plugin/review-standards.md``
§Slice 1 reviewer checklist (1.1 - 1.5).

Anti-scope:
    * No actual reserve / commit / release wiring — that lands in
      SLICE 2 / 3, asserted by ``NotImplementedError`` here.
    * No LiteLLM proxy boot — we only need ``CustomGuardrail`` and the
      composition source from ``litellm.py`` importable. Environments
      without LiteLLM installed are cleanly skipped via
      ``pytest.importorskip``.
"""

from __future__ import annotations

import importlib
import inspect
import sys
from pathlib import Path

import pytest

# Skip the whole module cleanly when LiteLLM (and therefore the
# ``CustomGuardrail`` base class) is not installed. The dedicated
# missing-extra test in ``test_litellm_missing_extra.py`` covers the
# fail-closed ImportError shape for the legacy callback module — the
# guardrail module mirrors that shape (U01).
pytest.importorskip(
    "litellm.integrations.custom_guardrail",
    reason="LiteLLM with guardrail support not installed; "
    "install spendguard-sdk[litellm-guardrail]",
)

from litellm.integrations.custom_guardrail import CustomGuardrail  # noqa: E402
from litellm.integrations.custom_logger import CustomLogger  # noqa: E402

from spendguard.integrations.litellm import _LoopBoundCallback  # noqa: E402
from spendguard.integrations.litellm_guardrail import SpendGuardGuardrail  # noqa: E402

# ---------------------------------------------------------------------------
# Construction + composition shape (reviewer checks 1.2, 1.3)
# ---------------------------------------------------------------------------


def test_construct_with_explicit_guardrail_name():
    """``__init__`` accepts ``guardrail_name`` kwarg and forwards it
    into ``CustomGuardrail.__init__`` (reviewer check 1.2)."""
    g = SpendGuardGuardrail(guardrail_name="test")
    # ``CustomGuardrail`` stores guardrail_name on the instance for
    # later registry lookup; verify the super call ran.
    assert g.guardrail_name == "test"


def test_construct_defaults_to_spendguard_name():
    """Default name matches the SLICE 5 ``proxy_config.yaml`` snippet."""
    g = SpendGuardGuardrail()
    assert g.guardrail_name == "spendguard"


def test_construct_subclasses_custom_guardrail():
    """Composition wrapper itself extends ``CustomGuardrail`` (the
    LiteLLM proxy registry discovers guardrails by ``isinstance`` check
    in upstream 1.55+)."""
    g = SpendGuardGuardrail(guardrail_name="test")
    assert isinstance(g, CustomGuardrail)


def test_delegate_is_loop_bound_callback_instance():
    """Reviewer check 1.3 (Blocker): ``_delegate`` is a
    ``_LoopBoundCallback`` — composition, NOT inheritance.
    ``SpendGuardGuardrail`` MUST NOT subclass ``_LoopBoundCallback``
    or multiply-inherit ``CustomGuardrail`` + ``CustomLogger``."""
    g = SpendGuardGuardrail(guardrail_name="test")
    # 1. The delegate slot exists.
    assert hasattr(g, "_delegate"), (
        "SpendGuardGuardrail must hold a delegate attribute "
        "(composition shape per design.md §4)."
    )
    # 2. The delegate is the exact ``_LoopBoundCallback`` re-used from
    #    the legacy callback module.
    assert isinstance(g._delegate, _LoopBoundCallback)
    # 3. The wrapper does NOT itself inherit ``_LoopBoundCallback``.
    assert not isinstance(g, _LoopBoundCallback)
    # 4. MRO contains ``CustomGuardrail`` (and its ``CustomLogger`` base
    #    transitively) but NOT ``_LoopBoundCallback``.
    mro = type(g).__mro__
    assert CustomGuardrail in mro
    assert _LoopBoundCallback not in mro
    # 5. Delegate lazy-loop state is not bound at construction —
    #    ``_LoopBoundCallback.__init__`` keeps ``_client`` ``None``
    #    until ``_ensure_client()`` runs on a serving loop.
    assert g._delegate._client is None


def test_construct_forwards_explicit_socket_path_and_tenant():
    """Explicit kwargs flow through to the delegate so SLICE 4's
    env-driven default factory can be swapped in without breaking the
    direct-instantiation path used by integration tests."""
    g = SpendGuardGuardrail(
        guardrail_name="test",
        socket_path="/tmp/unix-spendguard-skeleton.sock",
        tenant_id="tenant-skeleton",
    )
    assert g._delegate._socket_path == "/tmp/unix-spendguard-skeleton.sock"
    assert g._delegate._tenant_id == "tenant-skeleton"


# ---------------------------------------------------------------------------
# Hook method shape (slice doc test plan step 1, reviewer 2.x / 3.x deferred)
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "method_name",
    [
        "async_pre_call_hook",
        "async_post_call_success_hook",
        "async_post_call_failure_hook",
    ],
)
def test_hook_methods_are_coroutines(method_name: str):
    """Three hook methods exist as coroutine functions. We do NOT call
    them — SLICE 1 stubs raise ``NotImplementedError`` per the slice
    doc; the actual reserve / commit / release wiring lands in
    SLICE 2 / 3."""
    g = SpendGuardGuardrail(guardrail_name="test")
    method = getattr(g, method_name, None)
    assert method is not None, (
        f"SpendGuardGuardrail must expose {method_name!r}"
    )
    assert inspect.iscoroutinefunction(method), (
        f"{method_name} must be an async def coroutine function "
        f"(matches CustomGuardrail's async hook contract)"
    )
    # SLICE 1 contract: the hook is defined on the wrapper class itself
    # (overriding the base no-op), not silently inherited from
    # ``CustomGuardrail``. SLICE 2 / 3 replace these bodies in place.
    assert method_name in SpendGuardGuardrail.__dict__, (
        f"{method_name} must be defined directly on "
        "SpendGuardGuardrail (slice doc: stubs that raise "
        "NotImplementedError)."
    )


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("method_name", "args"),
    [
        # COV_D11_S2 (2026-06-06) wired async_pre_call_hook → delegate.
        # COV_D11_S3 (2026-06-07) wired both post-call hooks → delegate.
        # All three hook delegation contracts are now pinned by the
        # dedicated SLICE 2 / SLICE 3 test files:
        #   * test_litellm_guardrail_pre_call.py — SLICE 2 wiring
        #   * test_litellm_guardrail_post_call.py — SLICE 3 wiring
        # Empty parametrize keeps the parametrize-shape regression visible
        # but produces zero collected cases (pytest skips parametrized
        # functions with no cases via the `--collect-only` reporter).
    ],
)
async def test_hook_methods_raise_not_implemented(method_name, args):
    """Historical regression scaffold: all SLICE 1 ``NotImplementedError``
    hook stubs have been wired (pre-call in SLICE 2; success + failure
    post-call in SLICE 3). The body is preserved as a no-op so the
    `not_implemented` invariant trips loudly if anyone re-stubs a hook
    via a drive-by refactor — and the parametrize list above documents
    where the live delegation tests live.

    Intentionally empty parametrize → zero collected cases. If a future
    slice adds a new ``NotImplementedError`` stub, append a row here.
    """
    g = SpendGuardGuardrail(guardrail_name="test")
    method = getattr(g, method_name)
    with pytest.raises(NotImplementedError) as exc_info:
        await method(*args)
    msg = str(exc_info.value)
    assert "COV_D11" in msg, (
        f"NotImplementedError must reference wiring slice; got: {msg!r}"
    )


# ---------------------------------------------------------------------------
# Reviewer check 1.4 — no module-level mutable state beyond logger
# ---------------------------------------------------------------------------


def _module_source() -> str:
    return Path(
        importlib.import_module(
            "spendguard.integrations.litellm_guardrail"
        ).__file__
    ).read_text(encoding="utf-8")


def test_no_module_level_singleton_or_default_instance():
    """Reviewer check 1.4 (Major): the module must not pre-construct
    a singleton ``SpendGuardGuardrail`` or attach mutable state to a
    module-level name beyond the logger. An import-time singleton
    would force loop binding before LiteLLM's ASGI loop spins up
    (Round 3 P0.3 in ``test_litellm_skeleton.py`` regression context)."""
    import spendguard.integrations.litellm_guardrail as mod

    # No module-level instance of the class itself.
    for attr_name, attr_value in vars(mod).items():
        if attr_name.startswith("_"):
            continue
        if attr_name == "SpendGuardGuardrail":
            continue
        assert not isinstance(attr_value, SpendGuardGuardrail), (
            f"Module-level instance forbidden: {attr_name}"
        )

    # The logger is the only mutable module-level name we accept,
    # mirroring ``litellm.py``'s ``log`` convention.
    # R2 fix per R1 review M2: replaced tautological ``isinstance(mod.log,
    # type(mod.log))`` (vacuously true for any non-None value) with explicit
    # ``logging.Logger`` check so renaming or dropping the logger surfaces here.
    import logging as _logging
    assert isinstance(mod.log, _logging.Logger), "log must remain a logging.Logger"


# ---------------------------------------------------------------------------
# Reviewer check 1.5 — ImportError shape covered (U01)
# ---------------------------------------------------------------------------


def test_import_error_message_when_custom_guardrail_missing(monkeypatch):
    """Reviewer check 1.5: missing ``CustomGuardrail`` base surfaces a
    SpendGuard-shaped install hint, not a raw ``ModuleNotFoundError``.

    Simulates the upgrade scenario where the operator installed
    ``spendguard-sdk[litellm]`` (which targets the legacy 1.50 floor)
    but has not yet bumped to the 1.55 guardrail surface.
    """
    for k in list(sys.modules):
        if (
            k == "litellm.integrations.custom_guardrail"
            or k == "spendguard.integrations.litellm_guardrail"
        ):
            monkeypatch.delitem(sys.modules, k, raising=False)
    # Block ``CustomGuardrail`` import without breaking the rest of
    # the LiteLLM tree (we still need ``_LoopBoundCallback`` to import
    # cleanly during the failure path).
    monkeypatch.setitem(
        sys.modules, "litellm.integrations.custom_guardrail", None,
    )
    with pytest.raises(ImportError) as exc_info:
        importlib.import_module("spendguard.integrations.litellm_guardrail")
    msg = str(exc_info.value)
    assert "spendguard-sdk[litellm-guardrail]" in msg, (
        f"ImportError must reference the extra name; got: {msg!r}"
    )


def test_module_uses_lazy_custom_guardrail_import_pattern():
    """Reviewer check 1.1 (Blocker): ``CustomGuardrail`` is imported
    inside a ``try / except ImportError`` block with the install hint
    message. Source-level scan keeps the regression bar high — a
    future drive-by refactor that hoists the import out of the try
    block would fail this test."""
    src = _module_source()
    assert "from litellm.integrations.custom_guardrail import CustomGuardrail" in src
    assert "except ImportError" in src
    # The install hint must mention the dedicated guardrail extra so
    # operators don't misdiagnose as missing the legacy ``[litellm]``
    # extra (which has a lower floor).
    assert "'spendguard-sdk[litellm-guardrail]'" in src


# ---------------------------------------------------------------------------
# Slice-doc acceptance gate 3: ``from spendguard.integrations import …``
# ---------------------------------------------------------------------------


def test_top_level_integrations_namespace_exposes_class():
    """Slice-doc test plan step 3: ``from spendguard.integrations
    import SpendGuardGuardrail`` must succeed. The lazy PEP 562
    ``__getattr__`` keeps the integrations namespace import lightweight
    while still surfacing the new D11 entry point."""
    import spendguard.integrations as integrations_pkg

    # Lazy attribute access through __getattr__ — using getattr() here
    # is load-bearing: it specifically exercises the PEP 562 hook path
    # instead of relying on a module-level binding that direct attribute
    # access would expose.
    cls = getattr(integrations_pkg, "SpendGuardGuardrail")  # noqa: B009
    assert cls is SpendGuardGuardrail
    # ``dir()`` advertises it so IDEs / repl tab-complete reaches it.
    assert "SpendGuardGuardrail" in dir(integrations_pkg)


def test_legacy_callback_export_still_works():
    """Slice-doc test plan step 4: the legacy callback path stays
    importable. SLICE 1 must NOT mutate ``litellm.py`` or its public
    surface (``implementation.md`` §3 backwards compat)."""
    from spendguard.integrations.litellm import SpendGuardLiteLLMCallback

    # Sanity: still subclasses ``CustomLogger`` so existing
    # ``litellm_settings.callbacks: [...]`` registration paths work.
    assert issubclass(SpendGuardLiteLLMCallback, CustomLogger)
