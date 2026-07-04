import os
from pathlib import Path
import shutil
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parents[2]
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

    def test_make_verify_agentrust_success_streams(self):
        env = os.environ.copy()
        env["PYTHONNOUSERSITE"] = "1"
        observed = subprocess.run(
            ["make", "verify-agentrust", f"PYTHON={sys.executable}"],
            cwd=ROOT,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

        self.assertEqual(0, observed.returncode)
        self.assertEqual("agentrust_make_verify=pass\n", observed.stdout)
        self.assertEqual("", observed.stderr)
        reports = sorted(REPORT_ROOT.glob("*/verify-agentrust-report.json"))
        self.assertEqual(1, len(reports))
        payload = reports[0].read_text()
        self.assertTrue(payload.endswith("\n"))
        report = __import__("json").loads(payload)
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


if __name__ == "__main__":
    unittest.main()
