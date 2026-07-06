import json
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parents[2]
RUN_ROOT = ROOT / "integrations" / "agentrust" / "out" / ".verify-agentrust" / "agentrust-20260706T000001Z-abcdef123457"
RUN_ROOT_REL = RUN_ROOT.relative_to(ROOT).as_posix()
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
    TRACE_KEY.chmod(0o600)


def reset_run_root():
    shutil.rmtree(RUN_ROOT, ignore_errors=True)
    prepare_trace_key()


def copy_fixture_bundle(fixture: str, target: Path):
    shutil.copytree(ROOT / "integrations" / "agentrust" / "examples" / fixture, target)


def rewrite_bundle_with_wrong_evidence_root(bundle_path: Path):
    bundle = json.loads(bundle_path.read_text())
    bundle["created_at"] = "2026-07-01T00:00:01.000Z"
    preimage = dict(bundle)
    preimage.pop("evidence_bundle_hash", None)
    digest = hashlib.sha256(
        json.dumps(preimage, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()
    bundle["evidence_bundle_hash"] = "sha256:" + digest
    bundle_path.write_text(json.dumps(bundle, sort_keys=True, separators=(",", ":")))


def repo_relative(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


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
        reset_run_root()

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
            repo_relative(TRACE_KEY),
            "--out-dir",
            RUN_ROOT_REL,
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
        mutation_root.mkdir(parents=True)
        copy_fixture_bundle("allow", mutation_root / "bundle")
        (mutation_root / "out").mkdir()
        trust_store = mutation_root / "trust-store.json"
        trust_store.write_bytes(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
        )
        observed = run_internal(
            [
                "verify",
                "--bundle",
                repo_relative(mutation_root / "bundle"),
                "--trust-store",
                repo_relative(trust_store),
                "--trace-signing-key",
                repo_relative(TRACE_KEY),
                "--out-dir",
                repo_relative(mutation_root / "out"),
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

    def test_mutated_signature_fixture_prefix_is_pinned(self):
        from spendguard_agentrust._verify_internal import _flip_base64url_prefix
        from spendguard_agentrust.verify import VerificationError

        self.assertEqual("A62Y2", _flip_base64url_prefix("B62Y2"))
        with self.assertRaises(VerificationError) as caught:
            _flip_base64url_prefix("A62Y2")
        self.assertEqual("negative_mutation_setup_invalid", caught.exception.diagnostic.code)

    def test_private_rejects_mutation_fixture_mismatch_before_boundary_validation(self):
        cases = (("mutated_signature", "deny"), ("wrong_key", "allow"))
        for mutation, fixture in cases:
            with self.subTest(mutation=mutation, fixture=fixture):
                reset_run_root()
                mutation_root = RUN_ROOT / ".mutations" / mutation / fixture
                observed = run_internal(
                    [
                        "verify",
                        "--bundle",
                        repo_relative(mutation_root / "bundle"),
                        "--trust-store",
                        repo_relative(mutation_root / "trust-store.json"),
                        "--trace-signing-key",
                        repo_relative(TRACE_KEY),
                        "--out-dir",
                        repo_relative(mutation_root / "out"),
                        "--fixture",
                        fixture,
                        "--mutation",
                        mutation,
                    ]
                )

                self.assertEqual(2, observed.returncode)
                self.assertEqual(b"", observed.stdout)
                payload = json.loads(observed.stderr)
                self.assertEqual("cli_value_invalid", payload["code"])

    def test_private_mutation_missing_files_emit_diagnostic_json(self):
        cases = (
            ("mutated_signature", "allow", "decision.json", "file_set_invalid"),
            ("mutated_policy_hash", "allow", "policy-bundle.json", "file_set_invalid"),
            ("mutated_signature", "allow", "bundle.json", "bundle_index_invalid"),
        )
        for mutation, fixture, missing_name, expected_code in cases:
            with self.subTest(mutation=mutation, missing_name=missing_name):
                reset_run_root()
                mutation_root = RUN_ROOT / ".mutations" / mutation / fixture
                mutation_root.mkdir(parents=True)
                copy_fixture_bundle(fixture, mutation_root / "bundle")
                (mutation_root / "bundle" / missing_name).unlink()
                (mutation_root / "out").mkdir()
                trust_store = mutation_root / "trust-store.json"
                trust_store.write_bytes(
                    (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
                )

                observed = run_internal(
                    [
                        "verify",
                        "--bundle",
                        repo_relative(mutation_root / "bundle"),
                        "--trust-store",
                        repo_relative(trust_store),
                        "--trace-signing-key",
                        repo_relative(TRACE_KEY),
                        "--out-dir",
                        repo_relative(mutation_root / "out"),
                        "--fixture",
                        fixture,
                        "--mutation",
                        mutation,
                    ]
                )

                self.assertEqual(2, observed.returncode, observed.stderr)
                self.assertEqual(b"", observed.stdout)
                self.assertNotIn(b"Traceback", observed.stderr)
                payload = json.loads(observed.stderr)
                self.assertEqual(expected_code, payload["code"])
                self.assertEqual([], list((mutation_root / "out").iterdir()))

    def test_bundle_mutations_reject_preexisting_evidence_root_drift_before_mutation(self):
        cases = (("mutated_signature", "decision.json"), ("mutated_policy_hash", "policy-bundle.json"))
        for mutation, mutated_name in cases:
            with self.subTest(mutation=mutation):
                reset_run_root()
                mutation_root = RUN_ROOT / ".mutations" / mutation / "allow"
                mutation_root.mkdir(parents=True)
                copy_fixture_bundle("allow", mutation_root / "bundle")
                rewrite_bundle_with_wrong_evidence_root(mutation_root / "bundle" / "bundle.json")
                originals = {
                    path.relative_to(mutation_root).as_posix(): path.read_bytes()
                    for path in (mutation_root / "bundle").iterdir()
                    if path.is_file()
                }
                (mutation_root / "out").mkdir()
                trust_store = mutation_root / "trust-store.json"
                trust_store.write_bytes(
                    (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
                )
                originals["trust-store.json"] = trust_store.read_bytes()

                observed = run_internal(
                    [
                        "verify",
                        "--bundle",
                        repo_relative(mutation_root / "bundle"),
                        "--trust-store",
                        repo_relative(trust_store),
                        "--trace-signing-key",
                        repo_relative(TRACE_KEY),
                        "--out-dir",
                        repo_relative(mutation_root / "out"),
                        "--fixture",
                        "allow",
                        "--mutation",
                        mutation,
                    ]
                )

                self.assertEqual(4, observed.returncode, observed.stderr)
                self.assertEqual(b"", observed.stdout)
                payload = json.loads(observed.stderr)
                self.assertEqual("evidence_bundle_hash_mismatch", payload["code"])
                self.assertEqual(
                    originals[f"bundle/{mutated_name}"],
                    (mutation_root / "bundle" / mutated_name).read_bytes(),
                )
                for relative, data in originals.items():
                    self.assertEqual(data, (mutation_root / relative).read_bytes(), relative)
                self.assertEqual([], list((mutation_root / "out").iterdir()))

    def test_private_mutation_reordered_bundle_fails_before_mutation(self):
        mutation_root = RUN_ROOT / ".mutations" / "mutated_policy_hash" / "allow"
        mutation_root.mkdir(parents=True)
        copy_fixture_bundle("allow", mutation_root / "bundle")
        bundle_path = mutation_root / "bundle" / "bundle.json"
        bundle = json.loads(bundle_path.read_text())
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
        policy_path = mutation_root / "bundle" / "policy-bundle.json"
        original_policy = policy_path.read_bytes()
        (mutation_root / "out").mkdir()
        trust_store = mutation_root / "trust-store.json"
        trust_store.write_bytes(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
        )

        observed = run_internal(
            [
                "verify",
                "--bundle",
                repo_relative(mutation_root / "bundle"),
                "--trust-store",
                repo_relative(trust_store),
                "--trace-signing-key",
                repo_relative(TRACE_KEY),
                "--out-dir",
                repo_relative(mutation_root / "out"),
                "--fixture",
                "allow",
                "--mutation",
                "mutated_policy_hash",
            ]
        )

        self.assertEqual(2, observed.returncode, observed.stderr)
        self.assertEqual(b"", observed.stdout)
        self.assertNotIn(b"Traceback", observed.stderr)
        payload = json.loads(observed.stderr)
        self.assertEqual("canonical_json_invalid", payload["code"])
        self.assertEqual(original_policy, policy_path.read_bytes())
        self.assertEqual([], list((mutation_root / "out").iterdir()))

    def test_private_mutation_tampered_decision_bytes_fail_before_mutation(self):
        mutation_root = RUN_ROOT / ".mutations" / "mutated_signature" / "allow"
        mutation_root.mkdir(parents=True)
        copy_fixture_bundle("allow", mutation_root / "bundle")
        decision_path = mutation_root / "bundle" / "decision.json"
        decision_path.write_text('{"schema":"x","schema":"y"}\n', encoding="utf-8")
        (mutation_root / "out").mkdir()
        trust_store = mutation_root / "trust-store.json"
        trust_store.write_bytes(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
        )

        observed = run_internal(
            [
                "verify",
                "--bundle",
                repo_relative(mutation_root / "bundle"),
                "--trust-store",
                repo_relative(trust_store),
                "--trace-signing-key",
                repo_relative(TRACE_KEY),
                "--out-dir",
                repo_relative(mutation_root / "out"),
                "--fixture",
                "allow",
                "--mutation",
                "mutated_signature",
            ]
        )

        self.assertEqual(4, observed.returncode, observed.stderr)
        self.assertEqual(b"", observed.stdout)
        self.assertNotIn(b"Traceback", observed.stderr)
        payload = json.loads(observed.stderr)
        self.assertEqual("file_hash_mismatch", payload["code"])
        self.assertEqual("phase-6", payload["phase"])
        self.assertEqual([], list((mutation_root / "out").iterdir()))

    def test_non_bundle_mutations_keep_strict_fixture_root(self):
        cases = (("wrong_key", "deny"), ("missing_trace_signature", "deny"))
        for mutation, fixture in cases:
            with self.subTest(mutation=mutation):
                reset_run_root()
                mutation_root = RUN_ROOT / ".mutations" / mutation / fixture
                mutation_root.mkdir(parents=True)
                copy_fixture_bundle(fixture, mutation_root / "bundle")
                rewrite_bundle_with_wrong_evidence_root(mutation_root / "bundle" / "bundle.json")
                (mutation_root / "out").mkdir()
                trust_store = mutation_root / "trust-store.json"
                trust_store.write_bytes(
                    (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
                )

                observed = run_internal(
                    [
                        "verify",
                        "--bundle",
                        repo_relative(mutation_root / "bundle"),
                        "--trust-store",
                        repo_relative(trust_store),
                        "--trace-signing-key",
                        repo_relative(TRACE_KEY),
                        "--out-dir",
                        repo_relative(mutation_root / "out"),
                        "--fixture",
                        fixture,
                        "--mutation",
                        mutation,
                    ]
                )

                self.assertEqual(4, observed.returncode, observed.stderr)
                self.assertEqual(b"", observed.stdout)
                payload = json.loads(observed.stderr)
                self.assertEqual("evidence_bundle_hash_mismatch", payload["code"])
                self.assertEqual([], list((mutation_root / "out").iterdir()))

    def test_wrong_key_rejects_preexisting_trust_store_drift_before_mutation(self):
        mutation_root = RUN_ROOT / ".mutations" / "wrong_key" / "deny"
        mutation_root.mkdir(parents=True)
        copy_fixture_bundle("deny", mutation_root / "bundle")
        (mutation_root / "out").mkdir()
        trust_store = mutation_root / "trust-store.json"
        payload = json.loads(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_text()
        )
        payload["keys"][0]["public_key_base64url"] = "GX9rI-FshTLGq8g4-s1ep4m-DHaykgM0A5v6iz02jWE"
        trust_store.write_text(json.dumps(payload, sort_keys=True, separators=(",", ":")))
        originals = {
            path.relative_to(mutation_root).as_posix(): path.read_bytes()
            for path in (mutation_root / "bundle").iterdir()
            if path.is_file()
        }
        originals["trust-store.json"] = trust_store.read_bytes()

        observed = run_internal(
            [
                "verify",
                "--bundle",
                repo_relative(mutation_root / "bundle"),
                "--trust-store",
                repo_relative(trust_store),
                "--trace-signing-key",
                repo_relative(TRACE_KEY),
                "--out-dir",
                repo_relative(mutation_root / "out"),
                "--fixture",
                "deny",
                "--mutation",
                "wrong_key",
            ]
        )

        self.assertEqual(6, observed.returncode, observed.stderr)
        self.assertEqual(b"", observed.stdout)
        diagnostic = json.loads(observed.stderr)
        self.assertEqual("negative_mutation_setup_invalid", diagnostic["code"])
        self.assertEqual("make-setup", diagnostic["phase"])
        for relative, data in originals.items():
            self.assertEqual(data, (mutation_root / relative).read_bytes(), relative)
        self.assertEqual([], list((mutation_root / "out").iterdir()))


if __name__ == "__main__":
    unittest.main()
