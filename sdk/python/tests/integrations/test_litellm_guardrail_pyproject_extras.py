# ruff: noqa: ANN001, ANN201, ANN202
"""COV_D11_S5 — packaging extras + module-path resolution tests.

Tier 1 unit tests per ``docs/internal/slices/COV_D11_S5_proxy_config_entry.md``
test plan + ``docs/specs/coverage/D11_litellm_proxy_plugin/review-standards.md``
§Slice 5 reviewer checklist (5.2 Blocker: ``litellm-guardrail`` extra
exists with the correct floor; existing ``litellm`` extra unchanged).

Coverage:
    * pyproject.toml's ``[project.optional-dependencies]`` exposes the
      ``litellm-guardrail`` extra (spec name; the SLICE 1 ImportError
      already points operators at this exact extras spelling).
    * The extra pins ``litellm[proxy]>=1.55`` so ``CustomGuardrail`` is
      guaranteed to be importable (the surface only landed in 1.55+).
    * The legacy ``litellm`` extra is NOT mutated by SLICE 5
      (review-standards §5.2: "Existing `litellm` extra unchanged
      (floor stays at 1.50)").
    * The factory module-path string the README + yaml stanza
      reference is importable via ``importlib`` — closes the
      doc-to-implementation feedback loop so an operator typo can
      never reach prod without a failing test first.

Anti-scope:
    * No live ``pip install`` exercising the extras resolver
      (covered by the integ-test matrix, not the unit suite).
    * No yaml parse (lives in
      ``test_litellm_guardrail_proxy_config.py``).
"""

from __future__ import annotations

import importlib
import sys
from pathlib import Path

import pytest

# tomllib is stdlib on 3.11+; this test suite runs on 3.10 via the
# `tomli` backport (already in the dev extras via pytest dep tree).
# Use stdlib when available so we do not add a new dep.
if sys.version_info >= (3, 11):
    import tomllib as toml_loader  # type: ignore[import-not-found]
else:  # pragma: no cover - 3.10 fallback path
    import tomli as toml_loader  # type: ignore[import-not-found,no-redef]


# pyproject.toml lives at sdk/python/pyproject.toml; this test file
# sits at sdk/python/tests/integrations/ so the pyproject is 2 parents
# up.
_PYPROJECT = (
    Path(__file__).resolve().parents[2] / "pyproject.toml"
)


@pytest.fixture(scope="module")
def pyproject_data():
    """Parse pyproject.toml once per module. Cached so the 5+ tests
    here do not re-parse the same file.
    """
    assert _PYPROJECT.exists(), (
        f"pyproject.toml not found at {_PYPROJECT!r}; "
        "SLICE 5 test cannot validate packaging extras without it."
    )
    return toml_loader.loads(_PYPROJECT.read_text())


# ---------------------------------------------------------------------------
# Extras presence + shape.
# ---------------------------------------------------------------------------


def test_pyproject_has_litellm_guardrail_extras(pyproject_data):
    """``[project.optional-dependencies].litellm-guardrail`` exists.
    This is the canonical extras name shipped by SLICE 5 per
    ``review-standards.md`` §5.2 Blocker. The SLICE 1
    ``ImportError`` message already references this exact spelling
    (``install spendguard-sdk[litellm-guardrail]``) so the extras
    key MUST match — a rename would surface as a misleading install
    hint at first hook invocation.
    """
    optional = pyproject_data["project"]["optional-dependencies"]
    assert "litellm-guardrail" in optional, (
        "review-standards §5.2 Blocker: pyproject must expose the "
        "'litellm-guardrail' extra. Existing extras: "
        f"{sorted(optional.keys())!r}"
    )


def test_litellm_guardrail_extras_pins_litellm_proxy_at_or_above_1_55(
    pyproject_data,
):
    """The ``litellm-guardrail`` extra pins ``litellm[proxy]>=1.55``
    (review-standards §5.2 Blocker). The 1.55 floor is non-negotiable:
    LiteLLM shipped ``CustomGuardrail`` in 1.55 — pinning lower would
    leave the SLICE 1 lazy-import ImportError reachable at boot
    against a "supported" LiteLLM version.
    """
    extras = pyproject_data["project"]["optional-dependencies"][
        "litellm-guardrail"
    ]
    litellm_entries = [e for e in extras if "litellm" in e.lower()]
    assert litellm_entries, (
        f"litellm-guardrail extras must include a litellm pin; got: {extras!r}"
    )

    # Look for ``litellm[proxy]`` with a version specifier of >=1.55
    # or higher. We do not parse PEP 440 here; substring is enough to
    # catch the common drift modes (typoed floor, missing [proxy]
    # sub-extra, etc.).
    matched = False
    for entry in litellm_entries:
        if "litellm[proxy]" in entry and ">=1.55" in entry:
            matched = True
            break
    assert matched, (
        f"review-standards §5.2 Blocker: litellm-guardrail extra must "
        f"pin 'litellm[proxy]>=1.55'; got: {litellm_entries!r}"
    )


def test_existing_litellm_extras_unchanged(pyproject_data):
    """Review-standards §5.2: ``Existing 'litellm' extra unchanged
    (floor stays at 1.50)``. SLICE 5 must NOT bump the legacy extra's
    floor — operators on ``pip install spendguard-sdk[litellm]`` for
    the legacy ``CustomLogger`` path stay on 1.50+ until they
    explicitly migrate.
    """
    extras = pyproject_data["project"]["optional-dependencies"]["litellm"]
    # The legacy extra still includes ``litellm[proxy]>=1.50`` —
    # SLICE 5 must not silently bump this to 1.55 by editing the wrong
    # line.
    legacy_proxy_pin = next(
        (e for e in extras if "litellm[proxy]" in e), None,
    )
    assert legacy_proxy_pin is not None
    assert ">=1.50" in legacy_proxy_pin, (
        f"legacy litellm extra's floor must stay at >=1.50; got: "
        f"{legacy_proxy_pin!r}. Did SLICE 5 accidentally bump it?"
    )


# ---------------------------------------------------------------------------
# Module-path resolution — the dotted string operators paste into
# proxy_config.yaml MUST resolve via importlib (closes the
# doc-to-implementation feedback loop the SLICE 7 docs page depends
# on).
# ---------------------------------------------------------------------------


def test_factory_function_importable_via_module_path():
    """The operator-facing yaml stanza references the factory via the
    dotted module path. Verify the SDK's exported constant matches
    AND resolves via ``importlib.import_module`` + ``getattr`` —
    mirroring LiteLLM's ``get_instance_fn`` resolution
    (``litellm/proxy/types_utils/utils.py:30-65``).
    """
    pytest.importorskip(
        "litellm.integrations.custom_guardrail",
        reason="LiteLLM with guardrail support not installed",
    )

    from spendguard.integrations.litellm_guardrail import (
        SPENDGUARD_GUARDRAIL_MODULE_PATH,
        spendguard_guardrail_factory,
    )

    # Sanity: the constant is the literal string operators paste into
    # yaml. Pin the canonical spelling so a future refactor that
    # renames the module / function surfaces here, not in operator
    # deployments.
    assert (
        SPENDGUARD_GUARDRAIL_MODULE_PATH
        == "spendguard.integrations.litellm_guardrail.spendguard_guardrail_factory"
    )

    # Resolve via the same shape LiteLLM uses: split on '.', last
    # component is the attribute name, prefix is the module path
    # (verified against ``get_instance_fn`` in LiteLLM 1.55+).
    parts = SPENDGUARD_GUARDRAIL_MODULE_PATH.split(".")
    module_name = ".".join(parts[:-1])
    attr_name = parts[-1]

    module = importlib.import_module(module_name)
    resolved = getattr(module, attr_name)

    assert resolved is spendguard_guardrail_factory, (
        "yaml dotted path must resolve to the exported factory "
        "function; got a different object — likely a name shadow."
    )
    assert callable(resolved)
