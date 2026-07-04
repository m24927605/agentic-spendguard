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
    return run_verify_args(
        [
            "verify",
            "--bundle",
            f"integrations/agentrust/examples/{fixture}",
            "--trust-store",
            "integrations/agentrust/examples/trust-store.json",
            "--trace-signing-key",
            "integrations/agentrust/test-private-keys/spendguard-agentrust-fixture.test.ed25519.pem",
            "--out-dir",
            "integrations/agentrust/out",
        ]
    )


def run_verify_args(args):
    env = os.environ.copy()
    env["PYTHONPATH"] = str(ROOT)
    env["PYTHONNOUSERSITE"] = "1"
    return subprocess.run(
        [sys.executable, "-m", "spendguard_agentrust", *args],
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

    def test_rejects_existing_final_output(self):
        output_dir = OUT_ROOT / "demo-allow"
        output_dir.mkdir(parents=True)

        observed = run_verify("allow")

        self.assertEqual(2, observed.returncode)
        self.assertEqual(b"", observed.stdout)
        payload = json.loads(observed.stderr)
        self.assertEqual("output_target_invalid", payload["code"])
        self.assertEqual("phase-2", payload["phase"])

    def test_rejects_stale_fixture_hash(self):
        decision = ROOT / "integrations" / "agentrust" / "examples" / "allow" / "decision.json"
        original = decision.read_bytes()
        try:
            decision.write_bytes(original + b" ")
            observed = run_verify("allow")
        finally:
            decision.write_bytes(original)

        self.assertEqual(4, observed.returncode)
        self.assertEqual(b"", observed.stdout)
        payload = json.loads(observed.stderr)
        self.assertEqual("file_hash_mismatch", payload["code"])
        self.assertEqual("phase-6", payload["phase"])

    def test_non_default_manifest_issuer_is_preserved(self):
        issuer = "spiffe://agent.example/a"
        observed = run_verify_args(
            [
                "verify",
                "--bundle",
                "integrations/agentrust/examples/allow",
                "--trust-store",
                "integrations/agentrust/examples/trust-store.json",
                "--trace-signing-key",
                "integrations/agentrust/test-private-keys/spendguard-agentrust-fixture.test.ed25519.pem",
                "--out-dir",
                "integrations/agentrust/out",
                "--manifest-issuer",
                issuer,
            ]
        )

        self.assertEqual(0, observed.returncode)
        manifest = json.loads((OUT_ROOT / "demo-allow" / "agent-manifest.json").read_text())
        self.assertEqual(issuer, manifest["issuer"])


if __name__ == "__main__":
    unittest.main()
