"""Slice 1 unit tests -- :class:`SpendGuardChatModelWrapper` skeleton.

Covers C01..C06 from `tests.md` §2.1.

Each test is skip-tagged with ``pytest.importorskip("langflow")`` so the
suite runs cleanly in dev environments without the full Langflow
install (the package is large and CI installs it on demand).
"""

from __future__ import annotations

import pytest


def _component_cls():
    pytest.importorskip("langflow")
    from spendguard_langflow.component import SpendGuardChatModelWrapper

    return SpendGuardChatModelWrapper


def test_C01_class_imports_under_supported_langflow() -> None:
    """C01: component class imports cleanly under langflow>=1.8."""
    pytest.importorskip("langflow")
    from spendguard_langflow import SpendGuardChatModelWrapper

    assert SpendGuardChatModelWrapper is not None
    assert SpendGuardChatModelWrapper.__name__ == "SpendGuardChatModelWrapper"


def test_C02_class_introspection_lists_eight_inputs() -> None:
    """C02: exactly 8 declared inputs in spec order."""
    cls = _component_cls()
    names = [getattr(inp, "name", None) for inp in cls.inputs]
    assert names == [
        "inner",
        "sidecar_uds_path",
        "tenant_id",
        "budget_id",
        "window_instance_id",
        "unit_token_kind",
        "model_family",
        "claim_estimator_chars_per_token",
    ]
    assert len(cls.inputs) == 8


def test_C03_inner_handle_input_type() -> None:
    """C03: ``inner`` is a ``HandleInput`` with ``input_types=['LanguageModel']``."""
    cls = _component_cls()
    inner = next(i for i in cls.inputs if i.name == "inner")
    from langflow.inputs import HandleInput  # type: ignore

    assert isinstance(inner, HandleInput)
    assert getattr(inner, "input_types", None) == ["LanguageModel"]
    assert getattr(inner, "required", False) is True


def test_C04_output_is_languagemodel_handle() -> None:
    """C04: single Output ``name='model'`` ``types=['LanguageModel']``."""
    cls = _component_cls()
    assert len(cls.outputs) == 1
    out = cls.outputs[0]
    assert getattr(out, "name", None) == "model"
    assert getattr(out, "method", None) == "build_model"
    assert getattr(out, "types", None) == ["LanguageModel"]


def test_C05_required_and_advanced_flags() -> None:
    """C05: required inputs flagged required; advanced inputs flagged advanced."""
    cls = _component_cls()
    by_name = {i.name: i for i in cls.inputs}
    required_inputs = {
        "inner",
        "sidecar_uds_path",
        "tenant_id",
        "budget_id",
        "window_instance_id",
    }
    advanced_inputs = {
        "unit_token_kind",
        "model_family",
        "claim_estimator_chars_per_token",
    }
    for name in required_inputs:
        assert getattr(by_name[name], "required", False) is True, name
    for name in advanced_inputs:
        assert getattr(by_name[name], "advanced", False) is True, name


def test_C06_display_metadata_present() -> None:
    """C06: ``display_name`` / ``icon`` / ``documentation`` / description sane."""
    cls = _component_cls()
    assert cls.display_name == "SpendGuard Budget Gate"
    assert cls.icon == "shield"
    assert cls.documentation.startswith("https://")
    assert isinstance(cls.description, str)
    assert len(cls.description) >= 50
