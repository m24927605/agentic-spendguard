# ruff: noqa: ANN001, ANN201, ANN202, ANN401, S106, S108
# Rationale: ``monkeypatch`` is typed Any; literal sentinel tokens are
# not real credentials; ``/tmp`` paths never touch disk (UDS values
# inspected as strings only).
"""COV_D11_S5 — ``spendguard_guardrail_factory`` proxy_config.yaml tests.

Tier 1 unit tests per ``docs/slices/COV_D11_S5_proxy_config_entry.md``
test plan + ``docs/specs/coverage/D11_litellm_proxy_plugin/review-standards.md``
§Slice 5 reviewer checklist (5.1 - 5.4).

Coverage:
    * Full inline-config dict → constructs guardrail.
    * Env-var fallback when inline keys missing.
    * Missing required keys (both inline + env) → fail-closed at boot.
    * Distinct instances per invocation (no module-level singleton).
    * Inline ``disabled: true`` → no-op delegate installed.
    * Inline ``resolver_module:`` → SLICE 4b dispatch verified.
    * Inline keys win over env vars when both set.
    * YAML smoke: load the shipped ``proxy_config.yaml`` and feed its
      ``litellm_params`` into the factory.
    * LiteLLM-registry kwargs splice (``guardrail_name`` /
      ``event_hook`` / ``default_on`` / etc.) is tolerated.
    * Factory delegates to ``from_config``.

Anti-scope:
    * No demo / no real sidecar / no LiteLLM proxy boot.
    * No docs page parse (SLICE 7).
"""

from __future__ import annotations

from pathlib import Path
from typing import Any
from unittest.mock import patch

import pytest

# Skip cleanly when LiteLLM (and therefore ``CustomGuardrail``) is
# missing, matching the SLICE 1-4b importorskip pattern.
pytest.importorskip(
    "litellm.integrations.custom_guardrail",
    reason="LiteLLM with guardrail support not installed; "
    "install spendguard-sdk[litellm-guardrail]",
)

from spendguard.errors import SpendGuardConfigError  # noqa: E402
from spendguard.integrations.litellm import _LoopBoundCallback  # noqa: E402
from spendguard.integrations.litellm_guardrail import (  # noqa: E402
    SPENDGUARD_GUARDRAIL_MODULE_PATH,
    SpendGuardGuardrail,
    _NoopGuardrailDelegate,
    spendguard_guardrail_factory,
)

# Path to the SLICE 5 operator-facing yaml stanza. Discovered relative
# to the repository root (this test file lives at
# ``sdk/python/tests/integrations/`` so the repo root is 4 parents
# up). Mirrors how the docs page (SLICE 7) will reference it.
_REPO_ROOT = Path(__file__).resolve().parents[4]
_PROXY_CONFIG_YAML = (
    _REPO_ROOT / "examples" / "litellm-proxy" / "proxy_config.yaml"
)


@pytest.fixture
def clean_env(monkeypatch):
    """Strip every ``SPENDGUARD_*`` var so each test sets exactly what
    it needs — avoids inheriting host config that would mask a
    missing-var assertion. Mirrors the SLICE 4 / 4b fixture.
    """
    import os
    for k in [k for k in os.environ if k.startswith("SPENDGUARD_")]:
        monkeypatch.delenv(k, raising=False)
    return monkeypatch


# ---------------------------------------------------------------------------
# Inline-config happy path.
# ---------------------------------------------------------------------------


def test_spendguard_guardrail_factory_with_full_inline_config(clean_env):
    """Full inline dict → constructed guardrail with all required
    fields wired to the delegate. Env vars are NOT consulted when
    inline values cover the required surface (review-standards §5.4
    'fail-closed at boot' only applies when BOTH inline and env are
    missing).
    """
    g = spendguard_guardrail_factory({
        "tenant_id": "inline-tenant",
        "sidecar_address": "unix:///tmp/inline.sock",
        "api_key": "sk-inline",
        "proxy_timeout_ms": 7500,
    })

    assert isinstance(g, SpendGuardGuardrail)
    assert g._delegate._tenant_id == "inline-tenant"
    assert g._delegate._socket_path == "unix:///tmp/inline.sock"
    assert g._config_api_key == "sk-inline"
    assert g._config_proxy_timeout_ms == 7500


# ---------------------------------------------------------------------------
# Env fallback when inline missing.
# ---------------------------------------------------------------------------


def test_spendguard_guardrail_factory_with_env_fallback(clean_env):
    """Empty inline dict → factory reads SPENDGUARD_* env vars to
    satisfy the required-key contract. Mirrors the SLICE 4
    ``from_env`` behaviour but routed through SLICE 5's merge layer.
    """
    clean_env.setenv("SPENDGUARD_TENANT_ID", "env-tenant-s5")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/env-s5.sock")
    clean_env.setenv("SPENDGUARD_API_KEY", "sk-env-s5")

    g = spendguard_guardrail_factory({})

    assert isinstance(g, SpendGuardGuardrail)
    assert g._delegate._tenant_id == "env-tenant-s5"
    assert g._delegate._socket_path == "unix:///tmp/env-s5.sock"
    assert g._config_api_key == "sk-env-s5"


def test_spendguard_guardrail_factory_env_fallback_legacy_sidecar_uds(clean_env):
    """``SPENDGUARD_SIDECAR_UDS`` is the legacy alias for
    ``SPENDGUARD_SIDECAR_ADDRESS`` — when only the legacy var is set
    the factory honours it so existing operators (e.g. the
    ``examples/litellm-proxy-composite`` deployment) do not regress.
    Mirrors SLICE 4's ``_read_env_config`` fallback semantics.
    """
    clean_env.setenv("SPENDGUARD_TENANT_ID", "tenant-legacy")
    clean_env.setenv("SPENDGUARD_SIDECAR_UDS", "/run/spendguard/legacy.sock")

    g = spendguard_guardrail_factory({})

    assert g._delegate._socket_path == "/run/spendguard/legacy.sock"


# ---------------------------------------------------------------------------
# Missing required keys → SpendGuardConfigError (fail-closed at boot).
# ---------------------------------------------------------------------------


def test_spendguard_guardrail_factory_raises_on_missing_tenant_id(clean_env):
    """Neither inline nor env supplies ``tenant_id`` →
    ``SpendGuardConfigError`` naming the key. The error MUST surface
    at construction time, NOT first hook invocation — this is the
    review-standards §5.4 Major "fail-closed at boot" gate.
    """
    # Only sidecar_address supplied (inline); tenant_id absent
    # everywhere.
    with pytest.raises(SpendGuardConfigError) as exc_info:
        spendguard_guardrail_factory({
            "sidecar_address": "unix:///tmp/x.sock",
        })

    msg = str(exc_info.value)
    assert "tenant_id" in msg, (
        f"error message must name the missing key; got: {msg!r}"
    )


def test_spendguard_guardrail_factory_raises_on_missing_sidecar_address(
    clean_env,
):
    """Neither inline nor env supplies ``sidecar_address`` (and no
    legacy alias) → ``SpendGuardConfigError``. The factory must check
    BOTH the canonical key and the legacy aliases before raising.
    """
    with pytest.raises(SpendGuardConfigError) as exc_info:
        spendguard_guardrail_factory({"tenant_id": "t-no-sock"})

    assert "sidecar_address" in str(exc_info.value)


def test_spendguard_guardrail_factory_raises_on_non_dict_input(clean_env):
    """Defensive: the factory rejects non-dict ``litellm_params`` with
    a typed error that names the expected module path so operators
    can fix their yaml. Pinned because the LiteLLM registry
    occasionally passes through a ``LitellmParams`` model — when that
    happens we want a clear "expected dict" message, not an opaque
    ``AttributeError`` from ``.items()``.
    """
    with pytest.raises(SpendGuardConfigError) as exc_info:
        spendguard_guardrail_factory("not a dict")  # type: ignore[arg-type]

    msg = str(exc_info.value)
    assert "dict" in msg
    # Error message helps operators by naming the expected dotted path.
    assert "spendguard_guardrail_factory" in msg


# ---------------------------------------------------------------------------
# Non-singleton invariant (review-standards 1.4 carryover).
# ---------------------------------------------------------------------------


def test_spendguard_guardrail_factory_returns_distinct_instances(clean_env):
    """Two calls → two distinct objects with two distinct underlying
    delegates. No module-level singleton. Matches the SLICE 4
    ``test_from_env_creates_separate_instances`` invariant.
    """
    config = {
        "tenant_id": "distinct-t",
        "sidecar_address": "unix:///tmp/distinct.sock",
    }
    g1 = spendguard_guardrail_factory(config)
    g2 = spendguard_guardrail_factory(config)

    assert g1 is not g2
    assert g1._delegate is not g2._delegate


# ---------------------------------------------------------------------------
# Disabled mode — inline ``disabled: true`` installs no-op delegate.
# ---------------------------------------------------------------------------


def test_spendguard_guardrail_factory_inline_disabled_returns_noop(clean_env):
    """Inline ``disabled: True`` → no-op delegate installed; hooks
    short-circuit without touching the sidecar gRPC channel. Mirrors
    the SLICE 4 ``test_from_config_disabled_bool_honoured`` invariant.
    """
    g = spendguard_guardrail_factory({
        "tenant_id": "t-disabled",
        "sidecar_address": "unix:///tmp/disabled.sock",
        "disabled": True,
    })

    assert isinstance(g._delegate, _NoopGuardrailDelegate)
    assert g._config_disabled is True


def test_spendguard_guardrail_factory_env_disabled_truthy_string(clean_env):
    """``SPENDGUARD_DISABLED=true`` (env-only, no inline) → no-op
    delegate. The env-var truthy-string parser is shared with SLICE 4
    via ``_coerce_config_dict._parse_disabled`` — pinned so a future
    rename of ``_parse_disabled`` does not silently regress the
    SLICE 5 env-fallback path.
    """
    clean_env.setenv("SPENDGUARD_TENANT_ID", "t-env-d")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/env-d.sock")
    clean_env.setenv("SPENDGUARD_DISABLED", "true")

    g = spendguard_guardrail_factory({})

    assert isinstance(g._delegate, _NoopGuardrailDelegate)
    assert g._config_disabled is True


# ---------------------------------------------------------------------------
# Resolver-module dispatch — inline SLICE 4b path verification.
# ---------------------------------------------------------------------------


@pytest.fixture
def fixture_resolver_on_path(monkeypatch):
    """Insert the SDK ``tests`` dir onto ``sys.path`` so the shared
    ``integrations.fixtures.fake_resolver`` fixture is importable via
    ``importlib.import_module``. Mirrors the SLICE 4b fixture pattern.
    """
    import os
    import sys as _sys
    tests_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    monkeypatch.syspath_prepend(tests_dir)
    yield tests_dir
    for mod_name in list(_sys.modules):
        if mod_name.startswith("integrations.fixtures.fake_resolver"):
            _sys.modules.pop(mod_name, None)


def test_spendguard_guardrail_factory_inline_resolver_module_loads(
    clean_env, fixture_resolver_on_path,
):
    """Inline ``resolver_module: "pkg.mod:fn"`` → SLICE 4b dispatch
    fires. The fixture resolver returns sentinel ``FIXTURE_*`` values
    distinguishable from the env-var defaults, proving the operator
    factory was called rather than the single-tenant closure.
    """
    from spendguard.integrations.litellm import ResolverContext
    from tests.integrations.fixtures.fake_resolver import (  # type: ignore[import-not-found]  # noqa: E501
        FIXTURE_BUDGET_ID,
        FIXTURE_UNIT_ID,
        FIXTURE_WINDOW_ID,
    )

    g = spendguard_guardrail_factory({
        "tenant_id": "t-rm-inline",
        "sidecar_address": "unix:///tmp/rm.sock",
        "resolver_module": "integrations.fixtures.fake_resolver:make_triple",
    })

    assert isinstance(g._delegate, _LoopBoundCallback)
    ctx = ResolverContext(
        data={}, user_api_key_dict=None, call_type="completion",
    )
    binding = g._delegate._budget_resolver(ctx)
    assert binding.budget_id == FIXTURE_BUDGET_ID
    assert binding.window_instance_id == FIXTURE_WINDOW_ID
    assert binding.unit.unit_id == FIXTURE_UNIT_ID


def test_spendguard_guardrail_factory_bad_resolver_module_raises_at_boot(
    clean_env,
):
    """Inline ``resolver_module`` pointing at a non-importable module
    → ``SpendGuardConfigError`` at construction time. Pins the
    "fail-closed at boot" review-standards §5.4 Major: a typo in the
    yaml MUST surface before the first request, NOT on first hook
    invocation.
    """
    with pytest.raises(SpendGuardConfigError) as exc_info:
        spendguard_guardrail_factory({
            "tenant_id": "t-rm-bad",
            "sidecar_address": "unix:///tmp/rm-bad.sock",
            "resolver_module": "definitely_not_a_real_module_xyz:bad_attr",
        })

    msg = str(exc_info.value)
    # SLICE 4b's error message names the env-var spelling even when
    # invoked via inline config; we just check the operator can
    # identify the offending field.
    assert "definitely_not_a_real_module_xyz" in msg or "SPENDGUARD_RESOLVER_MODULE" in msg


# ---------------------------------------------------------------------------
# Inline-precedence-over-env contract.
# ---------------------------------------------------------------------------


def test_spendguard_guardrail_factory_inline_precedence_over_env(clean_env):
    """When both inline and env supply the same key, inline wins.
    Operators set deterministic yaml in deployments where env-var
    drift is a real risk — the contract is that yaml is the
    source of truth, env vars only fill gaps.
    """
    clean_env.setenv("SPENDGUARD_TENANT_ID", "env-loser")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///env-loser.sock")
    clean_env.setenv("SPENDGUARD_API_KEY", "env-loser-key")

    g = spendguard_guardrail_factory({
        "tenant_id": "inline-winner",
        "sidecar_address": "unix:///inline-winner.sock",
        "api_key": "inline-winner-key",
    })

    assert g._delegate._tenant_id == "inline-winner"
    assert g._delegate._socket_path == "unix:///inline-winner.sock"
    assert g._config_api_key == "inline-winner-key"


def test_spendguard_guardrail_factory_empty_inline_falls_back_to_env(
    clean_env,
):
    """An empty-string / whitespace inline value does NOT block the
    env fallback. Pinned because yaml authors occasionally type
    ``tenant_id: ""`` to "stub out" a key — without this fallback the
    factory would treat that as "operator explicitly set empty" and
    raise. We treat empty strings as "missing" so the env var fires.
    """
    clean_env.setenv("SPENDGUARD_TENANT_ID", "env-wins-on-empty-inline")
    clean_env.setenv("SPENDGUARD_SIDECAR_ADDRESS", "unix:///tmp/env-fb.sock")

    g = spendguard_guardrail_factory({
        "tenant_id": "",
        "sidecar_address": "   ",
    })

    assert g._delegate._tenant_id == "env-wins-on-empty-inline"
    assert g._delegate._socket_path == "unix:///tmp/env-fb.sock"


# ---------------------------------------------------------------------------
# YAML smoke — load the shipped operator-facing config and feed it in.
# ---------------------------------------------------------------------------


def test_spendguard_guardrail_factory_yaml_smoke(clean_env):
    """Load ``examples/litellm-proxy/proxy_config.yaml`` via PyYAML,
    extract the SpendGuard guardrail's ``litellm_params``, fill the
    required keys from env vars, and feed the merged dict into the
    factory. Verifies the shipped yaml stanza is parseable AND the
    factory accepts the literal dict shape an operator would receive
    at proxy boot. Closes the doc-to-implementation feedback loop
    (review-standards §5.1 Blocker: mode literal must be ``pre_call``).
    """
    yaml = pytest.importorskip(
        "yaml",
        reason="PyYAML required for yaml smoke test "
        "(transitively installed via litellm[proxy])",
    )

    clean_env.setenv("SPENDGUARD_TENANT_ID", "yaml-smoke-tenant")
    clean_env.setenv(
        "SPENDGUARD_SIDECAR_ADDRESS",
        "unix:///tmp/yaml-smoke.sock",
    )

    assert _PROXY_CONFIG_YAML.exists(), (
        f"Operator-facing yaml not found at {_PROXY_CONFIG_YAML!r}; "
        "SLICE 5 must ship the file at this path."
    )

    cfg = yaml.safe_load(_PROXY_CONFIG_YAML.read_text())

    # Verify the operator-facing shape so a future yaml edit that
    # accidentally renames `guardrails:` or `litellm_params:` fails
    # noisily (review-standards §5.1 mode literal pinned here).
    assert "guardrails" in cfg
    assert len(cfg["guardrails"]) == 1
    entry = cfg["guardrails"][0]
    assert entry["guardrail_name"] == "spendguard"
    params = entry["litellm_params"]
    assert params["guardrail"] == SPENDGUARD_GUARDRAIL_MODULE_PATH, (
        "yaml ``guardrail:`` path must match the factory's dotted "
        "module path the SDK exports — operators copy this string "
        "verbatim and we cannot let it drift."
    )
    assert params["mode"] == "pre_call", (
        "review-standards §5.1 Blocker: mode must be 'pre_call' "
        "literal, not 'during_call' / 'logging_only'."
    )
    assert params["default_on"] is True, (
        "review-standards §5.3 Major: default_on must be True so the "
        "guardrail fires on every request without per-key opt-in."
    )

    g = spendguard_guardrail_factory(params)

    assert isinstance(g, SpendGuardGuardrail)
    assert g._delegate._tenant_id == "yaml-smoke-tenant"
    assert g._delegate._socket_path == "unix:///tmp/yaml-smoke.sock"


# ---------------------------------------------------------------------------
# LiteLLM-registry kwargs-splice tolerance (verified against
# `litellm/proxy/guardrails/guardrail_registry.py:563-568`).
# ---------------------------------------------------------------------------


def test_spendguard_guardrail_factory_tolerates_litellm_registry_kwargs(
    clean_env,
):
    """LiteLLM's registry resolves the dotted ``guardrail:`` path to
    this factory and calls it like a class constructor::

        factory(
            guardrail_name="spendguard",
            event_hook="pre_call",
            default_on=True,
            **extra_inline_params,
        )

    The factory MUST strip the registry-binding kwargs
    (``guardrail_name`` / ``event_hook`` / ``default_on`` etc.) and
    feed only the SpendGuard-config keys to ``from_config``. Without
    this filter the binding-key splice would surface as "unknown
    config key" errors from a future SLICE 4b hardener pass.
    """
    g = spendguard_guardrail_factory(
        guardrail_name="spendguard",
        event_hook="pre_call",
        default_on=True,
        supported_event_hooks=["pre_call"],
        tenant_id="kwargs-splice-tenant",
        sidecar_address="unix:///tmp/kwargs.sock",
    )

    assert isinstance(g, SpendGuardGuardrail)
    assert g._delegate._tenant_id == "kwargs-splice-tenant"
    assert g._delegate._socket_path == "unix:///tmp/kwargs.sock"


def test_spendguard_guardrail_factory_dict_and_kwargs_merge(clean_env):
    """Defensive: when a caller passes BOTH a dict AND kwargs (e.g. a
    custom bootstrap that wraps the factory), the kwargs win. This
    matches LiteLLM's "explicit kwarg wins over inline" precedence
    so a wrapper that injects a kwarg cannot be silently shadowed by
    a stale dict value.
    """
    g = spendguard_guardrail_factory(
        {"tenant_id": "dict-loser", "sidecar_address": "unix:///dict.sock"},
        tenant_id="kwargs-winner",
        sidecar_address="unix:///kwargs.sock",
    )

    assert g._delegate._tenant_id == "kwargs-winner"
    assert g._delegate._socket_path == "unix:///kwargs.sock"


# ---------------------------------------------------------------------------
# Delegation contract — factory routes through ``from_config``.
# ---------------------------------------------------------------------------


def test_spendguard_guardrail_factory_calls_from_config_path(clean_env):
    """The factory's job is to merge inline + env then hand off to
    ``SpendGuardGuardrail.from_config``. Mocking the classmethod
    verifies the delegation contract so a future refactor that moves
    construction logic out of ``from_config`` cannot silently bypass
    it.
    """
    with patch.object(
        SpendGuardGuardrail,
        "from_config",
        wraps=SpendGuardGuardrail.from_config,
    ) as mocked:
        g = spendguard_guardrail_factory({
            "tenant_id": "delegated-tenant",
            "sidecar_address": "unix:///tmp/delegated.sock",
        })

    mocked.assert_called_once()
    # The merged dict passed to from_config carries the tenant_id /
    # sidecar_address keys regardless of how the factory normalises
    # binding-key splices — pinned here so the contract is stable.
    delegated_arg = mocked.call_args[0][0]
    assert delegated_arg["tenant_id"] == "delegated-tenant"
    assert delegated_arg["sidecar_address"] == "unix:///tmp/delegated.sock"
    assert isinstance(g, SpendGuardGuardrail)


# ---------------------------------------------------------------------------
# Type-checking sanity / unused-import guard.
# ---------------------------------------------------------------------------

_ = Any
