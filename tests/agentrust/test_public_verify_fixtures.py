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


def clean_output_path(path: Path) -> None:
    if not os.path.lexists(path):
        return
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
    else:
        path.unlink()


def reset_out_root() -> None:
    if os.path.lexists(OUT_ROOT) and not OUT_ROOT.is_dir():
        clean_output_path(OUT_ROOT)
    if OUT_ROOT.is_symlink():
        OUT_ROOT.unlink()
    OUT_ROOT.mkdir(parents=True, exist_ok=True)
    for bundle_id in ("demo-allow", "demo-deny"):
        clean_output_path(OUT_ROOT / bundle_id)
    for temp_dir in OUT_ROOT.glob(".tmp-demo-*"):
        clean_output_path(temp_dir)


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
        reset_out_root()

    def tearDown(self):
        reset_out_root()

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

    def test_output_target_preflight_runs_before_input_validation(self):
        output_dir = OUT_ROOT / "demo-allow"
        output_dir.mkdir(parents=True)
        decision = ROOT / "integrations" / "agentrust" / "examples" / "allow" / "decision.json"
        original = decision.read_bytes()
        try:
            decision.write_bytes(original + b" ")
            observed = run_verify("allow")
        finally:
            decision.write_bytes(original)

        self.assertEqual(2, observed.returncode)
        self.assertEqual(b"", observed.stdout)
        payload = json.loads(observed.stderr)
        self.assertEqual("output_target_invalid", payload["code"])
        self.assertEqual("phase-2", payload["phase"])

    def test_cleans_stale_temp_directory_before_writes(self):
        stale = OUT_ROOT / (".tmp-demo-allow-" + "0" * 32)
        stale.mkdir(parents=True)

        observed = run_verify("allow")

        self.assertEqual(0, observed.returncode, observed.stderr)
        self.assertFalse(stale.exists())
        self.assertTrue((OUT_ROOT / "demo-allow").is_dir())

    def test_rejects_stale_temp_symlink_before_writes(self):
        stale = OUT_ROOT / (".tmp-demo-allow-" + "0" * 32)
        stale.symlink_to("missing-target", target_is_directory=True)
        try:
            observed = run_verify("allow")
        finally:
            stale.unlink(missing_ok=True)

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

        self.assertEqual(2, observed.returncode)
        self.assertEqual(b"", observed.stdout)
        self.assertNotIn(b"Traceback", observed.stderr)
        payload = json.loads(observed.stderr)
        self.assertEqual("file_set_invalid", payload["code"])
        self.assertEqual("phase-3", payload["phase"])

    def test_rejects_unlisted_physical_bundle_file(self):
        extra = ROOT / "integrations" / "agentrust" / "examples" / "deny" / "unexpected-extra.json"
        self.assertFalse(extra.exists())
        extra.write_text('{"unexpected":true}\n', encoding="utf-8")
        try:
            observed = run_verify("deny")
        finally:
            extra.unlink(missing_ok=True)

        self.assertEqual(2, observed.returncode)
        self.assertEqual(b"", observed.stdout)
        payload = json.loads(observed.stderr)
        self.assertEqual("file_set_invalid", payload["code"])
        self.assertEqual("phase-3", payload["phase"])

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

    def test_rejects_duplicate_trust_key_ids(self):
        trust_store = ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json"
        original = trust_store.read_bytes()
        try:
            payload = json.loads(original)
            payload["keys"].append(dict(payload["keys"][0]))
            trust_store.write_text(json.dumps(payload, sort_keys=True, separators=(",", ":")))
            observed = run_verify("allow")
        finally:
            trust_store.write_bytes(original)

        self.assertEqual(2, observed.returncode)
        self.assertEqual(b"", observed.stdout)
        payload = json.loads(observed.stderr)
        self.assertEqual("trust_store_invalid", payload["code"])

    def test_trust_store_canonical_parse_errors_are_not_schema_errors(self):
        trust_store = ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json"
        original = trust_store.read_bytes()
        try:
            trust_store.write_text(
                '{"version":"spendguard-agentrust-trust-store/v1","version":"duplicate","keys":[]}\n',
                encoding="utf-8",
            )
            observed = run_verify("allow")
        finally:
            trust_store.write_bytes(original)

        self.assertEqual(2, observed.returncode)
        self.assertEqual(b"", observed.stdout)
        payload = json.loads(observed.stderr)
        self.assertEqual("canonical_json_invalid", payload["code"])

    def test_bundle_noncanonical_bytes_fail_after_index_parse(self):
        bundle_path = ROOT / "integrations" / "agentrust" / "examples" / "allow" / "bundle.json"
        original = bundle_path.read_bytes()
        try:
            bundle = json.loads(original)
            reordered = {
                "version": bundle["version"],
                "tenant_id": bundle["tenant_id"],
                "run_id": bundle["run_id"],
                "files": bundle["files"],
                "evidence_bundle_hash": bundle["evidence_bundle_hash"],
                "decision_path": bundle["decision_path"],
                "decision_id": bundle["decision_id"],
                "created_at": bundle["created_at"],
                "bundle_id": bundle["bundle_id"],
            }
            bundle_path.write_text(json.dumps(reordered, separators=(",", ":")) + "\n")
            observed = run_verify("allow")
        finally:
            bundle_path.write_bytes(original)

        self.assertEqual(2, observed.returncode)
        self.assertEqual(b"", observed.stdout)
        payload = json.loads(observed.stderr)
        self.assertEqual("canonical_json_invalid", payload["code"])

    def test_bundle_media_type_mismatch_fails_before_evidence_root(self):
        bundle_path = ROOT / "integrations" / "agentrust" / "examples" / "allow" / "bundle.json"
        original = bundle_path.read_bytes()
        try:
            bundle = json.loads(original)
            bundle["files"][0]["media_type"] = "application/json"
            bundle_path.write_text(json.dumps(bundle, sort_keys=True, separators=(",", ":")))
            observed = run_verify("allow")
        finally:
            bundle_path.write_bytes(original)

        self.assertEqual(2, observed.returncode)
        self.assertEqual(b"", observed.stdout)
        payload = json.loads(observed.stderr)
        self.assertEqual("media_type_invalid", payload["code"])

    def test_trust_store_rejects_invalid_shape_through_public_cli(self):
        trust_store = ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json"
        original = trust_store.read_bytes()
        cases = {
            "empty_subject_prefix": lambda payload: payload["keys"][0].update(
                {"allowed_subject_prefixes": [""]}
            ),
            "unknown_key_field": lambda payload: payload["keys"][0].update({"unexpected": True}),
            "missing_environment": lambda payload: payload["keys"][0].pop("environment"),
            "unselected_bad_public_key": lambda payload: payload["keys"].append(
                {
                    **payload["keys"][0],
                    "key_id": "unused-key",
                    "public_key_base64url": "@@@",
                }
            ),
        }
        for label, mutate in cases.items():
            with self.subTest(label=label):
                payload = json.loads(original)
                mutate(payload)
                try:
                    trust_store.write_text(
                        json.dumps(payload, sort_keys=True, separators=(",", ":"))
                    )
                    observed = run_verify("allow")
                finally:
                    trust_store.write_bytes(original)

                self.assertEqual(2, observed.returncode, observed.stderr)
                self.assertEqual(b"", observed.stdout)
                diagnostic = json.loads(observed.stderr)
                self.assertEqual("trust_store_invalid", diagnostic["code"])

    def test_signed_event_rejects_data_json_data_b64_mismatch(self):
        from spendguard_agentrust.verify import VerificationError, _verify_signed_event

        decision = json.loads(
            (ROOT / "integrations" / "agentrust" / "examples" / "allow" / "decision.json").read_text()
        )
        trust_store = json.loads(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_text()
        )
        decision["cloudevent"]["data_json"]["spendguard"]["policy_bundle_hash"] = (
            "sha256:" + "9" * 64
        )

        with self.assertRaises(VerificationError) as caught:
            _verify_signed_event(decision, trust_store)

        self.assertEqual("data_b64_mismatch", caught.exception.diagnostic.code)
        self.assertEqual(2, caught.exception.diagnostic.exit_code)
        self.assertEqual("phase-8", caught.exception.diagnostic.phase)
        self.assertEqual("/cloudevent/data_b64", caught.exception.diagnostic.json_pointer)

    def test_signed_event_rejects_unsigned_subject_as_schema_invalid(self):
        from spendguard_agentrust.verify import VerificationError, _verify_signed_event

        base_decision = json.loads(
            (ROOT / "integrations" / "agentrust" / "examples" / "allow" / "decision.json").read_text()
        )
        trust_store = json.loads(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_text()
        )
        cases = ("spiffe://wrong.example/tenant/run", {"bad": "subject"})
        for subject in cases:
            with self.subTest(subject=subject):
                decision = json.loads(json.dumps(base_decision))
                decision["cloudevent"]["subject"] = subject

                with self.assertRaises(VerificationError) as caught:
                    _verify_signed_event(decision, trust_store)

                self.assertEqual("schema_invalid", caught.exception.diagnostic.code)
                self.assertEqual(2, caught.exception.diagnostic.exit_code)
                self.assertEqual("phase-7", caught.exception.diagnostic.phase)
                self.assertEqual("/subject", caught.exception.diagnostic.json_pointer)

    def test_signed_event_schema_rejects_malformed_cloudevent_fields_before_canonicalization(self):
        from spendguard_agentrust.verify import VerificationError, _validate_signed_event_shape

        base_decision = json.loads(
            (ROOT / "integrations" / "agentrust" / "examples" / "allow" / "decision.json").read_text()
        )
        cases = {
            "non_string_source": lambda event: event["cloudevent"].update({"source": 123}),
            "bad_datacontenttype": lambda event: event["cloudevent"].update({"datacontenttype": ""}),
            "producer_sequence_too_large": lambda event: event["cloudevent"].update(
                {"producer_sequence": 9223372036854775808}
            ),
        }
        for label, mutate in cases.items():
            with self.subTest(label=label):
                event = json.loads(json.dumps(base_decision))
                mutate(event)

                with self.assertRaises(VerificationError) as caught:
                    _validate_signed_event_shape(event)

                self.assertEqual("schema_invalid", caught.exception.diagnostic.code)
                self.assertEqual(2, caught.exception.diagnostic.exit_code)
                self.assertEqual("phase-7", caught.exception.diagnostic.phase)

    def test_signed_event_requires_exact_three_digit_rfc3339_millis(self):
        from spendguard_agentrust.verify import VerificationError, _ledger_canonical_bytes

        base_decision = json.loads(
            (ROOT / "integrations" / "agentrust" / "examples" / "allow" / "decision.json").read_text()
        )
        for value in ("2026-07-01T00:00:00.0Z", "2026-07-01T00:00:00.001000Z"):
            with self.subTest(value=value):
                decision = json.loads(json.dumps(base_decision))
                decision["cloudevent"]["time"] = value

                with self.assertRaises(VerificationError) as caught:
                    _ledger_canonical_bytes(decision)

                self.assertEqual("canonical_json_invalid", caught.exception.diagnostic.code)
                self.assertEqual("phase-8", caught.exception.diagnostic.phase)
                self.assertEqual("/cloudevent/time", caught.exception.diagnostic.json_pointer)

    def test_trace_signing_key_rejects_extra_pem_blocks(self):
        from spendguard_agentrust.verify import VerificationError, _verify_trace_signing_key

        key_bytes = (
            ROOT
            / "integrations"
            / "agentrust"
            / "test-private-keys"
            / "spendguard-agentrust-fixture.test.ed25519.pem"
        ).read_bytes()

        with self.assertRaises(VerificationError) as caught:
            _verify_trace_signing_key(key_bytes + key_bytes, "allow")

        self.assertEqual("trace_key_invalid", caught.exception.diagnostic.code)

    def test_canonical_json_loader_rejects_duplicate_keys(self):
        from spendguard_agentrust.diagnostics import Diagnostic
        from spendguard_agentrust.verify import VerificationError, _loads_canonical_json

        for payload in (
            b'{"a":1,"a":2}',
            b'{"a":1}\n',
            b'{"a":NaN}',
            b'{"a":Infinity}',
            b'{"a":-Infinity}',
        ):
            with self.subTest(payload=payload):
                with self.assertRaises(VerificationError) as caught:
                    _loads_canonical_json(
                        payload,
                        Diagnostic("canonical_json_invalid", 2, "phase-7"),
                    )

                self.assertEqual("canonical_json_invalid", caught.exception.diagnostic.code)
                self.assertEqual("phase-7", caught.exception.diagnostic.phase)

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
        self.assertEqual(b"", observed.stdout)
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
