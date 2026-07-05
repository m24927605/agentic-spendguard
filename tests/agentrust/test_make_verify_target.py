import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parents[2]
MAKEFILE = ROOT / "Makefile"
OUT_ROOT = ROOT / "integrations" / "agentrust" / "out"
REPORT_ROOT = OUT_ROOT / ".verify-agentrust"


class MakeVerifyTargetTest(unittest.TestCase):
    def setUp(self):
        for bundle_id in ("demo-allow", "demo-deny"):
            shutil.rmtree(OUT_ROOT / bundle_id, ignore_errors=True)
        shutil.rmtree(REPORT_ROOT, ignore_errors=True)

    def tearDown(self):
        for bundle_id in ("demo-allow", "demo-deny"):
            shutil.rmtree(OUT_ROOT / bundle_id, ignore_errors=True)
        shutil.rmtree(REPORT_ROOT, ignore_errors=True)

    def _run_make(self, target: str):
        env = os.environ.copy()
        env["PYTHONNOUSERSITE"] = "1"
        return subprocess.run(
            ["make", target, f"PYTHON={sys.executable}"],
            cwd=ROOT,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def _run_make_module(self, *args: str):
        env = os.environ.copy()
        env["PYTHONPATH"] = str(ROOT)
        env["PYTHONNOUSERSITE"] = "1"
        return subprocess.run(
            [sys.executable, "-m", "spendguard_agentrust._make_verify", *args],
            cwd=ROOT,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def test_make_verify_agentrust_success_streams(self):
        observed = self._run_make("verify-agentrust")

        self.assertEqual(0, observed.returncode)
        self.assertEqual("agentrust_make_verify=pass\n", observed.stdout)
        self.assertEqual("", observed.stderr)
        reports = sorted(REPORT_ROOT.glob("*/verify-agentrust-report.json"))
        self.assertEqual(1, len(reports))
        payload = reports[0].read_text()
        self.assertTrue(payload.endswith("\n"))
        report = json.loads(payload)
        self.assertEqual(
            [
                "conformance_checks",
                "mode",
                "negative_tests",
                "package_versions",
                "schema",
                "success_fixtures",
                "status",
            ],
            list(report.keys()),
        )
        self.assertEqual("pass", report["status"])
        self.assertEqual(
            {
                "artifact_preflight_envelope",
                "event_type_binding",
                "manifest_issuer_variant",
                "private_parser_matrix",
                "public_parser_matrix",
            },
            {
                key
                for key, value in report["conformance_checks"].items()
                if value["status"] == "deferred_fixture_mvp"
            },
        )
        self.assertEqual("pass", report["conformance_checks"]["fixture_identity"]["status"])
        self.assertEqual("pass", report["conformance_checks"]["internal_guard"]["status"])
        self.assertEqual("pass", report["conformance_checks"]["package_guard_surfaces"]["status"])

    def test_fixture_target_keeps_smoke_behavior(self):
        observed = self._run_make("verify-agentrust-fixture")

        self.assertEqual(0, observed.returncode)
        self.assertEqual("agentrust_make_verify=pass\n", observed.stdout)
        self.assertEqual("", observed.stderr)

    def test_make_target_migration_names_are_available_before_full_promotion(self):
        makefile = MAKEFILE.read_text()

        expected_targets = [
            "verify-agentrust:",
            "verify-agentrust-fixture:",
            "verify-agentrust-full:",
            "verify-agentrust-preflight:",
            "verify-agentrust-output-lane:",
            "verify-agentrust-negative-lane:",
            "verify-agentrust-report-lane:",
            "verify-agentrust-artifact-lane:",
        ]
        for target in expected_targets:
            with self.subTest(target=target):
                self.assertIn(target, makefile)

        verify_target_body = makefile.split("verify-agentrust:", 1)[1].split("\n\n", 1)[0]
        self.assertIn("verify-agentrust-fixture", verify_target_body)
        self.assertNotIn("verify-agentrust-full", verify_target_body)

    def test_raw_full_target_pending_diagnostic_is_json_only(self):
        observed = self._run_make_module("verify-full")

        self.assertEqual(6, observed.returncode)
        self.assertEqual("", observed.stdout)
        diagnostic = json.loads(observed.stderr)
        self.assertEqual("spendguard-agentrust-diagnostic/v1", diagnostic["schema"])
        self.assertEqual(6, diagnostic["exit_code"])
        self.assertEqual("deferred_fixture_mvp", diagnostic["code"])
        self.assertIsNone(diagnostic["json_pointer"])

    def test_make_full_target_fails_pending_without_stdout(self):
        observed = self._run_make("verify-agentrust-full")

        self.assertNotEqual(0, observed.returncode)
        self.assertEqual("", observed.stdout)
        self.assertIn("deferred_fixture_mvp", observed.stderr)


if __name__ == "__main__":
    unittest.main()
