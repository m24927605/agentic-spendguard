import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parents[2]
OUT_ROOT = ROOT / "integrations" / "agentrust" / "out"
EXPECTED_ROOT = ROOT / "integrations" / "agentrust" / "examples" / "expected-outputs"
SELECTED_OUTPUTS = [
    "trace-level0.json",
    "agent-manifest.json",
    "spendguard-artifacts.json",
]
ALL_OUTPUTS = [*SELECTED_OUTPUTS, "verification-report.json"]


def run_verify(fixture):
    env = os.environ.copy()
    env["PYTHONPATH"] = str(ROOT)
    env["PYTHONNOUSERSITE"] = "1"
    return subprocess.run(
        [
            sys.executable,
            "-m",
            "spendguard_agentrust",
            "verify",
            "--bundle",
            f"integrations/agentrust/examples/{fixture}",
            "--trust-store",
            "integrations/agentrust/examples/trust-store.json",
            "--trace-signing-key",
            "integrations/agentrust/test-private-keys/spendguard-agentrust-fixture.test.ed25519.pem",
            "--out-dir",
            "integrations/agentrust/out",
        ],
        cwd=ROOT,
        env=env,
        text=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


class PublicVerifyFixtureTest(unittest.TestCase):
    def setUp(self):
        for bundle_id in ("demo-allow", "demo-deny"):
            shutil.rmtree(OUT_ROOT / bundle_id, ignore_errors=True)

    def tearDown(self):
        for bundle_id in ("demo-allow", "demo-deny"):
            shutil.rmtree(OUT_ROOT / bundle_id, ignore_errors=True)

    def test_allow_and_deny_write_expected_outputs(self):
        for fixture, bundle_id in (("allow", "demo-allow"), ("deny", "demo-deny")):
            with self.subTest(fixture=fixture):
                observed = run_verify(fixture)

                self.assertEqual(0, observed.returncode)
                self.assertEqual(b"", observed.stderr)
                expected_stdout = json.dumps(
                    {
                        "bundle_id": bundle_id,
                        "outputs": SELECTED_OUTPUTS,
                        "schema": "spendguard-agentrust-verify-result/v1",
                        "status": "pass",
                    },
                    separators=(",", ":"),
                ).encode("utf-8")
                self.assertEqual(expected_stdout, observed.stdout)

                output_dir = OUT_ROOT / bundle_id
                for name in ALL_OUTPUTS:
                    self.assertEqual(
                        (EXPECTED_ROOT / fixture / name).read_bytes(),
                        (output_dir / name).read_bytes(),
                        name,
                    )


if __name__ == "__main__":
    unittest.main()
