import json
import contextlib
import io
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
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
        run_root = reports[0].parent
        self.assertFalse(os.path.lexists(run_root / ".trace-key"))
        self.assertFalse(os.path.lexists(run_root / ".mutations"))
        self.assertEqual(
            {
                "missing_trace_signature",
                "mutated_policy_hash",
                "mutated_signature",
                "wrong_key",
            },
            set(report["negative_tests"]),
        )
        for result in report["negative_tests"].values():
            self.assertEqual("pass", result["status"])
            self.assertTrue(result["outputs_absent"])

    def test_make_trace_key_copy_is_private_and_cleanup_removes_it(self):
        from spendguard_agentrust._make_verify import (
            _cleanup_sensitive_workdirs,
            _new_run_root,
            _prepare_trace_key,
        )

        run_root = _new_run_root()
        key = run_root / ".trace-key" / "trace-exporter.test.ed25519.pem"
        try:
            _prepare_trace_key(run_root)

            dir_status = (run_root / ".trace-key").lstat()
            self.assertEqual(0o700, dir_status.st_mode & 0o777)
            status = key.lstat()
            self.assertEqual(0o600, status.st_mode & 0o777)
            self.assertEqual(1, status.st_nlink)
            self.assertFalse(key.is_symlink())
        finally:
            _cleanup_sensitive_workdirs(run_root)

        self.assertFalse(os.path.lexists(run_root / ".trace-key"))

    def test_make_cleanup_runs_after_failure(self):
        import spendguard_agentrust._make_verify as make_verify

        run_root = make_verify._new_run_root()
        original_new_run_root = make_verify._new_run_root
        original_check_package_guard = make_verify._check_package_guard

        def selected_run_root():
            return run_root

        def fail_after_trace_key_copy():
            raise AssertionError("forced package guard failure")

        make_verify._new_run_root = selected_run_root
        make_verify._check_package_guard = fail_after_trace_key_copy
        try:
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                observed = make_verify._run_fixture_smoke()
        finally:
            make_verify._new_run_root = original_new_run_root
            make_verify._check_package_guard = original_check_package_guard

        self.assertEqual(6, observed)
        diagnostic = json.loads(stderr.getvalue())
        self.assertEqual("spendguard-agentrust-diagnostic/v1", diagnostic["schema"])
        self.assertEqual("packaging_guard_failed", diagnostic["code"])
        self.assertEqual("make-setup", diagnostic["phase"])
        self.assertFalse(os.path.lexists(run_root / ".trace-key"))
        self.assertFalse(os.path.lexists(run_root / ".mutations"))

    def test_make_cleanup_runs_after_negative_failure(self):
        import spendguard_agentrust._make_verify as make_verify

        run_root = make_verify._new_run_root()
        originals = {
            "_new_run_root": make_verify._new_run_root,
            "_check_package_guard": make_verify._check_package_guard,
            "_check_help": make_verify._check_help,
            "_check_fixture": make_verify._check_fixture,
            "_check_private_success": make_verify._check_private_success,
            "_check_private_negative": make_verify._check_private_negative,
        }

        def selected_run_root():
            return run_root

        def noop(*args):
            return None

        def fail_after_mutation_workdir(run_root_arg, mutation, fixture):
            (run_root_arg / ".mutations" / mutation / fixture).mkdir(parents=True)
            raise AssertionError("forced negative failure")

        make_verify._new_run_root = selected_run_root
        make_verify._check_package_guard = noop
        make_verify._check_help = noop
        make_verify._check_fixture = noop
        make_verify._check_private_success = noop
        make_verify._check_private_negative = fail_after_mutation_workdir
        try:
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                observed = make_verify._run_fixture_smoke()
        finally:
            for name, original in originals.items():
                setattr(make_verify, name, original)

        self.assertEqual(1, observed)
        summary = json.loads(stderr.getvalue())
        self.assertEqual("spendguard-agentrust-make-summary/v1", summary["schema"])
        self.assertEqual("fail", summary["status"])
        self.assertEqual(
            make_verify._repo_relative(run_root / "verify-agentrust-report.json"),
            summary["report_path"],
        )
        report = json.loads((run_root / "verify-agentrust-report.json").read_text())
        self.assertEqual("fail", report["status"])
        self.assertEqual("pass", report["success_fixtures"]["allow"]["status"])
        self.assertTrue(report["success_fixtures"]["allow"]["outputs_match"])
        self.assertEqual("pass", report["success_fixtures"]["deny"]["status"])
        self.assertTrue(report["success_fixtures"]["deny"]["outputs_match"])
        self.assertFalse(os.path.lexists(run_root / ".trace-key"))
        self.assertFalse(os.path.lexists(run_root / ".mutations"))

    def test_make_run_root_setup_failure_emits_compact_diagnostic(self):
        import spendguard_agentrust._make_verify as make_verify

        original_new_run_root = make_verify._new_run_root

        def fail_new_run_root():
            raise OSError("forced report root failure")

        make_verify._new_run_root = fail_new_run_root
        try:
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                observed = make_verify._run_fixture_smoke()
        finally:
            make_verify._new_run_root = original_new_run_root

        self.assertEqual(2, observed)
        diagnostic = json.loads(stderr.getvalue())
        self.assertEqual("spendguard-agentrust-diagnostic/v1", diagnostic["schema"])
        self.assertEqual("output_target_invalid", diagnostic["code"])
        self.assertEqual("make-setup", diagnostic["phase"])

    def test_make_failure_report_records_completed_and_failed_negative_checks(self):
        import spendguard_agentrust._make_verify as make_verify

        run_root = make_verify._new_run_root()
        originals = {
            "_new_run_root": make_verify._new_run_root,
            "_check_package_guard": make_verify._check_package_guard,
            "_check_help": make_verify._check_help,
            "_check_fixture": make_verify._check_fixture,
            "_check_private_success": make_verify._check_private_success,
            "_run_internal": make_verify._run_internal,
        }

        def selected_run_root():
            return run_root

        def noop(*args):
            return None

        def fake_internal(*args):
            mutation = args[args.index("--mutation") + 1]
            expected_exit, expected_code, expected_phase, expected_pointer = make_verify.NEGATIVE_EXPECTATIONS[mutation]
            observed_exit = 0 if mutation == "wrong_key" else expected_exit
            payload = {
                "schema": "spendguard-agentrust-diagnostic/v1",
                "exit_code": observed_exit,
                "code": expected_code,
                "phase": expected_phase,
                "json_pointer": expected_pointer,
            }
            return subprocess.CompletedProcess(args, observed_exit, b"", json.dumps(payload).encode())

        make_verify._new_run_root = selected_run_root
        make_verify._check_package_guard = noop
        make_verify._check_help = noop
        make_verify._check_fixture = noop
        make_verify._check_private_success = noop
        make_verify._run_internal = fake_internal
        try:
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                observed = make_verify._run_fixture_smoke()
        finally:
            for name, original in originals.items():
                setattr(make_verify, name, original)

        self.assertEqual(1, observed)
        summary = json.loads(stderr.getvalue())
        self.assertEqual("fail", summary["status"])
        report = json.loads((run_root / "verify-agentrust-report.json").read_text())
        self.assertEqual("pass", report["success_fixtures"]["allow"]["status"])
        self.assertEqual("pass", report["success_fixtures"]["deny"]["status"])
        self.assertEqual("pass", report["negative_tests"]["mutated_signature"]["status"])
        self.assertEqual("fail", report["negative_tests"]["wrong_key"]["status"])
        self.assertEqual(3, report["negative_tests"]["wrong_key"]["expected_exit_code"])
        self.assertEqual(0, report["negative_tests"]["wrong_key"]["observed_exit_code"])
        self.assertEqual("pass", report["negative_tests"]["mutated_policy_hash"]["status"])
        self.assertEqual("pass", report["negative_tests"]["missing_trace_signature"]["status"])
        self.assertFalse(os.path.lexists(run_root / ".trace-key"))
        self.assertFalse(os.path.lexists(run_root / ".mutations"))

    def test_make_negative_phase_regression_fails_check(self):
        import spendguard_agentrust._make_verify as make_verify

        run_root = make_verify._new_run_root()
        originals = {
            "_new_run_root": make_verify._new_run_root,
            "_check_package_guard": make_verify._check_package_guard,
            "_check_help": make_verify._check_help,
            "_check_fixture": make_verify._check_fixture,
            "_check_private_success": make_verify._check_private_success,
            "_run_internal": make_verify._run_internal,
        }

        def selected_run_root():
            return run_root

        def noop(*args):
            return None

        def fake_internal(*args):
            mutation = args[args.index("--mutation") + 1]
            expected_exit, expected_code, expected_phase, expected_pointer = make_verify.NEGATIVE_EXPECTATIONS[mutation]
            payload = {
                "schema": "spendguard-agentrust-diagnostic/v1",
                "exit_code": expected_exit,
                "code": expected_code,
                "phase": "phase-9" if mutation == "mutated_policy_hash" else expected_phase,
                "json_pointer": expected_pointer,
            }
            return subprocess.CompletedProcess(args, expected_exit, b"", json.dumps(payload).encode())

        make_verify._new_run_root = selected_run_root
        make_verify._check_package_guard = noop
        make_verify._check_help = noop
        make_verify._check_fixture = noop
        make_verify._check_private_success = noop
        make_verify._run_internal = fake_internal
        try:
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                observed = make_verify._run_fixture_smoke()
        finally:
            for name, original in originals.items():
                setattr(make_verify, name, original)

        self.assertEqual(1, observed)
        report = json.loads((run_root / "verify-agentrust-report.json").read_text())
        self.assertEqual("fail", report["negative_tests"]["mutated_policy_hash"]["status"])
        self.assertEqual("pass", report["negative_tests"]["mutated_signature"]["status"])
        self.assertEqual("pass", report["negative_tests"]["wrong_key"]["status"])
        self.assertEqual("pass", report["negative_tests"]["missing_trace_signature"]["status"])
        self.assertFalse(os.path.lexists(run_root / ".trace-key"))
        self.assertFalse(os.path.lexists(run_root / ".mutations"))

    def test_make_final_report_write_failure_emits_diagnostic_json(self):
        import spendguard_agentrust._make_verify as make_verify

        run_root = make_verify._new_run_root()
        originals = {
            "_new_run_root": make_verify._new_run_root,
            "_check_package_guard": make_verify._check_package_guard,
            "_check_help": make_verify._check_help,
            "_check_fixture": make_verify._check_fixture,
            "_check_private_success": make_verify._check_private_success,
            "_check_private_negative": make_verify._check_private_negative,
            "_write_report": make_verify._write_report,
        }

        def selected_run_root():
            return run_root

        def noop(*args):
            return None

        def negative_pass(run_root_arg, mutation, fixture):
            expected_exit, expected_code, _, _ = make_verify.NEGATIVE_EXPECTATIONS[mutation]
            return {
                "diagnostic_code": expected_code,
                "expected_exit_code": expected_exit,
                "observed_exit_code": expected_exit,
                "outputs_absent": True,
                "status": "pass",
            }

        def fail_report_write(run_root_arg, negative_results):
            raise OSError("forced report write failure")

        make_verify._new_run_root = selected_run_root
        make_verify._check_package_guard = noop
        make_verify._check_help = noop
        make_verify._check_fixture = noop
        make_verify._check_private_success = noop
        make_verify._check_private_negative = negative_pass
        make_verify._write_report = fail_report_write
        try:
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                observed = make_verify._run_fixture_smoke()
        finally:
            for name, original in originals.items():
                setattr(make_verify, name, original)

        self.assertEqual(6, observed)
        self.assertTrue(stderr.getvalue().endswith("\n"))
        diagnostic = json.loads(stderr.getvalue())
        self.assertEqual("spendguard-agentrust-diagnostic/v1", diagnostic["schema"])
        self.assertEqual("packaging_guard_failed", diagnostic["code"])
        self.assertEqual("make-final", diagnostic["phase"])

    def test_make_failure_report_records_fixture_fail_and_continues(self):
        import spendguard_agentrust._make_verify as make_verify

        run_root = make_verify._new_run_root()
        originals = {
            "_new_run_root": make_verify._new_run_root,
            "_check_package_guard": make_verify._check_package_guard,
            "_check_help": make_verify._check_help,
            "_check_fixture": make_verify._check_fixture,
            "_check_private_success": make_verify._check_private_success,
            "_check_private_negative": make_verify._check_private_negative,
        }

        def selected_run_root():
            return run_root

        def noop(*args):
            return None

        def fixture_check(fixture):
            if fixture == "allow":
                raise AssertionError("forced allow fixture mismatch")

        def negative_pass(run_root_arg, mutation, fixture):
            expected_exit, expected_code, _, _ = make_verify.NEGATIVE_EXPECTATIONS[mutation]
            return {
                "diagnostic_code": expected_code,
                "expected_exit_code": expected_exit,
                "observed_exit_code": expected_exit,
                "outputs_absent": True,
                "status": "pass",
            }

        make_verify._new_run_root = selected_run_root
        make_verify._check_package_guard = noop
        make_verify._check_help = noop
        make_verify._check_fixture = fixture_check
        make_verify._check_private_success = noop
        make_verify._check_private_negative = negative_pass
        try:
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                observed = make_verify._run_fixture_smoke()
        finally:
            for name, original in originals.items():
                setattr(make_verify, name, original)

        self.assertEqual(1, observed)
        report = json.loads((run_root / "verify-agentrust-report.json").read_text())
        self.assertEqual("fail", report["success_fixtures"]["allow"]["status"])
        self.assertEqual("pass", report["success_fixtures"]["deny"]["status"])
        self.assertEqual("fail", report["conformance_checks"]["fixture_identity"]["status"])
        self.assertEqual("fail", report["conformance_checks"]["internal_guard"]["status"])
        self.assertTrue(
            all(result["status"] == "pass" for result in report["negative_tests"].values())
        )
        self.assertFalse(os.path.lexists(run_root / ".trace-key"))
        self.assertFalse(os.path.lexists(run_root / ".mutations"))

    def test_make_help_failure_is_recorded_and_later_checks_continue(self):
        import spendguard_agentrust._make_verify as make_verify

        run_root = make_verify._new_run_root()
        originals = {
            "_new_run_root": make_verify._new_run_root,
            "_check_package_guard": make_verify._check_package_guard,
            "_check_help": make_verify._check_help,
            "_check_fixture": make_verify._check_fixture,
            "_check_private_success": make_verify._check_private_success,
            "_check_private_negative": make_verify._check_private_negative,
        }

        def selected_run_root():
            return run_root

        def noop(*args):
            return None

        def fail_help():
            raise AssertionError("forced help failure")

        def negative_pass(run_root_arg, mutation, fixture):
            expected_exit, expected_code, _, _ = make_verify.NEGATIVE_EXPECTATIONS[mutation]
            return {
                "diagnostic_code": expected_code,
                "expected_exit_code": expected_exit,
                "observed_exit_code": expected_exit,
                "outputs_absent": True,
                "status": "pass",
            }

        make_verify._new_run_root = selected_run_root
        make_verify._check_package_guard = noop
        make_verify._check_help = fail_help
        make_verify._check_fixture = noop
        make_verify._check_private_success = noop
        make_verify._check_private_negative = negative_pass
        try:
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                observed = make_verify._run_fixture_smoke()
        finally:
            for name, original in originals.items():
                setattr(make_verify, name, original)

        self.assertEqual(1, observed)
        report = json.loads((run_root / "verify-agentrust-report.json").read_text())
        self.assertEqual("fail", report["conformance_checks"]["public_parser_matrix"]["status"])
        self.assertEqual("pass", report["success_fixtures"]["allow"]["status"])
        self.assertEqual("pass", report["success_fixtures"]["deny"]["status"])
        self.assertTrue(
            all(result["status"] == "pass" for result in report["negative_tests"].values())
        )
        self.assertFalse(os.path.lexists(run_root / ".trace-key"))
        self.assertFalse(os.path.lexists(run_root / ".mutations"))

    def test_make_trace_key_source_symlink_is_rejected_before_copy(self):
        import spendguard_agentrust._make_verify as make_verify

        run_root = make_verify._new_run_root()
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "trace-source.pem"
            symlink = Path(temp) / "trace-source-link.pem"
            source.write_bytes(make_verify.PUBLIC_TRACE_KEY.read_bytes())
            symlink.symlink_to(source)
            original_public_trace_key = make_verify.PUBLIC_TRACE_KEY
            make_verify.PUBLIC_TRACE_KEY = symlink
            try:
                with self.assertRaises(AssertionError):
                    make_verify._prepare_trace_key(run_root)
                self.assertFalse(
                    os.path.lexists(run_root / ".trace-key" / "trace-exporter.test.ed25519.pem")
                )
            finally:
                make_verify.PUBLIC_TRACE_KEY = original_public_trace_key
                make_verify._cleanup_sensitive_workdirs(run_root)

    def test_negative_outputs_absent_rejects_hidden_and_symlink_residue(self):
        from spendguard_agentrust._make_verify import _negative_outputs_absent

        cases = (
            ("final_bundle_symlink", "demo-allow"),
            ("output_broken_symlink", "trace-level0.json"),
            ("arbitrary_broken_symlink", "leftover"),
            ("temp_dir", ".tmp-leak"),
            ("probe_dir", ".probe-leak"),
            ("regular_file", "leak.json"),
            ("regular_dir", "debug"),
        )
        for label, residue_name in cases:
            with self.subTest(label=label):
                with tempfile.TemporaryDirectory() as temp:
                    mutation_root = Path(temp)
                    out_dir = mutation_root / "out"
                    out_dir.mkdir()
                    residue = out_dir / residue_name
                    if label.endswith("symlink"):
                        residue.symlink_to("missing-target")
                    elif label.endswith("file"):
                        residue.write_text("leak", encoding="utf-8")
                    else:
                        residue.mkdir()

                    self.assertFalse(_negative_outputs_absent(mutation_root, "allow"))

        with tempfile.TemporaryDirectory() as temp:
            mutation_root = Path(temp) / "mutation"
            target = Path(temp) / "target"
            mutation_root.mkdir()
            target.mkdir()
            (mutation_root / "out").symlink_to(target, target_is_directory=True)

            self.assertFalse(_negative_outputs_absent(mutation_root, "allow"))

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
        self.assertTrue(observed.stderr.endswith("\n"))
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
