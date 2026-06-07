"""Pytest config + shared fixtures for the SpendGuard Dify plugin.

Tests are isolated per review-standards.md cross-cutting "Test isolation":
NO Docker, NO running sidecar, NO outbound HTTP. We mock the upstream
SDK and the SpendGuard sidecar client.

Path bootstrap: pytest is invoked from ``plugins/dify/spendguard`` so the
``provider`` / ``models`` packages are importable without an editable
install (which would require Python 3.12 + ``dify-plugin`` install).
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

# Ensure plugin root is on sys.path BEFORE conftest imports anything.
PLUGIN_ROOT = Path(__file__).resolve().parent.parent
if str(PLUGIN_ROOT) not in sys.path:
    sys.path.insert(0, str(PLUGIN_ROOT))

# Default env so _DifyReservation.__init__ doesn't raise on import-side
# instantiation; individual tests can override via monkeypatch.
os.environ.setdefault("SPENDGUARD_SIDECAR_UDS", "/tmp/fake-spendguard.sock")
os.environ.setdefault("SPENDGUARD_TENANT_ID", "test-tenant")


def pytest_configure(config):
    """Inject the synchronous loop stub so unit tests don't need the
    real daemon background thread (which interferes with pytest-asyncio
    under gevent-monkey-patched dify_plugin imports)."""
    try:
        from models.llm.spendguard_llm import _DaemonLoop, _SyncLoopStub
    except ImportError:
        # dify-plugin not installed; tests will be skipped by importorskip.
        return
    _DaemonLoop.set_test_instance(_SyncLoopStub())
