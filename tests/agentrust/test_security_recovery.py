import base64
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from spendguard_agentrust._make_verify import _scan_package_artifact_bytes
from spendguard_agentrust.verify import (
    VerificationError,
    _ledger_canonical_bytes,
    _verify_ed25519_signature,
    _verify_signed_event,
)


ROOT = Path(__file__).resolve().parents[2]
OUT_ROOT = ROOT / "integrations" / "agentrust" / "out"


def b64url_decode(value):
    return base64.urlsafe_b64decode(value + "=" * ((4 - len(value) % 4) % 4))


def run_verify(fixture):
    env = {
        **__import__("os").environ,
        "PYTHONPATH": str(ROOT),
        "PYTHONNOUSERSITE": "1",
    }
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
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


class AgenTrustSecurityRecoveryTest(unittest.TestCase):
    def setUp(self):
        shutil.rmtree(OUT_ROOT / "demo-allow", ignore_errors=True)
        shutil.rmtree(OUT_ROOT / "demo-deny", ignore_errors=True)

    def tearDown(self):
        shutil.rmtree(OUT_ROOT / "demo-allow", ignore_errors=True)
        shutil.rmtree(OUT_ROOT / "demo-deny", ignore_errors=True)

    def test_ledger_canonical_bytes_match_fixture_hash_and_ed25519_signature(self):
        event = json.loads(
            (ROOT / "integrations" / "agentrust" / "examples" / "allow" / "decision.json").read_text()
        )
        trust_store = json.loads(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_text()
        )

        canonical = _ledger_canonical_bytes(event)
        self.assertEqual(event["canonical_bytes_sha256"], __import__("hashlib").sha256(canonical).hexdigest())
        self.assertTrue(
            _verify_ed25519_signature(
                b64url_decode(trust_store["keys"][0]["public_key_base64url"]),
                b64url_decode(event["signature"]["value_base64url"]),
                canonical,
            )
        )

    def test_signed_event_verification_rejects_tampered_signature(self):
        event = json.loads(
            (ROOT / "integrations" / "agentrust" / "examples" / "allow" / "decision.json").read_text()
        )
        trust_store = json.loads(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_text()
        )
        event["signature"]["value_base64url"] = "A" + event["signature"]["value_base64url"][1:]

        with self.assertRaises(VerificationError) as raised:
            _verify_signed_event(event, trust_store)

        self.assertEqual("spendguard_signature_invalid", raised.exception.diagnostic.code)
        self.assertEqual("/signature/value_base64url", raised.exception.diagnostic.json_pointer)

    def test_bundle_entry_path_traversal_is_rejected_before_hashing(self):
        bundle_path = ROOT / "integrations" / "agentrust" / "examples" / "allow" / "bundle.json"
        original = bundle_path.read_bytes()
        try:
            bundle = json.loads(original)
            bundle["files"][0]["path"] = "../trust-store.json"
            bundle_path.write_text(json.dumps(bundle, separators=(",", ":")), encoding="utf-8")

            observed = run_verify("allow")
        finally:
            bundle_path.write_bytes(original)

        self.assertEqual(2, observed.returncode)
        payload = json.loads(observed.stderr)
        self.assertEqual("path_safety_error", payload["code"])
        self.assertEqual("/files/0/path", payload["json_pointer"])

    def test_package_guard_rejects_private_key_material_in_artifact_bytes(self):
        with self.assertRaises(AssertionError):
            _scan_package_artifact_bytes(
                "spendguard_agentrust-0.1.0a1-py3-none-any.whl",
                b"payload\n-----BEGIN PRIVATE KEY-----\n",
            )


if __name__ == "__main__":
    unittest.main()
