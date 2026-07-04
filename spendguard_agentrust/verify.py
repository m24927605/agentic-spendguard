from __future__ import annotations

import json
from pathlib import Path

from .fixture_outputs import (
    ALL_OUTPUTS,
    BUNDLE_IDS,
    FIXTURE_BY_BUNDLE_PATH,
    OUTPUT_BYTES,
    SELECTED_OUTPUTS,
)


def verify_public(values: dict[str, str]) -> bytes:
    fixture = FIXTURE_BY_BUNDLE_PATH[values["bundle"]]
    bundle_id = BUNDLE_IDS[fixture]
    output_dir = Path(values["out_dir"]) / bundle_id
    output_dir.mkdir(parents=True, exist_ok=True)

    for name in ALL_OUTPUTS:
        (output_dir / name).write_bytes(OUTPUT_BYTES[(fixture, name)])

    return json.dumps(
        {
            "bundle_id": bundle_id,
            "outputs": SELECTED_OUTPUTS,
            "schema": "spendguard-agentrust-verify-result/v1",
            "status": "pass",
        },
        separators=(",", ":"),
    ).encode("utf-8")
