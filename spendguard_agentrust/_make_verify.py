from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from .fixture_outputs import ALL_OUTPUTS, BUNDLE_IDS, OUTPUT_BYTES, SELECTED_OUTPUTS


ROOT = Path(__file__).resolve().parents[1]
OUT_ROOT = ROOT / "integrations" / "agentrust" / "out"


def main() -> int:
    try:
        _check_help()
        for fixture in ("allow", "deny"):
            _check_fixture(fixture)
    except AssertionError as exc:
        sys.stderr.write(str(exc))
        return 1

    sys.stdout.write("agentrust_make_verify=pass\n")
    return 0


def _base_env() -> dict[str, str]:
    env = os.environ.copy()
    env["PYTHONPATH"] = str(ROOT)
    env["PYTHONNOUSERSITE"] = "1"
    return env


def _run_public(*args: str) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        [sys.executable, "-m", "spendguard_agentrust", *args],
        cwd=ROOT,
        env=_base_env(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def _check_help() -> None:
    observed = _run_public("verify", "--help")
    _assert_equal(0, observed.returncode, "public help exit")
    _assert_equal(b"", observed.stderr, "public help stderr")


def _check_fixture(fixture: str) -> None:
    bundle_id = BUNDLE_IDS[fixture]
    output_dir = OUT_ROOT / bundle_id
    shutil.rmtree(output_dir, ignore_errors=True)
    observed = _run_public(
        "verify",
        "--bundle",
        f"integrations/agentrust/examples/{fixture}",
        "--trust-store",
        "integrations/agentrust/examples/trust-store.json",
        "--trace-signing-key",
        "integrations/agentrust/test-private-keys/spendguard-agentrust-fixture.test.ed25519.pem",
        "--out-dir",
        "integrations/agentrust/out",
    )
    _assert_equal(0, observed.returncode, f"{fixture} verifier exit")
    _assert_equal(b"", observed.stderr, f"{fixture} verifier stderr")
    expected_stdout = json.dumps(
        {
            "bundle_id": bundle_id,
            "outputs": SELECTED_OUTPUTS,
            "schema": "spendguard-agentrust-verify-result/v1",
            "status": "pass",
        },
        separators=(",", ":"),
    ).encode("utf-8")
    _assert_equal(expected_stdout, observed.stdout, f"{fixture} verifier stdout")
    for name in ALL_OUTPUTS:
        _assert_equal(
            OUTPUT_BYTES[(fixture, name)],
            (output_dir / name).read_bytes(),
            f"{fixture} {name}",
        )
    shutil.rmtree(output_dir, ignore_errors=True)


def _assert_equal(expected, observed, label: str) -> None:
    if expected != observed:
        raise AssertionError(f"{label} mismatch")


if __name__ == "__main__":
    raise SystemExit(main())
