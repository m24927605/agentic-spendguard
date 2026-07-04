import os
from pathlib import Path
import shutil
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parents[2]
OUT_ROOT = ROOT / "integrations" / "agentrust" / "out"


class MakeVerifyTargetTest(unittest.TestCase):
    def setUp(self):
        for bundle_id in ("demo-allow", "demo-deny"):
            shutil.rmtree(OUT_ROOT / bundle_id, ignore_errors=True)

    def tearDown(self):
        for bundle_id in ("demo-allow", "demo-deny"):
            shutil.rmtree(OUT_ROOT / bundle_id, ignore_errors=True)

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


if __name__ == "__main__":
    unittest.main()
