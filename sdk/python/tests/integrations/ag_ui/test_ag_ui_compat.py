# ruff: noqa: S101
"""TA-28 — pinned ag-ui-protocol compat (mirror twin of TP-28/TP-29).

Marker resolution (COV_D39_02, 2026-06-10): the Python ``CustomEvent``
import path at the exact test pin ``ag-ui-protocol==0.1.19`` is
``ag_ui.core.CustomEvent`` (defined in ``ag_ui.core.events``; re-exported
from ``ag_ui.core``). It IS a pydantic ``BaseModel``, so the PRIMARY
runtime-parse path applies — ``CustomEvent.model_validate(built_event)``
— not the TP-29 key-set fallback. ``timestamp`` is optional on the model
and absent-timestamp events validate cleanly.

``pytest.importorskip``-guarded: with zero extras installed this file
SKIPS; under the ``ag-ui`` extra / dev pin it runs (acceptance A2.7).
This is the ONLY place in the repo's Python tree allowed to import
``ag_ui`` (implementation.md §1.2 / §6).
"""

from __future__ import annotations

import pytest

pytest.importorskip("ag_ui")

from ag_ui.core import CustomEvent, EventType  # noqa: E402

from ._vectors import ALL_BUILDERS  # noqa: E402


@pytest.mark.parametrize("fn,cls,kw,name", ALL_BUILDERS)
def test_ta28_custom_event_model_validate(fn, cls, kw, name):
    """TA-28: CustomEvent.model_validate succeeds for all five builders
    under the exact test pin, with and without the optional timestamp."""
    evt = fn(cls(**kw), timestamp_ms=1765843200000)
    parsed = CustomEvent.model_validate(evt)
    assert parsed.type == EventType.CUSTOM
    assert parsed.name == name
    assert parsed.value == evt["value"]
    assert parsed.timestamp == 1765843200000

    no_ts = fn(cls(**kw))
    parsed_no_ts = CustomEvent.model_validate(no_ts)
    assert parsed_no_ts.timestamp is None
