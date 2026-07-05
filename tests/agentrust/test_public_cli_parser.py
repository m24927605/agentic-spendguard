import json
import os
import subprocess
import sys
import unittest

from spendguard_agentrust.cli import HELP_STDOUT, parse_public_args


ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
VALID_ARGS = [
    "verify",
    "--bundle",
    "integrations/agentrust/examples/allow",
    "--trust-store",
    "integrations/agentrust/examples/trust-store.json",
    "--trace-signing-key",
    "integrations/agentrust/test-private-keys/spendguard-agentrust-fixture.test.ed25519.pem",
    "--out-dir",
    "integrations/agentrust/out",
]
EXPECTED_HELP = (
    "usage: python -m spendguard_agentrust verify --bundle BUNDLE "
    "--trust-store TRUST_STORE --trace-signing-key TRACE_SIGNING_KEY "
    "--out-dir OUT_DIR [--mode fixture] [--fixture-profile upstream] "
    "[--manifest-issuer SPIFFE_URI]\n"
)


def run_cli(*args):
    env = os.environ.copy()
    env["PYTHONPATH"] = ROOT
    env["PYTHONNOUSERSITE"] = "1"
    return subprocess.run(
        [sys.executable, "-m", "spendguard_agentrust", *args],
        cwd=ROOT,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


class PublicCliParserTest(unittest.TestCase):
    def assert_diagnostic(self, observed, code):
        self.assertEqual(2, observed.returncode)
        self.assertEqual("", observed.stdout)
        payload = json.loads(observed.stderr)
        self.assertEqual(
            [
                "schema",
                "exit_code",
                "phase",
                "code",
                "message",
                "json_pointer",
            ],
            list(payload.keys()),
        )
        self.assertEqual("spendguard-agentrust-diagnostic/v1", payload["schema"])
        self.assertEqual(2, payload["exit_code"])
        self.assertEqual("phase-1", payload["phase"])
        self.assertEqual(code, payload["code"])
        self.assertEqual(code.replace("_", " "), payload["message"])
        self.assertIsNone(payload["json_pointer"])

    def test_exact_help_stdout(self):
        self.assertEqual(EXPECTED_HELP, HELP_STDOUT)

        observed = run_cli("verify", "--help")

        self.assertEqual(0, observed.returncode)
        self.assertEqual(EXPECTED_HELP, observed.stdout)
        self.assertEqual("", observed.stderr)

    def test_parser_precedence_examples(self):
        cases = [
            (["verify", "--mode", "--force"], "cli_missing_value"),
            (
                ["verify", "--tested-against", "foo", "--bundle", "x"],
                "unsupported_capability",
            ),
            (
                ["verify", "--bundle", "value", "extra", "--mode", "live"],
                "cli_parse_error",
            ),
            (["verify", "--unknown", "--force"], "unsupported_capability"),
            (["verify", "--force", "--unknown"], "unsupported_capability"),
            (["verify", "--tested-against", "--unknown"], "unsupported_capability"),
            (["verify", "--mode", "live", "extra"], "unsupported_capability"),
            (["verify", "--force", "--force"], "unsupported_capability"),
            (["verify", "extra", "--force"], "cli_parse_error"),
            (["verify", "--fixture-profile=upstream"], "cli_equals_form_unsupported"),
            (["verify", "--mode=fixture"], "cli_equals_form_unsupported"),
            (["verify", "--help", "--bundle", "x"], "cli_parse_error"),
            (["verify", "--help", "--help"], "cli_parse_error"),
            (["verify", "--help=x"], "cli_parse_error"),
            (["--bundle", "x", "verify"], "cli_parse_error"),
            ([], "cli_parse_error"),
        ]

        for argv, code in cases:
            with self.subTest(argv=argv):
                self.assert_diagnostic(run_cli(*argv), code)

    def test_supported_literals_and_defaults(self):
        parsed = parse_public_args(VALID_ARGS)

        self.assertEqual("ok", parsed.kind)
        self.assertEqual("integrations/agentrust/examples/allow", parsed.values["bundle"])
        self.assertEqual("fixture", parsed.values["mode"])
        self.assertEqual("upstream", parsed.values["fixture_profile"])
        self.assertEqual(
            "spiffe://spendguard.local/agentrust-exporter",
            parsed.values["manifest_issuer"],
        )

    def test_required_flags_duplicates_equals_and_values(self):
        cases = [
            (["verify", "--bundle", "integrations/agentrust/examples/allow"], "cli_missing_required_flag"),
            (
                VALID_ARGS + ["--bundle", "integrations/agentrust/examples/deny"],
                "cli_parse_error",
            ),
            (
                [
                    "verify",
                    "--bundle=integrations/agentrust/examples/allow",
                    "--trust-store",
                    "integrations/agentrust/examples/trust-store.json",
                    "--trace-signing-key",
                    "integrations/agentrust/test-private-keys/spendguard-agentrust-fixture.test.ed25519.pem",
                    "--out-dir",
                    "integrations/agentrust/out",
                ],
                "cli_equals_form_unsupported",
            ),
            (
                [
                    *VALID_ARGS[:2],
                    "./integrations/agentrust/examples/allow",
                    *VALID_ARGS[3:],
                ],
                "cli_value_invalid",
            ),
            (
                [
                    *VALID_ARGS[:6],
                    "integrations/agentrust/test-private-keys/spendguard-agentrust-wrong.test.ed25519.pem",
                    *VALID_ARGS[7:],
                ],
                "cli_value_invalid",
            ),
        ]

        for argv, code in cases:
            with self.subTest(argv=argv):
                parsed = parse_public_args(argv)
                self.assertEqual("diagnostic", parsed.kind)
                self.assertEqual(code, parsed.diagnostic.code)

    def test_unrepresentable_tokens_are_classified_without_subprocess(self):
        value_with_nul = [*VALID_ARGS]
        value_with_nul[2] = "bad\x00value"
        parsed = parse_public_args(value_with_nul)
        self.assertEqual("diagnostic", parsed.kind)
        self.assertEqual("cli_value_invalid", parsed.diagnostic.code)

        parsed = parse_public_args(["verify", "\udcff"])
        self.assertEqual("diagnostic", parsed.kind)
        self.assertEqual("cli_parse_error", parsed.diagnostic.code)

    def test_manifest_issuer_grammar(self):
        valid = [*VALID_ARGS, "--manifest-issuer", "spiffe://agent.example/a/%7E"]
        parsed = parse_public_args(valid)
        self.assertEqual("ok", parsed.kind)
        self.assertEqual("spiffe://agent.example/a/%7E", parsed.values["manifest_issuer"])

        for issuer in [
            "spiffe://Agent.example/a",
            "spiffe://agent.example/a/%7e",
            "spiffe://agent.example/a/%2F",
            "spiffe://agent.example/a/%20",
            "spiffe://agent.example/a//b",
            "spiffe://agent.example/a?x=1",
        ]:
            with self.subTest(issuer=issuer):
                parsed = parse_public_args([*VALID_ARGS, "--manifest-issuer", issuer])
                self.assertEqual("diagnostic", parsed.kind)
                self.assertEqual("cli_value_invalid", parsed.diagnostic.code)


if __name__ == "__main__":
    unittest.main()
