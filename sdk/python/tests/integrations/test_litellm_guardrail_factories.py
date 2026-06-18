# ruff: noqa: ANN001, ANN201, ANN202, ANN401, S105, S106, S108
# Rationale: test fixtures use ``monkeypatch`` (Any) + non-secret literal
# tokens; the test never speaks to an actual sidecar / LLM provider.
# ``/tmp`` paths are unit-test sentinels that never touch disk — the
# UDS path is forwarded into the delegate's stash and inspected as a
# string, not connected to.
"""COV_D11_S4 — ``SpendGuardGuardrail`` env-driven factory tests.

Tier 1 unit tests per ``docs/internal/slices/COV_D11_S4_env_defaults.md`` test
plan + ``docs/specs/coverage/D11_litellm_proxy_plugin/review-standards.md``
§Slice 4 reviewer checklist (4.1 - 4.7, scope-cut subset).

Coverage:
    * ``from_env`` happy path with required vars set.
    * ``from_env`` raises ``SpendGuardConfigError`` naming each
      missing required var (review-standards 4.1 Blocker).
    * ``SPENDGUARD_DISABLED`` parses truthy variants → no-op delegate
      installed; hooks short-circuit without touching the sidecar.
    * ``SPENDGUARD_PROXY_TIMEOUT_MS`` parses / fails on invalid.
    * ``from_kwargs`` passes through to ``__init__`` and ignores env.
    * ``from_config`` accepts the dict shape SLICE 5 yaml will emit.
    * ``from_env`` is non-singleton (separate instances per call).
    * All three hooks delegate after env construction (sanity tying
      SLICE 4's factory to the SLICE 1-3 hook delegation contract).

Anti-scope:
    * No ``proxy_config.yaml`` parse — SLICE 5.
    * No demo / no real sidecar / no LiteLLM proxy boot.
"""

from __future__ import annotations

from typing import Any
from unittest.mock import AsyncMock

import pytest

# Skip cleanly when LiteLLM (and therefore ``CustomGuardrail``) is
# missing, matching the SLICE 1-3 importorskip pattern.
pytest.importorskip(
    "litellm.integrations.custom_guardrail",
    reason="LiteLLM with guardrail support not installed; "
    "install spendguard-sdk[litellm-guardrail]",
)

from spendguard.errors import SpendGuardConfigError  # noqa: E402
from spendguard.integrations.litellm import _LoopBoundCallback  # noqa: E402
from spendguard.integrations.litellm_guardrail import (  # noqa: E402
    SpendGuardGuardrail,
    _NoopGuardrailDelegate,
)

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def clean_env(monkeypatch):
    """Strip every ``SPENDGUARD_*`` var from the test environment so
    each test sets exactly what it needs. Avoids inheriting host
    config that would mask a missing-var assertion.
    """
    import os
    for k in [k for k in os.environ if k.startswith("SPENDGUARD_")]:
        monkeypatch.delenv(k, raising=False)
    return monkeypatch


# ---------------------------------------------------------------------------
# Happy path
# ---------------------------------------------------------------------------


def test_from_env_success(clean_env):
    """Required env vars set → constructed guardrail forwards
    ``tenant_id`` and ``sidecar_address`` into the underlying
    delegate (review-standards 1.3 composition shape preserved)."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-from-env")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/sg-env.sock")

    g = SpendGuardGuardrail.from_env()

    assert isinstance(g, SpendGuardGuardrail)
    assert isinstance(g._delegate, _LoopBoundCallback)
    assert g._delegate._tenant_id == "tenant-from-env"
    assert g._delegate._socket_path == "unix:///tmp/sg-env.sock"
    # Lazy loop binding preserved — no client at construction.
    assert g._delegate._client is None


def test_from_env_legacy_sidecar_uds_alias(clean_env):
    """``SPENDGUARD_SIDECAR_UDS`` is the legacy alias for
    ``SPENDGUARD_SIDECAR_ADDRESS`` (matches the existing
    ``examples/litellm-proxy-composite`` deployment). When only the
    legacy var is set, it must be honoured so existing operators do
    not regress on the rename."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-legacy")
    clean_env.setenv("SPENDGUARD_SIDECAR_UDS", "/run/spendguard/legacy.sock")

    g = SpendGuardGuardrail.from_env()

    assert g._delegate._socket_path == "/run/spendguard/legacy.sock"


def test_from_env_address_wins_over_legacy_uds(clean_env):
    """When both are set, ``SPENDGUARD_SIDECAR_ADDRESS`` (the
    canonical SLICE 4 spelling) takes precedence. Operators
    migrating from the legacy var can set both during the migration
    window without surprises."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-both")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///new/addr.sock")
    clean_env.setenv("SPENDGUARD_SIDECAR_UDS", "/old/legacy.sock")

    g = SpendGuardGuardrail.from_env()

    assert g._delegate._socket_path == "unix:///new/addr.sock"


# ---------------------------------------------------------------------------
# Missing-required env vars → SpendGuardConfigError (review-standards 4.1)
# ---------------------------------------------------------------------------


def test_from_env_missing_tenant_id_raises(clean_env):
    """Unset ``SPENDGUARD_TENANT_ID`` → ``SpendGuardConfigError``
    naming the var (review-standards 4.1 Blocker)."""
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/x.sock")
    # Explicitly leave SPENDGUARD_TENANT_ID unset.

    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_env()

    msg = str(exc_info.value)
    assert "SPENDGUARD_TENANT_ID" in msg, (
        f"error message must name the missing var; got: {msg!r}"
    )


def test_from_env_empty_tenant_id_raises(clean_env):
    """Empty-string ``SPENDGUARD_TENANT_ID`` is treated as missing —
    operators must not silently pass through an empty value (the
    legacy callback path required a non-empty tenant via
    ``SpendGuardClient(... tenant_id=...)`` ``ValueError`` check at
    ``client.py:201``)."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "   ")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/x.sock")

    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_env()

    assert "SPENDGUARD_TENANT_ID" in str(exc_info.value)


def test_from_env_missing_sidecar_address_raises(clean_env):
    """Neither ``SPENDGUARD_SIDECAR_ADDRESS`` nor the legacy
    ``SPENDGUARD_SIDECAR_UDS`` set → ``SpendGuardConfigError``
    naming the canonical SLICE 4 var (review-standards 4.1
    Blocker)."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-test")
    # Explicitly leave both sidecar vars unset.

    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_env()

    msg = str(exc_info.value)
    assert "SPENDGUARD_SIDECAR_ADDRESS" in msg


# ---------------------------------------------------------------------------
# SPENDGUARD_DISABLED — truthy / falsy / no-op delegate
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "raw",
    ["true", "True", "TRUE", "1", "yes", "YES", "on", "ON"],
)
def test_from_env_disabled_parses_truthy_variants(clean_env, raw):
    """Each truthy variant → no-op delegate installed. The exact set
    is documented on ``_parse_disabled`` so adapter authors can rely
    on case-insensitive ``true``/``1``/``yes``/``on`` without yaml
    boolean surprises."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-d")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/d.sock")
    clean_env.setenv("SPENDGUARD_DISABLED", raw)

    g = SpendGuardGuardrail.from_env()

    assert isinstance(g._delegate, _NoopGuardrailDelegate), (
        f"truthy SPENDGUARD_DISABLED={raw!r} must install the no-op delegate"
    )
    assert g._config_disabled is True


@pytest.mark.parametrize(
    "raw",
    ["false", "False", "0", "no", "off", "", "  "],
)
def test_from_env_disabled_parses_falsy_variants(clean_env, raw):
    """Each falsy variant → standard ``_LoopBoundCallback`` delegate
    installed. Empty / whitespace strings default to disabled=False
    so an empty deployment-env var does not surprise-disable the
    guardrail."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-f")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/f.sock")
    clean_env.setenv("SPENDGUARD_DISABLED", raw)

    g = SpendGuardGuardrail.from_env()

    assert isinstance(g._delegate, _LoopBoundCallback)
    assert g._config_disabled is False


@pytest.mark.asyncio
async def test_from_env_disabled_hooks_short_circuit(clean_env):
    """Disabled-mode guardrail: all three hooks return without
    raising and without touching any sidecar IO. We assert by
    checking the delegate type — the no-op delegate's methods are
    coroutines that return None immediately."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-d2")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/d2.sock")
    clean_env.setenv("SPENDGUARD_DISABLED", "true")

    g = SpendGuardGuardrail.from_env()

    assert isinstance(g._delegate, _NoopGuardrailDelegate)

    # Pre-call hook: returns None (delegate's coroutine returns None;
    # the wrapper forwards the value verbatim per review-standards
    # 2.3 no-mutation invariant).
    pre_result = await g.async_pre_call_hook(
        user_api_key_dict=None, cache=None, data={}, call_type="completion",
    )
    assert pre_result is None

    # Success hook: returns None.
    success_result = await g.async_post_call_success_hook(
        data={"litellm_call_id": "id-1"},
        user_api_key_dict=None,
        response=None,
    )
    assert success_result is None

    # Failure hook: re-raises original exception per the SLICE 3
    # contract (review-standards 3.x propagation invariant carries
    # through even in disabled mode).
    err = RuntimeError("provider HTTP 500")
    with pytest.raises(RuntimeError) as exc_info:
        await g.async_post_call_failure_hook(
            request_data={"litellm_call_id": "id-1"},
            original_exception=err,
            user_api_key_dict=None,
        )
    assert exc_info.value is err


# ---------------------------------------------------------------------------
# SPENDGUARD_PROXY_TIMEOUT_MS — int parsing
# ---------------------------------------------------------------------------


def test_from_env_proxy_timeout_ms_parses(clean_env):
    """Integer string → captured on the instance's config stash so
    SLICE 5's bootstrap validator can read what was applied."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-t")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/t.sock")
    clean_env.setenv("SPENDGUARD_PROXY_TIMEOUT_MS", "10000")

    g = SpendGuardGuardrail.from_env()

    assert g._config_proxy_timeout_ms == 10000


def test_from_env_proxy_timeout_ms_defaults_to_5000(clean_env):
    """Unset → 5000 ms default. Captured as int so SLICE 5's
    validator does not need to coerce."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-t2")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/t2.sock")

    g = SpendGuardGuardrail.from_env()

    assert g._config_proxy_timeout_ms == 5000
    assert isinstance(g._config_proxy_timeout_ms, int)


def test_from_env_proxy_timeout_ms_invalid_raises(clean_env):
    """Non-integer ``SPENDGUARD_PROXY_TIMEOUT_MS`` → loud config
    error naming the var (review-standards "error messages name the
    offending env var" cross-cutting check)."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-bad")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/bad.sock")
    clean_env.setenv("SPENDGUARD_PROXY_TIMEOUT_MS", "notanumber")

    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_env()

    msg = str(exc_info.value)
    assert "SPENDGUARD_PROXY_TIMEOUT_MS" in msg
    assert "notanumber" in msg, (
        f"error must echo the bad value for ops debuggability; got: {msg!r}"
    )


# ---------------------------------------------------------------------------
# Optional SPENDGUARD_API_KEY surface
# ---------------------------------------------------------------------------


def test_from_env_api_key_captured(clean_env):
    """Optional ``SPENDGUARD_API_KEY`` is stashed for SLICE 5's
    bootstrap to inject into auth headers. None when unset."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-k")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/k.sock")
    clean_env.setenv("SPENDGUARD_API_KEY", "sk-test-not-real")

    g = SpendGuardGuardrail.from_env()

    assert g._config_api_key == "sk-test-not-real"


def test_from_env_api_key_default_none(clean_env):
    """Unset ``SPENDGUARD_API_KEY`` → None on the config stash."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-k2")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/k2.sock")

    g = SpendGuardGuardrail.from_env()

    assert g._config_api_key is None


# ---------------------------------------------------------------------------
# from_kwargs — explicit-kwargs constructor
# ---------------------------------------------------------------------------


def test_from_kwargs_passes_through(clean_env):
    """``from_kwargs`` delegates straight to ``__init__`` — the
    resulting guardrail must be indistinguishable from a direct
    ``SpendGuardGuardrail(**kwargs)`` call. We assert by checking
    the kwargs land on the delegate identically to SLICE 1's
    skeleton tests."""
    g = SpendGuardGuardrail.from_kwargs(
        guardrail_name="kw-test",
        socket_path="/tmp/kw.sock",
        tenant_id="tenant-kw",
    )

    assert g.guardrail_name == "kw-test"
    assert g._delegate._socket_path == "/tmp/kw.sock"
    assert g._delegate._tenant_id == "tenant-kw"


def test_from_kwargs_ignores_env(clean_env):
    """Even when env vars are set, ``from_kwargs`` does not consult
    them — kwargs are authoritative. Operators using ``from_kwargs``
    from a dict (e.g. parsed config) get deterministic behaviour
    regardless of stray host env vars."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "env-tenant-should-not-win")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "/env/should/not/win.sock")

    g = SpendGuardGuardrail.from_kwargs(
        guardrail_name="kw-only",
        socket_path="/kw/wins.sock",
        tenant_id="kw-wins",
    )

    assert g._delegate._tenant_id == "kw-wins"
    assert g._delegate._socket_path == "/kw/wins.sock"


# ---------------------------------------------------------------------------
# from_config — dict shape (SLICE 5 proxy_config.yaml prep)
# ---------------------------------------------------------------------------


def test_from_config_accepts_dict(clean_env):
    """``from_config`` accepts the dict shape SLICE 5's
    ``proxy_config.yaml`` parser will produce. Required keys
    (``tenant_id`` + ``sidecar_address``) land on the delegate;
    optional keys (``api_key`` / ``disabled`` / ``proxy_timeout_ms``)
    land on the config stash."""
    g = SpendGuardGuardrail.from_config({
        "tenant_id": "cfg-tenant",
        "sidecar_address": "unix:///tmp/cfg.sock",
        "api_key": "cfg-key",
        "disabled": False,
        "proxy_timeout_ms": 7500,
    })

    assert g._delegate._tenant_id == "cfg-tenant"
    assert g._delegate._socket_path == "unix:///tmp/cfg.sock"
    assert g._config_api_key == "cfg-key"
    assert g._config_disabled is False
    assert g._config_proxy_timeout_ms == 7500


def test_from_config_legacy_socket_path_alias(clean_env):
    """``socket_path`` is honoured as a legacy alias for
    ``sidecar_address`` so config produced by older yaml schemas
    continues to work."""
    g = SpendGuardGuardrail.from_config({
        "tenant_id": "cfg-legacy",
        "socket_path": "/legacy/cfg.sock",
    })

    assert g._delegate._socket_path == "/legacy/cfg.sock"


def test_from_config_disabled_bool_honoured(clean_env):
    """yaml booleans hit ``from_config`` as native Python ``True``
    — verify the bool path bypasses the string parser."""
    g = SpendGuardGuardrail.from_config({
        "tenant_id": "cfg-d",
        "sidecar_address": "/tmp/cfg-d.sock",
        "disabled": True,
    })

    assert isinstance(g._delegate, _NoopGuardrailDelegate)
    assert g._config_disabled is True


def test_from_config_missing_tenant_raises(clean_env):
    """Missing required ``tenant_id`` → ``SpendGuardConfigError``
    naming the key (mirrors the env-var error path)."""
    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_config({
            "sidecar_address": "/tmp/x.sock",
        })

    assert "tenant_id" in str(exc_info.value)


def test_from_config_missing_sidecar_raises(clean_env):
    """Missing required ``sidecar_address`` (and no legacy alias) →
    ``SpendGuardConfigError``."""
    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_config({
            "tenant_id": "cfg-t",
        })

    assert "sidecar_address" in str(exc_info.value)


def test_from_config_invalid_timeout_raises(clean_env):
    """Non-int ``proxy_timeout_ms`` → loud config error echoing the
    bad value, matching the env-var failure mode."""
    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_config({
            "tenant_id": "cfg-t",
            "sidecar_address": "/tmp/cfg.sock",
            "proxy_timeout_ms": "bad",
        })

    assert "proxy_timeout_ms" in str(exc_info.value)


def test_from_config_non_dict_raises(clean_env):
    """Defensive: non-dict caller payload surfaces a typed error
    instead of an opaque ``AttributeError``."""
    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_config("not a dict")  # type: ignore[arg-type]

    assert "dict" in str(exc_info.value)


# ---------------------------------------------------------------------------
# Non-singleton + hook delegation sanity
# ---------------------------------------------------------------------------


def test_from_env_creates_separate_instances(clean_env):
    """``from_env`` called twice returns two distinct guardrail
    objects with two distinct underlying delegates — no module-level
    singleton (carries over review-standards 1.4 "no module-level
    mutable state" invariant into SLICE 4)."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-s")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/s.sock")

    g1 = SpendGuardGuardrail.from_env()
    g2 = SpendGuardGuardrail.from_env()

    assert g1 is not g2
    assert g1._delegate is not g2._delegate


@pytest.mark.asyncio
async def test_all_three_hooks_delegate_after_env_construction(clean_env):
    """After ``from_env`` constructs a (non-disabled) guardrail,
    every hook still routes through the delegate. We swap the
    delegate with an ``AsyncMock`` post-construction and assert
    each hook awaits the corresponding delegate method — closes
    the loop between SLICE 4 (factory) and SLICE 1-3 (hook
    delegation contract)."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-h")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/h.sock")

    g = SpendGuardGuardrail.from_env()

    # The factory wired the real _LoopBoundCallback; replace it with
    # an AsyncMock so the test does not touch the gRPC layer.
    mock_delegate = AsyncMock()
    mock_delegate.async_pre_call_hook.return_value = {"forwarded": True}
    mock_delegate.async_log_success_event.return_value = None
    mock_delegate.async_log_failure_event.return_value = None
    g._delegate = mock_delegate  # type: ignore[assignment]

    # async_pre_call_hook → delegate.async_pre_call_hook.
    pre_result = await g.async_pre_call_hook(
        user_api_key_dict={"team": "t1"},
        cache=None,
        data={"litellm_call_id": "id-h-1", "model": "gpt-4o-mini"},
        call_type="completion",
    )
    assert pre_result == {"forwarded": True}
    mock_delegate.async_pre_call_hook.assert_awaited_once()

    # async_post_call_success_hook → delegate.async_log_success_event.
    await g.async_post_call_success_hook(
        data={"litellm_call_id": "id-h-1"},
        user_api_key_dict={"team": "t1"},
        response=None,
    )
    mock_delegate.async_log_success_event.assert_awaited_once()

    # async_post_call_failure_hook → delegate.async_log_failure_event.
    err = RuntimeError("simulated provider error")
    with pytest.raises(RuntimeError) as exc_info:
        await g.async_post_call_failure_hook(
            request_data={"litellm_call_id": "id-h-1"},
            original_exception=err,
            user_api_key_dict={"team": "t1"},
        )
    assert exc_info.value is err
    mock_delegate.async_log_failure_event.assert_awaited_once()


# ---------------------------------------------------------------------------
# Naming-clarity test (review-standards "do not let an English summary
# read as if env precedence is configurable on from_env")
# ---------------------------------------------------------------------------


def test_from_kwargs_does_not_read_env_when_env_is_set(clean_env):
    """Explicit naming for the kwargs-vs-env contract: ``from_kwargs``
    NEVER reads env. The companion ``from_env`` factory NEVER reads
    kwargs (no kwargs-overriding-env case exists; the two factories
    are deliberately separate). This test pins the contract for the
    review-standards 4.6 cross-cutting "no surprising precedence"
    check.
    """
    clean_env.setenv("SPENDGUARD_TENANT_ID", "should-be-ignored")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "/should/be/ignored.sock")

    g = SpendGuardGuardrail.from_kwargs(
        tenant_id="kw-wins",
        socket_path="/kw/wins.sock",
    )

    assert g._delegate._tenant_id == "kw-wins"
    assert g._delegate._socket_path == "/kw/wins.sock"


# ---------------------------------------------------------------------------
# Defensive: from_env with disabled also captures config stash
# ---------------------------------------------------------------------------


def test_from_env_disabled_still_stashes_config(clean_env):
    """Even in disabled mode the parsed config (api_key, timeout)
    must be visible on the instance — SLICE 5's bootstrap validator
    inspects these regardless of disabled state to surface
    misconfigurations early."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-dx")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/dx.sock")
    clean_env.setenv("SPENDGUARD_DISABLED", "yes")
    clean_env.setenv("SPENDGUARD_API_KEY", "key-dx")
    clean_env.setenv("SPENDGUARD_PROXY_TIMEOUT_MS", "3000")

    g = SpendGuardGuardrail.from_env()

    assert isinstance(g._delegate, _NoopGuardrailDelegate)
    assert g._config_api_key == "key-dx"
    assert g._config_disabled is True
    assert g._config_proxy_timeout_ms == 3000


# ===========================================================================
# SLICE 4b — resolver-module + single-tenant binding env-var wiring.
#
# Covers tests.md U06-U10 (deferred from SLICE 4 per the SLICE-PHASING
# note + 4 R2 doc-amendment) plus the budget-binding edge cases the
# slice doc calls out.
# ===========================================================================

# Importlib uses ``tests`` as the package root; pytest auto-adds it to
# sys.path when ``tests`` contains a directory with ``__init__.py``.
# We point ``SPENDGUARD_RESOLVER_MODULE`` at the fixture using the path
# relative to that root.
_FIXTURE_RESOLVER_SPEC = "integrations.fixtures.fake_resolver:make_triple"


@pytest.fixture
def fixture_resolver_on_path(monkeypatch):
    """Insert the SDK ``tests`` directory onto ``sys.path`` so the
    ``integrations.fixtures.fake_resolver`` module is importable via
    ``importlib.import_module``. Mirrors how the real LiteLLM proxy
    boots with ``PYTHONPATH=/path/to/operator/modules``.
    """
    import os
    import sys as _sys
    tests_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    monkeypatch.syspath_prepend(tests_dir)
    yield tests_dir
    # monkeypatch undoes syspath_prepend automatically; the fake_resolver
    # module may stay cached in sys.modules — purge it so each test sees
    # a fresh import (relevant for the "import is per-call" test).
    for mod_name in list(_sys.modules):
        if mod_name.startswith("integrations.fixtures.fake_resolver"):
            _sys.modules.pop(mod_name, None)


def _set_single_tenant_env(clean_env):
    """Helper: set the 3 budget-binding + 4 pricing-version env vars
    to deterministic non-fixture values so tests can distinguish the
    single-tenant default-resolver path from the operator-factory
    path (U08 invariant)."""
    clean_env.setenv("SPENDGUARD_BUDGET_ID", "env-budget")
    clean_env.setenv("SPENDGUARD_WINDOW_INSTANCE_ID", "env-window")
    clean_env.setenv("SPENDGUARD_UNIT_ID", "env-unit")
    clean_env.setenv("SPENDGUARD_PRICING_VERSION", "env-pricing-v1")
    clean_env.setenv("SPENDGUARD_FX_RATE_VERSION", "env-fx-v1")
    clean_env.setenv("SPENDGUARD_UNIT_CONVERSION_VERSION", "env-uc-v1")
    # Even-length hex required by `bytes.fromhex`.
    clean_env.setenv(
        "SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX",
        "deadbeef" * 8,  # 32-byte snapshot hash
    )


# ---------------------------------------------------------------------------
# U06 / U07 — single-tenant default resolver builds the binding from
# the 3 + 4 env vars.
# ---------------------------------------------------------------------------


def test_from_env_default_resolver_constructs_binding(clean_env):
    """U06: with all 8 env vars set, the loaded resolver returns a
    ``BudgetBinding`` whose ``budget_id`` / ``window_instance_id`` /
    ``unit.unit_id`` match the env values (review-standards 4.5
    Blocker: empty fields would fail-closed; non-empty fields land
    on the binding).
    """
    from spendguard.integrations.litellm import ResolverContext

    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-u06")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/u06.sock")
    _set_single_tenant_env(clean_env)

    g = SpendGuardGuardrail.from_env()

    # The wired delegate calls its resolver with a ResolverContext;
    # we invoke it directly with a stub context to exercise the
    # closure without booting the gRPC channel.
    ctx = ResolverContext(data={}, user_api_key_dict=None, call_type="completion")
    binding = g._delegate._budget_resolver(ctx)

    assert binding is not None
    assert binding.budget_id == "env-budget"
    assert binding.window_instance_id == "env-window"
    assert binding.unit.unit_id == "env-unit"


def test_from_env_default_resolver_loads_unit_ref_and_pricing(clean_env):
    """U07: ``BudgetBinding.unit.unit_id`` and
    ``BudgetBinding.pricing.pricing_version`` (+ fx, unit_conversion,
    snapshot hash) match env values field-by-field — mirror of the
    example_callback shape called out in review-standards 4.6
    Major."""
    from spendguard.integrations.litellm import ResolverContext

    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-u07")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/u07.sock")
    _set_single_tenant_env(clean_env)

    g = SpendGuardGuardrail.from_env()
    ctx = ResolverContext(data={}, user_api_key_dict=None, call_type="completion")
    binding = g._delegate._budget_resolver(ctx)

    assert binding.unit.unit_id == "env-unit"
    assert binding.unit.token_kind == "output_token"
    assert binding.unit.model_family == "gpt-4"
    assert binding.pricing.pricing_version == "env-pricing-v1"
    assert binding.pricing.fx_rate_version == "env-fx-v1"
    assert binding.pricing.unit_conversion_version == "env-uc-v1"
    # 32-byte snapshot hash decoded from the hex env var.
    assert binding.pricing.price_snapshot_hash == bytes.fromhex(
        "deadbeef" * 8,
    )


# ---------------------------------------------------------------------------
# U08 — SPENDGUARD_RESOLVER_MODULE dispatches to operator factory and
# ignores single-tenant env vars.
# ---------------------------------------------------------------------------


def test_resolver_module_env_imports_factory(
    clean_env, fixture_resolver_on_path,
):
    """U08: ``SPENDGUARD_RESOLVER_MODULE=...:make_triple`` imports +
    dispatches; the single-tenant env vars are NOT consulted
    (review-standards 4.3 Blocker).
    """
    from spendguard.integrations.litellm import ResolverContext
    from integrations.fixtures.fake_resolver import (
        FIXTURE_BUDGET_ID,
        FIXTURE_UNIT_ID,
        FIXTURE_WINDOW_ID,
    )

    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-u08")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/u08.sock")
    clean_env.setenv("SPENDGUARD_RESOLVER_MODULE", _FIXTURE_RESOLVER_SPEC)
    # Deliberately leave single-tenant vars unset to prove they are
    # NOT required when the resolver-module is set.

    g = SpendGuardGuardrail.from_env()
    ctx = ResolverContext(data={}, user_api_key_dict=None, call_type="completion")
    binding = g._delegate._budget_resolver(ctx)

    # The fixture's resolver returns FIXTURE_* sentinels, not env values
    # — proves the operator factory was dispatched to.
    assert binding.budget_id == FIXTURE_BUDGET_ID
    assert binding.window_instance_id == FIXTURE_WINDOW_ID
    assert binding.unit.unit_id == FIXTURE_UNIT_ID


# ---------------------------------------------------------------------------
# U09 / U10 — resolver-module bad path / missing attr / non-callable.
# ---------------------------------------------------------------------------


def test_resolver_module_bad_path_raises(clean_env):
    """U09: ``SPENDGUARD_RESOLVER_MODULE=nonexistent.module:bad`` →
    ``SpendGuardConfigError`` at boot (review-standards 4.2
    Blocker)."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-u09")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/u09.sock")
    clean_env.setenv(
        "SPENDGUARD_RESOLVER_MODULE",
        "definitely_not_a_real_package_xyzzy.module:bad",
    )

    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_env()

    msg = str(exc_info.value)
    assert "SPENDGUARD_RESOLVER_MODULE" in msg
    assert "definitely_not_a_real_package_xyzzy" in msg


def test_resolver_module_missing_attr_raises(
    clean_env, fixture_resolver_on_path,
):
    """U10: module imports but the named attribute is missing →
    ``SpendGuardConfigError`` (review-standards 4.2 Blocker)."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-u10")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/u10.sock")
    clean_env.setenv(
        "SPENDGUARD_RESOLVER_MODULE",
        "integrations.fixtures.fake_resolver:no_such_attr",
    )

    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_env()

    msg = str(exc_info.value)
    assert "SPENDGUARD_RESOLVER_MODULE" in msg
    assert "no_such_attr" in msg


def test_resolver_module_attr_not_callable_raises(
    clean_env, fixture_resolver_on_path,
):
    """Attribute exists but is not callable → typed config error
    naming the env var. Defensive check; the operator factory
    contract requires a zero-arg callable."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-nc")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/nc.sock")
    clean_env.setenv(
        "SPENDGUARD_RESOLVER_MODULE",
        "integrations.fixtures.fake_resolver:not_a_function",
    )

    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_env()

    assert "not callable" in str(exc_info.value)


def test_resolver_module_factory_returns_non_triple_raises(
    clean_env, fixture_resolver_on_path,
):
    """Factory exists and is callable but returns a non-triple →
    typed config error. Pins the contract documented in the docstring
    so silent shape drift cannot reach the hook layer."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-nt")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/nt.sock")
    clean_env.setenv(
        "SPENDGUARD_RESOLVER_MODULE",
        "integrations.fixtures.fake_resolver:not_callable",
    )

    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_env()

    assert "3-tuple" in str(exc_info.value)


def test_resolver_module_empty_string_raises(clean_env):
    """An empty / whitespace-only ``SPENDGUARD_RESOLVER_MODULE`` is
    treated as unset (the env reader strips); a value with no colon
    and no dot triggers the typed validation error."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-empty-rm")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/erm.sock")
    clean_env.setenv("SPENDGUARD_RESOLVER_MODULE", "no_separator_here")

    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_env()

    msg = str(exc_info.value)
    assert "SPENDGUARD_RESOLVER_MODULE" in msg


# ---------------------------------------------------------------------------
# Budget-binding partial-state validation (review-standards 4.5)
# ---------------------------------------------------------------------------


def test_from_env_budget_binding_partial_raises_config_error(clean_env):
    """Any subset of the 3 + 4 single-tenant vars set (but not all) →
    ``SpendGuardConfigError`` naming every missing var.
    Review-standards 4.5 Blocker: empty binding fields are fail-closed
    at construction time, mirroring ``litellm.py:306-315``.
    """
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-partial")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/partial.sock")
    # Only budget_id set — window/unit/pricing all missing.
    clean_env.setenv("SPENDGUARD_BUDGET_ID", "b1")

    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_env()

    msg = str(exc_info.value)
    assert "SPENDGUARD_WINDOW_INSTANCE_ID" in msg
    assert "SPENDGUARD_UNIT_ID" in msg
    assert "SPENDGUARD_PRICING_VERSION" in msg
    assert "SPENDGUARD_FX_RATE_VERSION" in msg
    assert "SPENDGUARD_UNIT_CONVERSION_VERSION" in msg
    assert "SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX" in msg


def test_from_env_budget_binding_all_unset_is_resolver_only(clean_env):
    """Legal: all 8 SLICE 4b vars unset → the SLICE 1 skeleton resolver
    stays put. Adapter authors who supply a resolver via
    ``from_kwargs`` or by replacing the delegate must not be
    penalised by SLICE 4b. SLICE 4 baseline tests rely on this.
    """
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-empty4b")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/e4b.sock")
    # Deliberately leave all 8 SLICE 4b vars unset.

    g = SpendGuardGuardrail.from_env()

    # Standard _LoopBoundCallback wired with the SLICE 1 sentinel
    # resolvers. The instance is constructable; first hook call
    # would surface the well-known "budget_resolver returned None"
    # error from `litellm.py:298-302` if not overridden — but that
    # surface is reachable only on hook invocation, not on construction.
    assert isinstance(g._delegate, _LoopBoundCallback)
    assert g._config_resolver_module is None
    assert g._config_budget_id is None


def test_from_env_invalid_price_snapshot_hash_raises(clean_env):
    """Non-hex ``SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX`` → typed config
    error naming the var. Hex decode failures are silent reads in the
    legacy path; SLICE 4b makes them loud at boot."""
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-badhex")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/bh.sock")
    clean_env.setenv("SPENDGUARD_BUDGET_ID", "b1")
    clean_env.setenv("SPENDGUARD_WINDOW_INSTANCE_ID", "w1")
    clean_env.setenv("SPENDGUARD_UNIT_ID", "u1")
    clean_env.setenv("SPENDGUARD_PRICING_VERSION", "v1")
    clean_env.setenv("SPENDGUARD_FX_RATE_VERSION", "fx1")
    clean_env.setenv("SPENDGUARD_UNIT_CONVERSION_VERSION", "uc1")
    clean_env.setenv(
        "SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX", "not-hex-content!",
    )

    with pytest.raises(SpendGuardConfigError) as exc_info:
        SpendGuardGuardrail.from_env()

    assert "SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX" in str(exc_info.value)


# ---------------------------------------------------------------------------
# Pricing-version propagation (review-standards 4.6 Major)
# ---------------------------------------------------------------------------


def test_from_env_pricing_version_vars_propagate_to_binding(clean_env):
    """Each of the 4 pricing-version env vars lands on the
    ``BudgetBinding.pricing`` ``PricingFreeze``. Pinned per-field so
    a future refactor that drops one var fails noisily.
    """
    from spendguard.integrations.litellm import ResolverContext

    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-pv")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/pv.sock")
    clean_env.setenv("SPENDGUARD_BUDGET_ID", "pv-budget")
    clean_env.setenv("SPENDGUARD_WINDOW_INSTANCE_ID", "pv-window")
    clean_env.setenv("SPENDGUARD_UNIT_ID", "pv-unit")
    clean_env.setenv("SPENDGUARD_PRICING_VERSION", "v2-pricing")
    clean_env.setenv("SPENDGUARD_FX_RATE_VERSION", "v3-fx")
    clean_env.setenv("SPENDGUARD_UNIT_CONVERSION_VERSION", "v4-uc")
    clean_env.setenv(
        "SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX",
        "0011223344556677889900aabbccddeeff" + "00" * 15,
    )

    g = SpendGuardGuardrail.from_env()
    ctx = ResolverContext(data={}, user_api_key_dict=None, call_type="completion")
    binding = g._delegate._budget_resolver(ctx)
    pricing = binding.pricing

    assert pricing.pricing_version == "v2-pricing"
    assert pricing.fx_rate_version == "v3-fx"
    assert pricing.unit_conversion_version == "v4-uc"
    assert pricing.price_snapshot_hash == bytes.fromhex(
        "0011223344556677889900aabbccddeeff" + "00" * 15,
    )


# ---------------------------------------------------------------------------
# Smoke: from_env + resolver wiring yields a hook-callable instance.
# ---------------------------------------------------------------------------


def test_from_env_resolver_only_path_hook_invocation_works(clean_env):
    """Smoke test: ``from_env`` with the single-tenant default
    resolver path produces a guardrail whose ``_budget_resolver`` is
    callable and returns a non-None binding — meaning the
    pre-call hook would NOT raise ``budget_resolver returned None``
    from ``litellm.py:298-302`` on first invocation.

    Without booting the gRPC channel, we directly verify the
    resolver-callable invariant; the broader pre-call hook is
    exercised under U12-U14 in the skeleton suite. SLICE 4b's
    correctness claim is that ``from_env`` produces an instance
    that passes the L298-L302 gate, not that the gRPC stack is
    wired (the existing tests cover that).
    """
    from spendguard.integrations.litellm import ResolverContext

    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-smoke")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/smoke.sock")
    _set_single_tenant_env(clean_env)

    g = SpendGuardGuardrail.from_env()

    # The delegate is a real `_LoopBoundCallback` (not the no-op).
    assert isinstance(g._delegate, _LoopBoundCallback)

    # The wired resolver returns a non-None binding for any context.
    ctx = ResolverContext(data={}, user_api_key_dict=None, call_type="completion")
    binding = g._delegate._budget_resolver(ctx)
    assert binding is not None
    assert binding.budget_id == "env-budget"
    assert binding.window_instance_id == "env-window"

    # The wired reconciler returns a single-element claim list with a
    # token-count-derived amount (max(tokens, 1)).
    class _FakeUsage:
        completion_tokens = 17

    class _FakeResponse:
        usage = _FakeUsage()

    claims = g._delegate._claim_reconciler(ctx, _FakeResponse())
    assert len(claims) == 1
    assert claims[0].amount_atomic == "17"
    assert claims[0].budget_id == "env-budget"
    assert claims[0].window_instance_id == "env-window"


def test_default_reconciler_uses_min_one_when_tokens_zero(clean_env):
    """Reconciler floor: ``max(tokens, 1)`` keeps the commit row
    non-empty so the stats aggregator never reads a zero-amount
    commit as a missing commit. Mirrors the example callback's
    ``_reconcile`` behaviour.
    """
    from spendguard.integrations.litellm import ResolverContext

    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-zero")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/zero.sock")
    _set_single_tenant_env(clean_env)

    g = SpendGuardGuardrail.from_env()

    class _ZeroResp:
        usage = type("U", (), {"completion_tokens": 0})()

    ctx = ResolverContext(data={}, user_api_key_dict=None, call_type="completion")
    claims = g._delegate._claim_reconciler(ctx, _ZeroResp())
    assert claims[0].amount_atomic == "1"


# ---------------------------------------------------------------------------
# from_kwargs / from_config precedence + dict shape.
# ---------------------------------------------------------------------------


def test_from_kwargs_resolver_kwarg_overrides_env(
    clean_env, fixture_resolver_on_path,
):
    """Explicit ``budget_resolver=`` kwarg wins over any env var
    settings. Pin the SLICE 4 "kwargs are authoritative" contract
    extends to the SLICE 4b resolver-wiring surface.
    """
    clean_env.setenv("SPENDGUARD_TENANT_ID", "env-tenant")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/env.sock")
    clean_env.setenv("SPENDGUARD_RESOLVER_MODULE", _FIXTURE_RESOLVER_SPEC)
    _set_single_tenant_env(clean_env)

    sentinel_binding = object()

    def _kwarg_resolver(_ctx):
        return sentinel_binding

    g = SpendGuardGuardrail.from_kwargs(
        socket_path="/kw/wins.sock",
        tenant_id="kw-wins",
        budget_resolver=_kwarg_resolver,
        claim_reconciler=lambda _ctx, _r: [],
    )

    # `from_kwargs` does NOT consult env; the kwarg resolver flows
    # directly into the delegate.
    assert g._delegate._budget_resolver is _kwarg_resolver
    assert g._delegate._budget_resolver(None) is sentinel_binding


def test_from_config_dict_resolver_key(clean_env, fixture_resolver_on_path):
    """``from_config`` accepts the SLICE 5 yaml shape with a
    ``resolver_module`` key. Same dispatch path as
    ``SPENDGUARD_RESOLVER_MODULE`` so the SLICE 5 yaml loader can
    forward the parsed value verbatim.
    """
    from spendguard.integrations.litellm import ResolverContext
    from integrations.fixtures.fake_resolver import (
        FIXTURE_BUDGET_ID,
    )

    g = SpendGuardGuardrail.from_config({
        "tenant_id": "cfg-rm",
        "sidecar_address": "unix:///tmp/cfg-rm.sock",
        "resolver_module": _FIXTURE_RESOLVER_SPEC,
    })

    assert g._config_resolver_module == _FIXTURE_RESOLVER_SPEC
    ctx = ResolverContext(data={}, user_api_key_dict=None, call_type="completion")
    binding = g._delegate._budget_resolver(ctx)
    assert binding.budget_id == FIXTURE_BUDGET_ID


def test_from_config_dict_single_tenant_keys(clean_env):
    """``from_config`` accepts the 3 + 4 single-tenant keys
    field-by-field (SLICE 5 yaml shape mirror of the env vars).
    """
    from spendguard.integrations.litellm import ResolverContext

    g = SpendGuardGuardrail.from_config({
        "tenant_id": "cfg-st",
        "sidecar_address": "unix:///tmp/cfg-st.sock",
        "budget_id": "cfg-budget",
        "window_instance_id": "cfg-window",
        "unit_id": "cfg-unit",
        "pricing_version": "cfg-pricing-v1",
        "fx_rate_version": "cfg-fx-v1",
        "unit_conversion_version": "cfg-uc-v1",
        "price_snapshot_hash_hex": "ab" * 32,
    })

    ctx = ResolverContext(data={}, user_api_key_dict=None, call_type="completion")
    binding = g._delegate._budget_resolver(ctx)
    assert binding.budget_id == "cfg-budget"
    assert binding.window_instance_id == "cfg-window"
    assert binding.unit.unit_id == "cfg-unit"
    assert binding.pricing.pricing_version == "cfg-pricing-v1"


def test_load_resolver_triple_supports_legacy_dot_syntax(
    clean_env, fixture_resolver_on_path,
):
    """The task prompt's smoke-test spelling uses dot-only syntax
    (``pkg.mod.fn``); SLICE 4b honours this as a fallback so
    operators who type the legacy spelling do not see a confusing
    'no colon' error. The canonical syntax is still ``pkg.mod:fn``.
    """
    from spendguard.integrations.litellm import ResolverContext
    from integrations.fixtures.fake_resolver import (
        FIXTURE_BUDGET_ID,
    )

    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-dot")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/dot.sock")
    clean_env.setenv(
        "SPENDGUARD_RESOLVER_MODULE",
        "integrations.fixtures.fake_resolver.make_triple",
    )

    g = SpendGuardGuardrail.from_env()
    ctx = ResolverContext(data={}, user_api_key_dict=None, call_type="completion")
    binding = g._delegate._budget_resolver(ctx)
    assert binding.budget_id == FIXTURE_BUDGET_ID


# Type-checking sanity — unused-import guard for the test file's
# ``Any`` symbol (kept for future-proofing as the test surface grows).
_ = Any
