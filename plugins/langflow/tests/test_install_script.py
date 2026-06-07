"""Slice 3 unit tests -- ``spendguard-langflow-install`` CLI.

Covers I01..I05 from `tests.md` §2.4. These tests do NOT need langflow
installed -- the install script is a pure file-copy CLI.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from spendguard_langflow._install import (
    _SYSTEM_PATH_PREFIXES,
    install,
    main,
)


def test_I01_install_copies_component_and_metadata(tmp_path: Path) -> None:
    """I01: install drops BOTH the component shim AND the metadata YAML."""
    target = tmp_path / "components"
    component_dst, metadata_dst = install(target)
    assert component_dst.exists()
    assert metadata_dst.exists()
    assert component_dst.name == "spendguard_chat_model_wrapper.py"
    assert metadata_dst.name == "spendguard_chat_model_wrapper.yaml"
    # The shim re-imports the pip-installed class.
    text = component_dst.read_text()
    assert "spendguard_langflow.component" in text
    assert "SpendGuardChatModelWrapper" in text


def test_I02_install_refuses_existing_without_force(tmp_path: Path) -> None:
    """I02: existing file without --force -> FileExistsError, exit code 2."""
    target = tmp_path / "components"
    install(target)
    with pytest.raises(FileExistsError):
        install(target, force=False)
    # CLI surface returns exit code 2 on FileExistsError.
    rc = main(["--target", str(target)])
    assert rc == 2


def test_I03_install_force_overwrites(tmp_path: Path) -> None:
    """I03: --force overwrites existing files cleanly."""
    target = tmp_path / "components"
    install(target)
    component_dst = target / "spendguard_chat_model_wrapper.py"
    # Tamper with the existing file so we can prove the overwrite happened.
    component_dst.write_text("# tampered")
    install(target, force=True)
    text = component_dst.read_text()
    assert "tampered" not in text
    assert "SpendGuardChatModelWrapper" in text


def test_I04_install_refuses_system_path() -> None:
    """I04: system paths refused (INV-8 supply-chain footgun guard)."""
    for prefix in _SYSTEM_PATH_PREFIXES:
        # Use an obviously-system path. Don't actually try to write --
        # the refusal fires BEFORE mkdir is attempted.
        target = Path(prefix) / "langflow_components"
        with pytest.raises(PermissionError) as ei:
            install(target)
        assert "system path" in str(ei.value)


def test_I05_install_target_auto_creates_parents(tmp_path: Path) -> None:
    """I05: target subdirectory doesn't exist -> mkdir + copy succeeds."""
    deep = tmp_path / "a" / "b" / "c" / "components"
    assert not deep.exists()
    component_dst, metadata_dst = install(deep)
    assert deep.exists()
    assert component_dst.exists()
    assert metadata_dst.exists()
