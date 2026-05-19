# ruff: noqa: ANN001, ANN201
"""Slice 1 — Tier 1 missing-extra test.

Runs WITHOUT `litellm` installed (no `pytest.importorskip` at module
top) so the optional-import invariant is verified in the exact
environment that matters. Codex Slice 1 R1 P2 fix.
"""

from __future__ import annotations

import importlib
import sys

import pytest


def test_module_imports_without_litellm_installed(monkeypatch):
    """Patch sys.modules to simulate `litellm` absence; assert
    ImportError mentions the [litellm] extra hint."""
    for k in list(sys.modules):
        if (
            k == "litellm"
            or k.startswith("litellm.")
            or k == "spendguard.integrations.litellm"
        ):
            monkeypatch.delitem(sys.modules, k, raising=False)
    monkeypatch.setitem(sys.modules, "litellm", None)

    with pytest.raises(ImportError) as exc_info:
        importlib.import_module("spendguard.integrations.litellm")
    msg = str(exc_info.value)
    assert "spendguard-sdk[litellm]" in msg, (
        f"ImportError must reference the extra name; got: {msg!r}"
    )
