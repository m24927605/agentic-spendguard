# ruff: noqa: ANN001, ANN201, ANN202, ANN401, S106, S108
# Rationale: test fixtures use ``monkeypatch`` (Any) + non-secret literal
# tokens; the test never speaks to an actual sidecar / LLM provider.
# ``/tmp`` paths are unit-test sentinels that never touch disk — the
# UDS path is forwarded into the delegate's stash and inspected as a
# string, not connected to.
"""COV_D11_S4 — ``SpendGuardGuardrail`` env-driven factory tests.

Tier 1 unit tests per ``docs/slices/COV_D11_S4_env_defaults.md`` test
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


# Type-checking sanity — unused-import guard for the test file's
# ``Any`` symbol (kept for future-proofing as the test surface grows).
_ = Any
