import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parents[2]
RUN_ROOT = ROOT / "integrations" / "agentrust" / "out" / ".verify-agentrust" / "unit-private"
PUBLIC_KEY = (
    ROOT
    / "integrations"
    / "agentrust"
    / "test-private-keys"
    / "spendguard-agentrust-fixture.test.ed25519.pem"
)
TRACE_KEY = RUN_ROOT / ".trace-key" / "trace-exporter.test.ed25519.pem"
EXPECTED_ROOT = ROOT / "integrations" / "agentrust" / "examples" / "expected-outputs"


def base_env():
    env = os.environ.copy()
    env["PYTHONPATH"] = str(ROOT)
    env["PYTHONNOUSERSITE"] = "1"
    return env


def prepare_trace_key():
    TRACE_KEY.parent.mkdir(parents=True, mode=0o700)
    TRACE_KEY.write_bytes(PUBLIC_KEY.read_bytes())


def run_internal(args, include_env=True):
    env = base_env()
    if include_env:
        env["SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT"] = "1"
    return subprocess.run(
        [sys.executable, "-m", "spendguard_agentrust._verify_internal", *args],
        cwd=ROOT,
        env=env,
        text=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


class PrivateVerifierTest(unittest.TestCase):
    def setUp(self):
        shutil.rmtree(RUN_ROOT, ignore_errors=True)
        prepare_trace_key()

    def tearDown(self):
        shutil.rmtree(RUN_ROOT, ignore_errors=True)

    def success_args(self, fixture):
        return [
            "verify",
            "--bundle",
            f"integrations/agentrust/examples/{fixture}",
            "--trust-store",
            "integrations/agentrust/examples/trust-store.json",
            "--trace-signing-key",
            "integrations/agentrust/out/.verify-agentrust/unit-private/.trace-key/trace-exporter.test.ed25519.pem",
            "--out-dir",
            "integrations/agentrust/out/.verify-agentrust/unit-private",
            "--fixture",
            fixture,
        ]

    def test_private_success_has_empty_streams_and_outputs(self):
        observed = run_internal(self.success_args("allow"))

        self.assertEqual(0, observed.returncode)
        self.assertEqual(b"", observed.stdout)
        self.assertEqual(b"", observed.stderr)
        for name in (
            "trace-level0.json",
            "agent-manifest.json",
            "spendguard-artifacts.json",
            "verification-report.json",
        ):
            self.assertEqual(
                (EXPECTED_ROOT / "allow" / name).read_bytes(),
                (RUN_ROOT / "demo-allow" / name).read_bytes(),
            )

    def test_private_env_gate_fails_closed(self):
        observed = run_internal(self.success_args("allow"), include_env=False)

        self.assertEqual(6, observed.returncode)
        payload = json.loads(observed.stderr)
        self.assertEqual("packaging_guard_failed", payload["code"])
        self.assertEqual("make-setup", payload["phase"])
        self.assertEqual(b"", observed.stdout)

    def test_private_negative_mutation_diagnostic(self):
        mutation_root = RUN_ROOT / ".mutations" / "mutated_signature" / "allow"
        (mutation_root / "bundle").mkdir(parents=True)
        (mutation_root / "out").mkdir()
        trust_store = mutation_root / "trust-store.json"
        trust_store.write_bytes(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
        )
        observed = run_internal(
            [
                "verify",
                "--bundle",
                "integrations/agentrust/out/.verify-agentrust/unit-private/.mutations/mutated_signature/allow/bundle",
                "--trust-store",
                "integrations/agentrust/out/.verify-agentrust/unit-private/.mutations/mutated_signature/allow/trust-store.json",
                "--trace-signing-key",
                "integrations/agentrust/out/.verify-agentrust/unit-private/.trace-key/trace-exporter.test.ed25519.pem",
                "--out-dir",
                "integrations/agentrust/out/.verify-agentrust/unit-private/.mutations/mutated_signature/allow/out",
                "--fixture",
                "allow",
                "--mutation",
                "mutated_signature",
            ]
        )

        self.assertEqual(3, observed.returncode)
        self.assertEqual(b"", observed.stdout)
        payload = json.loads(observed.stderr)
        self.assertEqual("phase-9", payload["phase"])
        self.assertEqual("spendguard_signature_invalid", payload["code"])
        self.assertEqual("/signature/value_base64url", payload["json_pointer"])


if __name__ == "__main__":
    unittest.main()
