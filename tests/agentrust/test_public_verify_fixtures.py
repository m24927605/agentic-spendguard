import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import hashlib


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

    def test_rejects_coherent_fixture_tamper(self):
        bundle_path = ROOT / "integrations" / "agentrust" / "examples" / "allow" / "bundle.json"
        decision = ROOT / "integrations" / "agentrust" / "examples" / "allow" / "decision.json"
        original_bundle = bundle_path.read_bytes()
        original_decision = decision.read_bytes()
        try:
            mutated_decision = original_decision + b" "
            decision.write_bytes(mutated_decision)
            bundle = json.loads(original_bundle)
            bundle["files"][0]["sha256"] = hashlib.sha256(mutated_decision).hexdigest()
            preimage = dict(bundle)
            preimage.pop("evidence_bundle_hash")
            bundle["evidence_bundle_hash"] = "sha256:" + hashlib.sha256(
                json.dumps(preimage, sort_keys=True, separators=(",", ":")).encode()
            ).hexdigest()
            bundle_path.write_text(json.dumps(bundle, separators=(",", ":")), encoding="utf-8")
            observed = run_verify("allow")
        finally:
            decision.write_bytes(original_decision)
            bundle_path.write_bytes(original_bundle)

        self.assertEqual(4, observed.returncode)
        payload = json.loads(observed.stderr)
        self.assertEqual("evidence_bundle_hash_mismatch", payload["code"])

    def test_rejects_missing_fixture_file_without_traceback(self):
        outcome = ROOT / "integrations" / "agentrust" / "examples" / "allow" / "outcome.json"
        hidden = outcome.with_suffix(".json.hidden")
        outcome.rename(hidden)
        try:
            observed = run_verify("allow")
        finally:
            hidden.rename(outcome)

        self.assertEqual(4, observed.returncode)
        self.assertEqual(b"", observed.stdout)
        self.assertNotIn(b"Traceback", observed.stderr)
        payload = json.loads(observed.stderr)
        self.assertEqual("file_hash_mismatch", payload["code"])

    def test_rejects_wrong_trust_key_with_same_key_id(self):
        trust_store = ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json"
        original = trust_store.read_bytes()
        try:
            payload = json.loads(original)
            payload["keys"][0]["public_key_base64url"] = "GX9rI-FshTLGq8g4-s1ep4m-DHaykgM0A5v6iz02jWE"
            trust_store.write_text(json.dumps(payload, separators=(",", ":")), encoding="utf-8")
            observed = run_verify("allow")
        finally:
            trust_store.write_bytes(original)

        self.assertEqual(3, observed.returncode)
        payload = json.loads(observed.stderr)
        self.assertEqual("spendguard_signature_invalid", payload["code"])
        self.assertEqual("/keys/0/public_key_base64url", payload["json_pointer"])

    def test_rejects_coherent_signature_tamper_at_fixture_root(self):
        bundle_path = ROOT / "integrations" / "agentrust" / "examples" / "allow" / "bundle.json"
        decision = ROOT / "integrations" / "agentrust" / "examples" / "allow" / "decision.json"
        original_bundle = bundle_path.read_bytes()
        original_decision = decision.read_bytes()
        try:
            payload = json.loads(original_decision)
            payload["signature"]["value_base64url"] = "A" + payload["signature"]["value_base64url"][1:]
            mutated_decision = json.dumps(payload, separators=(",", ":")).encode()
            decision.write_bytes(mutated_decision)
            bundle = json.loads(original_bundle)
            bundle["files"][0]["sha256"] = hashlib.sha256(mutated_decision).hexdigest()
            preimage = dict(bundle)
            preimage.pop("evidence_bundle_hash")
            bundle["evidence_bundle_hash"] = "sha256:" + hashlib.sha256(
                json.dumps(preimage, sort_keys=True, separators=(",", ":")).encode()
            ).hexdigest()
            bundle_path.write_text(json.dumps(bundle, separators=(",", ":")), encoding="utf-8")
            observed = run_verify("allow")
        finally:
            decision.write_bytes(original_decision)
            bundle_path.write_bytes(original_bundle)

        self.assertEqual(4, observed.returncode)
        payload = json.loads(observed.stderr)
        self.assertEqual("evidence_bundle_hash_mismatch", payload["code"])

    def test_rejects_symlinked_output_root(self):
        with tempfile.TemporaryDirectory() as tmp:
            shutil.rmtree(OUT_ROOT, ignore_errors=True)
            OUT_ROOT.symlink_to(Path(tmp), target_is_directory=True)
            try:
                observed = run_verify("allow")
            finally:
                OUT_ROOT.unlink(missing_ok=True)

        self.assertEqual(2, observed.returncode)
        payload = json.loads(observed.stderr)
        self.assertEqual("output_target_invalid", payload["code"])

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
