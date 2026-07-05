from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "agentrust-fixture.yml"


class AgenTrustFixtureWorkflowTest(unittest.TestCase):
    def test_workflow_runs_fixture_mvp_checks(self):
        self.assertTrue(WORKFLOW.is_file(), f"missing workflow: {WORKFLOW}")
        text = WORKFLOW.read_text()

        expected_fragments = [
            "name: AgenTrust fixture CI",
            "pull_request:",
            "push:",
            "branches: [main]",
            "permissions:",
            "contents: read",
            "group: ${{ github.workflow }}-${{ github.ref }}",
            "cancel-in-progress: true",
            "runs-on: ubuntu-latest",
            "timeout-minutes: 10",
            "uses: actions/checkout@v4",
            "uses: actions/setup-python@v5",
            'python-version: "3.12"',
            "python3 docs/specs/check-agentrust-vectors.py",
            'PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust',
            'PYTHON=python3 PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 make verify-agentrust',
            "git diff --check origin/main...HEAD",
            '".github/workflows/agentrust-fixture.yml"',
            '"spendguard_agentrust/**"',
            '"tests/agentrust/**"',
            '"integrations/agentrust/**"',
            '"docs/specs/agentrust-*"',
        ]

        for fragment in expected_fragments:
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, text)


if __name__ == "__main__":
    unittest.main()
