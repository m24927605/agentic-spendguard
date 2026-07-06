import base64
from datetime import datetime, timezone
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
    _check_cloudevent_canonicalization_probe,
    _cloudevent_proto_canonical_bytes,
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

    def test_non_ledger_proto_canonical_bytes_match_committed_vector(self):
        vector = json.loads(
            (ROOT / "docs" / "specs" / "agentrust-cloudevent-signature-vector-2026-07-01.json").read_text()
        )
        ledger_payload = json.loads(vector["ledger_json_branch"]["canonical_bytes_utf8"])
        event_time = datetime.fromtimestamp(ledger_payload["time_seconds"], timezone.utc).strftime(
            "%Y-%m-%dT%H:%M:%S"
        ) + f".{ledger_payload['time_nanos'] // 1_000_000:03d}Z"
        event = {
            "cloudevent": {
                "data_b64": vector["payload"]["data_b64_standard_padded"],
                "data_json": json.loads(vector["payload"]["data_json_canonical_without_lf"]),
                "datacontenttype": ledger_payload["datacontenttype"],
                "decision_id": ledger_payload["decisionid"],
                "id": ledger_payload["id"],
                "producer_id": vector["proto_branch"]["producer_id"],
                "producer_sequence": ledger_payload["producer_sequence"],
                "run_id": ledger_payload["runid"],
                "schema_bundle_id": ledger_payload["schema_bundle_id"],
                "source": ledger_payload["source"],
                "specversion": ledger_payload["specversion"],
                "tenant_id": ledger_payload["tenantid"],
                "time": event_time,
                "type": ledger_payload["type"],
            },
            "signature": {"key_id": ledger_payload["signing_key_id"]},
        }
        expected = bytes.fromhex(vector["proto_branch"]["canonical_bytes_hex"])

        canonical = _ledger_canonical_bytes(event)

        self.assertEqual(canonical, _cloudevent_proto_canonical_bytes(event))
        self.assertEqual(expected, canonical)
        self.assertEqual(
            vector["proto_branch"]["canonical_bytes_sha256"],
            __import__("hashlib").sha256(canonical).hexdigest(),
        )
        self.assertTrue(
            _verify_ed25519_signature(
                b64url_decode(vector["key"]["public_key_base64url"]),
                b64url_decode(vector["proto_branch"]["signature_base64url"]),
                canonical,
            )
        )
        _check_cloudevent_canonicalization_probe()

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
