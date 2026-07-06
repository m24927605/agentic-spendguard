import ast
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from spendguard_agentrust.fixture_outputs import ALL_OUTPUTS


ROOT = Path(__file__).resolve().parents[2]
OUT_ROOT = ROOT / "integrations" / "agentrust" / "out"
RUN_ROOT = OUT_ROOT / ".verify-agentrust" / "agentrust-20260706T000000Z-abcdef123456"
PUBLIC_KEY = (
    ROOT
    / "integrations"
    / "agentrust"
    / "test-private-keys"
    / "spendguard-agentrust-fixture.test.ed25519.pem"
)
TRACE_KEY = RUN_ROOT / ".trace-key" / "trace-exporter.test.ed25519.pem"
EVIDENCE_ENV = "SPENDGUARD_AGENTRUST_RUNTIME_READ_EVIDENCE"
PROBE_ENV = "SPENDGUARD_AGENTRUST_INTERNAL_READ_PROBE_PATH"

FORBIDDEN_RUNTIME_PREFIXES = (
    "docs/specs/",
    "packaging/",
    "proto/",
    "integrations/agentrust/examples/expected-outputs/",
)
FORBIDDEN_RUNTIME_EXACT = {
    "integrations/agentrust/examples/fixture-sha256-manifest.v1alpha1.json",
    "integrations/agentrust/examples/generated-output-sha256-manifest.v1alpha1.json",
    "integrations/agentrust/requirements-v1alpha1.in",
    "integrations/agentrust/requirements-v1alpha1.lock",
    "integrations/agentrust/test-private-keys/spendguard-agentrust-wrong.test.ed25519.pem",
}
FORBIDDEN_PROBES = {
    "vector": "docs/specs/agentrust-cloudevent-signature-vector-2026-07-01.json",
    "fixture_manifest": "integrations/agentrust/examples/fixture-sha256-manifest.v1alpha1.json",
    "generated_output_manifest": "integrations/agentrust/examples/"
    "generated-output-sha256-manifest.v1alpha1.json",
    "expected_output": "integrations/agentrust/examples/expected-outputs/allow/trace-level0.json",
    "package_guard_input": "packaging/agentrust_build_backend.py",
    "package_lock": "integrations/agentrust/requirements-v1alpha1.lock",
    "package_input": "integrations/agentrust/requirements-v1alpha1.in",
    "public_surface_scan_fixture": "tests/agentrust/test_build_backend.py",
}
RUNTIME_GUARD_FILES = tuple(
    sorted(
        path
        for path in (ROOT / "spendguard_agentrust").glob("*.py")
        if path.name != "_make_verify.py"
    )
)
ALLOWED_DIRECT_READ_FUNCTIONS = {"_read_regular_file_no_follow"}
ALLOWED_DIRECT_WRITE_FUNCTIONS = {
    "_verify_ed25519_signature_with_openssl",
    "_runtime_replace_bytes",
    "_runtime_write_bytes",
}
ALLOWED_REPO_OPEN_CALLERS = {
    "_mkdir_output_temp_dir",
    "_open_repo_dir_no_follow",
    "_open_repo_file_no_follow",
    "_open_repo_parent_dir_no_follow",
    "_read_regular_file_no_follow",
    "_reject_stale_output_temp_entries",
    "_runtime_replace_bytes",
    "_runtime_write_bytes",
    "_validate_bundle_file_set",
}
ALLOWED_DIRECT_OPEN_FUNCTIONS = {
    "_open_repo_dir_no_follow",
    "_open_repo_file_no_follow",
    "_open_root_dir_no_follow",
    "_runtime_replace_bytes",
    "_write_runtime_read_evidence",
}
FORBIDDEN_FILE_ATTRIBUTES = {
    "open",
    "read",
    "read_bytes",
    "read_text",
    "write_bytes",
    "write_text",
}
FORBIDDEN_RESOURCE_IMPORTS = {"importlib.resources", "pkgutil"}

def _base_env(evidence_path: Path):
    env = os.environ.copy()
    env["PYTHONPATH"] = str(ROOT)
    env["PYTHONNOUSERSITE"] = "1"
    env["SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT"] = "1"
    env[EVIDENCE_ENV] = str(evidence_path)
    env.pop(PROBE_ENV, None)
    return env


def _read_events(evidence_path: Path):
    return [json.loads(line) for line in evidence_path.read_text().splitlines()]


def _read_paths(events):
    return [event["path"] for event in events if event["event"] in {"read", "blocked"}]


def _write_paths(events):
    return [event["path"] for event in events if event["event"] == "write"]


def _clean_runtime_path(path: Path) -> None:
    if not os.path.lexists(path):
        return
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
    else:
        path.unlink()


def _allowed_runtime_write_path(
    path: str,
    *,
    mode: str,
    fixture: str,
    mutation: str | None = None,
) -> bool:
    if mode == "public" and _allowed_temp_output_path(path, "integrations/agentrust/out", fixture):
        return True
    if mode == "private" and mutation is None and _allowed_temp_output_path(
        path,
        "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456",
        fixture,
    ):
        return True
    if mode != "private" or mutation is None:
        return False
    mutation_prefix = (
        "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
        ".mutations/"
    )
    if not path.startswith(mutation_prefix):
        return False
    parts = path[len(mutation_prefix) :].split("/")
    if len(parts) < 2 or parts[0] != mutation or parts[1] != fixture:
        return False
    if len(parts) == 4 and parts[2] == "bundle" and parts[3] in {
        "bundle.json",
        "decision.json",
        "policy-bundle.json",
    }:
        return True
    if len(parts) == 3 and parts[2] == "trust-store.json":
        return True
    if len(parts) >= 4 and parts[2] == "out":
        return _allowed_temp_output_path(
            path,
            f"{mutation_prefix}{parts[0]}/{parts[1]}/out",
            fixture,
        )
    return False


def _allowed_runtime_path(
    path: str,
    *,
    mode: str,
    fixture: str,
    mutation: str | None = None,
) -> bool:
    fixture_files = {
        "allow": {
            "bundle.json",
            "decision.json",
            "outcome.json",
            "policy-bundle.json",
            "tool-catalog.json",
            "build-provenance.json",
        },
        "deny": {
            "bundle.json",
            "decision.json",
            "policy-bundle.json",
            "tool-catalog.json",
            "build-provenance.json",
        },
    }
    if mode == "public" or (mode == "private" and mutation is None):
        names = fixture_files[fixture]
        root = f"integrations/agentrust/examples/{fixture}"
        if path == root or path in {f"{root}/{name}" for name in names}:
            return True
        if path == "integrations/agentrust/examples/trust-store.json":
            return True
        if mode == "public" and path == (
            "integrations/agentrust/test-private-keys/"
            "spendguard-agentrust-fixture.test.ed25519.pem"
        ):
            return True
        if mode == "private" and path == (
            "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
            ".trace-key/trace-exporter.test.ed25519.pem"
        ):
            return True
    if mode != "private" or mutation is None:
        return _allowed_temp_output_path(path, "integrations/agentrust/out", fixture) or _allowed_temp_output_path(
            path,
            "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456",
            fixture,
        )
    mutation_prefix = (
        "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
        ".mutations"
    )
    mutation_root = f"{mutation_prefix}/{mutation}/{fixture}"
    if path == f"{mutation_root}/bundle" or path in {
        f"{mutation_root}/bundle/{name}" for name in fixture_files[fixture]
    }:
        return True
    if path == f"{mutation_root}/trust-store.json":
        return True
    if path == (
        "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
        ".trace-key/trace-exporter.test.ed25519.pem"
    ):
        return True
    return _allowed_temp_output_path(path, f"{mutation_root}/out", fixture)


def _allowed_temp_output_path(path: str, root: str, fixture: str) -> bool:
    bundle_id = f"demo-{fixture}"
    prefix = f"{root}/.tmp-{bundle_id}-"
    if not path.startswith(prefix):
        return False
    relative = path[len(root) + 1 :]
    parts = relative.split("/")
    if len(parts) != 2 or parts[1] not in ALL_OUTPUTS:
        return False
    suffix = parts[0].removeprefix(f".tmp-{bundle_id}-")
    return len(suffix) == 32 and all(char in "0123456789abcdef" for char in suffix)


class RuntimeReadBoundaryTest(unittest.TestCase):
    def setUp(self):
        self._reset_runtime_roots()

    def tearDown(self):
        self._reset_runtime_roots()

    def _reset_runtime_roots(self):
        if os.path.lexists(OUT_ROOT) and (OUT_ROOT.is_symlink() or not OUT_ROOT.is_dir()):
            _clean_runtime_path(OUT_ROOT)
        OUT_ROOT.mkdir(parents=True, exist_ok=True)
        _clean_runtime_path(OUT_ROOT / "demo-allow")
        _clean_runtime_path(OUT_ROOT / "demo-deny")
        _clean_runtime_path(RUN_ROOT)
        _clean_runtime_path(
            OUT_ROOT / ".verify-agentrust" / "agentrust-20260706T000010Z-abcdef123458",
        )

    def _assert_no_make_only_reads(self, events):
        for path in _read_paths(events):
            with self.subTest(path=path):
                self.assertNotIn(path, FORBIDDEN_RUNTIME_EXACT)
                self.assertFalse(
                    any(path.startswith(prefix) for prefix in FORBIDDEN_RUNTIME_PREFIXES),
                    path,
                )

    def _assert_allowed_runtime_paths(self, events, *, mode: str, fixture: str, mutation: str | None = None):
        for path in _read_paths(events):
            with self.subTest(path=path):
                self.assertTrue(
                    _allowed_runtime_path(path, mode=mode, fixture=fixture, mutation=mutation),
                    path,
                )

    def _assert_allowed_runtime_writes(
        self,
        events,
        *,
        mode: str,
        fixture: str,
        mutation: str | None = None,
    ):
        for path in _write_paths(events):
            if not _allowed_runtime_write_path(path, mode=mode, fixture=fixture, mutation=mutation):
                raise AssertionError(path)

    def _run_public(self, fixture: str, evidence_path: Path, extra_env=None):
        return self._run_public_args(
            [
                "verify",
                "--bundle",
                f"integrations/agentrust/examples/{fixture}",
                "--trust-store",
                "integrations/agentrust/examples/trust-store.json",
                "--trace-signing-key",
                "integrations/agentrust/test-private-keys/"
                "spendguard-agentrust-fixture.test.ed25519.pem",
                "--out-dir",
                "integrations/agentrust/out",
            ],
            evidence_path,
            extra_env,
        )

    def _run_public_args(self, args, evidence_path: Path, extra_env=None):
        env = _base_env(evidence_path)
        if extra_env:
            env.update(extra_env)
        return subprocess.run(
            [sys.executable, "-m", "spendguard_agentrust", *args],
            cwd=ROOT,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def _prepare_trace_key(self):
        TRACE_KEY.parent.mkdir(parents=True, mode=0o700)
        TRACE_KEY.write_bytes(PUBLIC_KEY.read_bytes())
        TRACE_KEY.chmod(0o600)

    def _copy_fixture_bundle(self, fixture: str, target: Path):
        shutil.copytree(ROOT / "integrations" / "agentrust" / "examples" / fixture, target)

    def _run_private(self, args, evidence_path: Path, extra_env=None):
        env = _base_env(evidence_path)
        env["SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT"] = "1"
        if extra_env:
            env.update(extra_env)
        return subprocess.run(
            [sys.executable, "-m", "spendguard_agentrust._verify_internal", *args],
            cwd=ROOT,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def _private_success_args(self, fixture: str):
        return [
            "verify",
            "--bundle",
            f"integrations/agentrust/examples/{fixture}",
            "--trust-store",
            "integrations/agentrust/examples/trust-store.json",
            "--trace-signing-key",
            "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
            ".trace-key/trace-exporter.test.ed25519.pem",
            "--out-dir",
            "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456",
            "--fixture",
            fixture,
        ]

    def test_public_success_records_only_runtime_inputs(self):
        for fixture in ("allow", "deny"):
            with self.subTest(fixture=fixture):
                with tempfile.TemporaryDirectory() as temp:
                    evidence_path = Path(temp) / "public.jsonl"

                    observed = self._run_public(fixture, evidence_path)

                    self.assertEqual(0, observed.returncode, observed.stderr)
                    events = _read_events(evidence_path)
                    self.assertIn(
                        {"schema": "spendguard-agentrust-runtime-read-evidence/v1", "event": "run", "mode": "public"},
                        events,
                    )
                    self.assertIn(f"integrations/agentrust/examples/{fixture}/bundle.json", _read_paths(events))
                    self.assertIn("integrations/agentrust/examples/trust-store.json", _read_paths(events))
                    self._assert_no_make_only_reads(events)
                    self._assert_allowed_runtime_paths(events, mode="public", fixture=fixture)
                    shutil.rmtree(OUT_ROOT / f"demo-{fixture}", ignore_errors=True)

    def test_public_cli_rejects_normalization_aliases(self):
        cases = (
            "integrations//agentrust/examples/allow",
            "./integrations/agentrust/examples/allow",
        )
        for bundle in cases:
            with self.subTest(bundle=bundle):
                with tempfile.TemporaryDirectory() as temp:
                    observed = self._run_public_args(
                        [
                            "verify",
                            "--bundle",
                            bundle,
                            "--trust-store",
                            "integrations/agentrust/examples/trust-store.json",
                            "--trace-signing-key",
                            "integrations/agentrust/test-private-keys/"
                            "spendguard-agentrust-fixture.test.ed25519.pem",
                            "--out-dir",
                            "integrations/agentrust/out",
                        ],
                        Path(temp) / "public-invalid.jsonl",
                    )

                self.assertEqual(2, observed.returncode)
                payload = json.loads(observed.stderr)
                self.assertEqual("cli_value_invalid", payload["code"])

    def test_public_symlinked_out_dir_rejects_before_writes(self):
        shutil.rmtree(OUT_ROOT, ignore_errors=True)
        with tempfile.TemporaryDirectory() as temp:
            target = Path(temp) / "out-target"
            target.mkdir()
            OUT_ROOT.symlink_to(target, target_is_directory=True)
            try:
                observed = self._run_public("allow", Path(temp) / "public-symlink.jsonl")
            finally:
                OUT_ROOT.unlink(missing_ok=True)
                OUT_ROOT.mkdir(parents=True, exist_ok=True)

            self.assertEqual(2, observed.returncode)
            payload = json.loads(observed.stderr)
            self.assertEqual("output_target_invalid", payload["code"])
            self.assertEqual([], list(target.iterdir()))

    def test_public_missing_out_dir_is_rejected_without_writes(self):
        shutil.rmtree(OUT_ROOT, ignore_errors=True)
        with tempfile.TemporaryDirectory() as temp:
            observed = self._run_public("allow", Path(temp) / "public-create-out.jsonl")

        self.assertEqual(2, observed.returncode)
        payload = json.loads(observed.stderr)
        self.assertEqual("output_target_invalid", payload["code"])
        self.assertFalse(OUT_ROOT.exists())
        self.assertFalse((OUT_ROOT / "demo-allow").exists())

    def test_public_evidence_env_is_ignored_without_internal_test_root(self):
        with tempfile.TemporaryDirectory() as temp:
            evidence_path = Path(temp) / "public-evidence.jsonl"
            env = os.environ.copy()
            env["PYTHONPATH"] = str(ROOT)
            env["PYTHONNOUSERSITE"] = "1"
            env.pop("SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT", None)
            env[EVIDENCE_ENV] = str(evidence_path)

            observed = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "spendguard_agentrust",
                    "verify",
                    "--bundle",
                    "integrations/agentrust/examples/allow",
                    "--trust-store",
                    "integrations/agentrust/examples/trust-store.json",
                    "--trace-signing-key",
                    "integrations/agentrust/test-private-keys/"
                    "spendguard-agentrust-fixture.test.ed25519.pem",
                    "--out-dir",
                    "integrations/agentrust/out",
                ],
                cwd=ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            self.assertEqual(0, observed.returncode, observed.stderr)
            self.assertFalse(evidence_path.exists())

    def test_public_success_records_runtime_output_writes(self):
        with tempfile.TemporaryDirectory() as temp:
            evidence_path = Path(temp) / "public-writes.jsonl"

            observed = self._run_public("allow", evidence_path)

            self.assertEqual(0, observed.returncode, observed.stderr)
            events = _read_events(evidence_path)
            write_paths = _write_paths(events)
            for name in ALL_OUTPUTS:
                self.assertTrue(
                    any(
                        path.startswith("integrations/agentrust/out/.tmp-demo-allow-")
                        and path.endswith(f"/{name}")
                        for path in write_paths
                    ),
                    name,
                )
            self._assert_allowed_runtime_writes(events, mode="public", fixture="allow")

    def test_runtime_write_guard_rejects_unexpected_write_event(self):
        with self.assertRaises(AssertionError):
            self._assert_allowed_runtime_writes(
                [
                    {
                        "schema": "spendguard-agentrust-runtime-read-evidence/v1",
                        "event": "write",
                        "operation": "output_write",
                        "path": "integrations/agentrust/examples/allow/decision.json",
                        "bytes": 2,
                    }
                ],
                mode="public",
                fixture="allow",
            )

    def test_generated_output_validator_rejects_non_trace_shape_errors(self):
        from spendguard_agentrust.verify import _validate_output_bytes, VerificationError

        cases = (
            (
                "agent-manifest.json",
                b'{"@type":"NotAgentManifest","version":"0.1","issuer":"spiffe://example.invalid/a"}',
                "manifest_package_validation_failed",
                "/@type",
            ),
            (
                "agent-manifest.json",
                (
                    b'{"@type":"AgentManifest","version":"0.1","issuer":"spiffe://example.invalid/a",'
                    b'"manifest_id":"019f1af9-b400-75f0-a56d-8a07ccac5e8f",'
                    b'"artifacts":{},"unexpected":true}'
                ),
                "manifest_package_validation_failed",
                "/unexpected",
            ),
            (
                "spendguard-artifacts.json",
                b'{"schema":"wrong","artifacts":[]}',
                "generated_output_schema_invalid",
                "/schema",
            ),
            (
                "spendguard-artifacts.json",
                b'{"schema":"spendguard-agentrust-artifacts/v1","artifacts":[],"unexpected":true}',
                "generated_output_schema_invalid",
                "/unexpected",
            ),
            (
                "verification-report.json",
                b'{"schema":"spendguard-agentrust-verification-report/v1","status":"pass","trace_exporter":{}}',
                "generated_output_schema_invalid",
                "/trace_exporter/signature_status",
            ),
            (
                "verification-report.json",
                (
                    b'{"schema":"spendguard-agentrust-verification-report/v1","status":"pass",'
                    b'"trace_exporter":{"signature_status":"trace_package_signed"},"unexpected":true}'
                ),
                "generated_output_schema_invalid",
                "/unexpected",
            ),
        )
        for name, payload, code, pointer in cases:
            with self.subTest(name=name):
                with self.assertRaises(VerificationError) as caught:
                    _validate_output_bytes(name, payload)
                self.assertEqual(code, caught.exception.diagnostic.code)
                self.assertEqual(5, caught.exception.diagnostic.exit_code)
                self.assertEqual("phase-11", caught.exception.diagnostic.phase)
                self.assertEqual(pointer, caught.exception.diagnostic.json_pointer)

    def test_output_validation_happens_before_temp_dir_creation(self):
        import spendguard_agentrust.verify as verify_module

        values = {
            "bundle": "integrations/agentrust/examples/allow",
            "trust_store": "integrations/agentrust/examples/trust-store.json",
            "trace_signing_key": "integrations/agentrust/test-private-keys/"
            "spendguard-agentrust-fixture.test.ed25519.pem",
            "out_dir": "integrations/agentrust/out",
            "manifest_issuer": "spiffe://spendguard.local/agentrust-exporter",
        }
        original_render = verify_module._render_output
        original_mkdir = verify_module._mkdir_output_temp_dir
        mkdir_called = False

        def bad_render(fixture, name, issuer):
            if name == "verification-report.json":
                return (
                    b'{"schema":"spendguard-agentrust-verification-report/v1",'
                    b'"status":"pass","trace_exporter":{}}'
                )
            return original_render(fixture, name, issuer)

        def recording_mkdir(path, boundary):
            nonlocal mkdir_called
            mkdir_called = True
            return original_mkdir(path, boundary)

        verify_module._render_output = bad_render
        verify_module._mkdir_output_temp_dir = recording_mkdir
        try:
            with self.assertRaises(verify_module.VerificationError) as caught:
                verify_module.verify_public(values)
        finally:
            verify_module._render_output = original_render
            verify_module._mkdir_output_temp_dir = original_mkdir

        self.assertEqual("generated_output_schema_invalid", caught.exception.diagnostic.code)
        self.assertFalse(mkdir_called)
        self.assertFalse((OUT_ROOT / "demo-allow").exists())

    def test_public_world_writable_out_dir_is_rejected(self):
        OUT_ROOT.mkdir(parents=True, exist_ok=True)
        original_mode = OUT_ROOT.stat().st_mode & 0o777
        OUT_ROOT.chmod(0o777)
        try:
            with tempfile.TemporaryDirectory() as temp:
                observed = self._run_public("allow", Path(temp) / "public-mode.jsonl")
        finally:
            OUT_ROOT.chmod(original_mode)

        self.assertEqual(2, observed.returncode)
        payload = json.loads(observed.stderr)
        self.assertEqual("output_target_invalid", payload["code"])

    def test_output_write_open_failure_uses_output_target_diagnostic(self):
        import spendguard_agentrust.verify as verify_module

        values = {
            "bundle": "integrations/agentrust/examples/allow",
            "trust_store": "integrations/agentrust/examples/trust-store.json",
            "trace_signing_key": "integrations/agentrust/test-private-keys/"
            "spendguard-agentrust-fixture.test.ed25519.pem",
            "out_dir": "integrations/agentrust/out",
        }
        boundary = verify_module._public_runtime_read_boundary(values, "allow", "demo-allow")
        temp_name = ".tmp-demo-allow-" + ("0" * 32)
        temp_dir = OUT_ROOT / temp_name
        temp_dir.mkdir(parents=True)
        original_open = verify_module._open_repo_file_no_follow

        def fail_output_open(repo_path, operation, flags, mode=0o600):
            if operation == "output_write":
                raise OSError("forced output open failure")
            return original_open(repo_path, operation, flags, mode)

        verify_module._open_repo_file_no_follow = fail_output_open
        try:
            with self.assertRaises(verify_module.VerificationError) as caught:
                verify_module._runtime_write_bytes(
                    f"integrations/agentrust/out/{temp_name}/trace-level0.json",
                    b"{}",
                    "output_write",
                    boundary,
                )
        finally:
            verify_module._open_repo_file_no_follow = original_open
            shutil.rmtree(temp_dir, ignore_errors=True)

        self.assertEqual("output_target_invalid", caught.exception.diagnostic.code)
        self.assertEqual(2, caught.exception.diagnostic.exit_code)
        self.assertEqual("phase-11", caught.exception.diagnostic.phase)

    def test_public_group_readable_trace_key_is_allowed(self):
        original_mode = PUBLIC_KEY.stat().st_mode & 0o777
        PUBLIC_KEY.chmod(0o644)
        try:
            with tempfile.TemporaryDirectory() as temp:
                observed = self._run_public("allow", Path(temp) / "public-key-readable.jsonl")
        finally:
            PUBLIC_KEY.chmod(original_mode)

        self.assertEqual(0, observed.returncode, observed.stderr)

    def test_private_success_records_only_runtime_inputs(self):
        for fixture in ("allow", "deny"):
            with self.subTest(fixture=fixture):
                self._reset_runtime_roots()
                self._prepare_trace_key()
                with tempfile.TemporaryDirectory() as temp:
                    evidence_path = Path(temp) / "private-success.jsonl"

                    observed = self._run_private(self._private_success_args(fixture), evidence_path)

                    self.assertEqual(0, observed.returncode, observed.stderr)
                    events = _read_events(evidence_path)
                    self.assertIn(
                        {"schema": "spendguard-agentrust-runtime-read-evidence/v1", "event": "run", "mode": "private"},
                        events,
                    )
                    self.assertIn(
                        "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                        ".trace-key/trace-exporter.test.ed25519.pem",
                        _read_paths(events),
                    )
                    self._assert_no_make_only_reads(events)
                    self._assert_allowed_runtime_paths(events, mode="private", fixture=fixture)
                    self._assert_allowed_runtime_writes(events, mode="private", fixture=fixture)

    def test_private_hardlinked_trace_key_is_rejected(self):
        self._prepare_trace_key()
        TRACE_KEY.unlink()
        os.link(PUBLIC_KEY, TRACE_KEY)
        with tempfile.TemporaryDirectory() as temp:
            evidence_path = Path(temp) / "hardlink.jsonl"

            observed = self._run_private(self._private_success_args("allow"), evidence_path)

            self.assertEqual(6, observed.returncode)
            payload = json.loads(observed.stderr)
            self.assertEqual("trace_key_invalid", payload["code"])

    def test_private_group_readable_trace_key_is_trace_key_invalid(self):
        self._prepare_trace_key()
        TRACE_KEY.chmod(0o640)
        with tempfile.TemporaryDirectory() as temp:
            observed = self._run_private(
                self._private_success_args("allow"),
                Path(temp) / "trace-key-mode.jsonl",
            )

        self.assertEqual(6, observed.returncode)
        payload = json.loads(observed.stderr)
        self.assertEqual("trace_key_invalid", payload["code"])

    def test_private_negative_records_boundary_without_make_only_reads(self):
        self._prepare_trace_key()
        mutation_root = RUN_ROOT / ".mutations" / "missing_trace_signature" / "deny"
        mutation_root.mkdir(parents=True)
        self._copy_fixture_bundle("deny", mutation_root / "bundle")
        (mutation_root / "out").mkdir()
        (mutation_root / "trust-store.json").write_bytes(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
        )
        with tempfile.TemporaryDirectory() as temp:
            evidence_path = Path(temp) / "private-negative.jsonl"

            observed = self._run_private(
                [
                    "verify",
                    "--bundle",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".mutations/missing_trace_signature/deny/bundle",
                    "--trust-store",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".mutations/missing_trace_signature/deny/trust-store.json",
                    "--trace-signing-key",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".trace-key/trace-exporter.test.ed25519.pem",
                    "--out-dir",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".mutations/missing_trace_signature/deny/out",
                    "--fixture",
                    "deny",
                    "--mutation",
                    "missing_trace_signature",
                ],
                evidence_path,
            )

            self.assertEqual(5, observed.returncode)
            events = _read_events(evidence_path)
            self.assertIn(
                {"schema": "spendguard-agentrust-runtime-read-evidence/v1", "event": "run", "mode": "private"},
                events,
            )
            self.assertIn(
                "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                ".mutations/missing_trace_signature/deny/bundle/bundle.json",
                _read_paths(events),
            )
            self.assertIn(
                "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                ".mutations/missing_trace_signature/deny/trust-store.json",
                _read_paths(events),
            )
            self.assertIn(
                "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                ".trace-key/trace-exporter.test.ed25519.pem",
                _read_paths(events),
            )
            self._assert_no_make_only_reads(events)
            self._assert_allowed_runtime_paths(
                events,
                mode="private",
                fixture="deny",
                mutation="missing_trace_signature",
            )
            self._assert_allowed_runtime_writes(
                events,
                mode="private",
                fixture="deny",
                mutation="missing_trace_signature",
            )

    def test_private_mutation_records_only_allowed_runtime_writes(self):
        cases = (
            ("mutated_signature", "allow", 3, {"bundle/bundle.json", "bundle/decision.json"}),
            ("mutated_policy_hash", "allow", 4, {"bundle/bundle.json", "bundle/policy-bundle.json"}),
            ("wrong_key", "deny", 3, {"trust-store.json"}),
        )
        for mutation, fixture, expected_exit, expected_writes in cases:
            with self.subTest(mutation=mutation):
                self._reset_runtime_roots()
                self._prepare_trace_key()
                mutation_root = RUN_ROOT / ".mutations" / mutation / fixture
                mutation_root.mkdir(parents=True)
                self._copy_fixture_bundle(fixture, mutation_root / "bundle")
                (mutation_root / "out").mkdir()
                (mutation_root / "trust-store.json").write_bytes(
                    (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
                )
                with tempfile.TemporaryDirectory() as temp:
                    evidence_path = Path(temp) / f"private-{mutation}-write.jsonl"

                    observed = self._run_private(
                        [
                            "verify",
                            "--bundle",
                            "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                            f".mutations/{mutation}/{fixture}/bundle",
                            "--trust-store",
                            "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                            f".mutations/{mutation}/{fixture}/trust-store.json",
                            "--trace-signing-key",
                            "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                            ".trace-key/trace-exporter.test.ed25519.pem",
                            "--out-dir",
                            "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                            f".mutations/{mutation}/{fixture}/out",
                            "--fixture",
                            fixture,
                            "--mutation",
                            mutation,
                        ],
                        evidence_path,
                    )

                    self.assertEqual(expected_exit, observed.returncode, observed.stderr)
                    events = _read_events(evidence_path)
                    write_root = (
                        "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                        f".mutations/{mutation}/{fixture}/"
                    )
                    self.assertEqual(
                        {write_root + path for path in expected_writes},
                        set(_write_paths(events)),
                    )
                    self._assert_no_make_only_reads(events)
                    self._assert_allowed_runtime_paths(
                        events,
                        mode="private",
                        fixture=fixture,
                        mutation=mutation,
                    )
                    self._assert_allowed_runtime_writes(
                        events,
                        mode="private",
                        fixture=fixture,
                        mutation=mutation,
                    )

    def test_deliberate_make_only_probe_is_blocked(self):
        self._prepare_trace_key()
        for label, probe_path in FORBIDDEN_PROBES.items():
            with self.subTest(label=label):
                with tempfile.TemporaryDirectory() as temp:
                    evidence_path = Path(temp) / "blocked.jsonl"

                    observed = self._run_private(
                        self._private_success_args("allow"),
                        evidence_path,
                        {PROBE_ENV: probe_path},
                    )

                    self.assertEqual(6, observed.returncode)
                    self.assertEqual(b"", observed.stdout)
                    payload = json.loads(observed.stderr)
                    self.assertEqual("runtime_read_boundary_violation", payload["code"])
                    events = _read_events(evidence_path)
                    self.assertIn(probe_path, _read_paths(events))

    def test_probe_cannot_read_trace_key_through_generic_runtime_read(self):
        self._prepare_trace_key()
        with tempfile.TemporaryDirectory() as temp:
            evidence_path = Path(temp) / "blocked-trace-key-probe.jsonl"

            observed = self._run_private(
                self._private_success_args("allow"),
                evidence_path,
                {
                    PROBE_ENV: (
                        "integrations/agentrust/out/.verify-agentrust/"
                        "agentrust-20260706T000000Z-abcdef123456/"
                        ".trace-key/trace-exporter.test.ed25519.pem"
                    )
                },
            )

            self.assertEqual(6, observed.returncode)
            self.assertEqual(b"", observed.stdout)
            payload = json.loads(observed.stderr)
            self.assertEqual("runtime_read_boundary_violation", payload["code"])
            events = _read_events(evidence_path)
            self.assertIn(
                "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                ".trace-key/trace-exporter.test.ed25519.pem",
                _read_paths(events),
            )

    def test_relative_traversal_probe_is_blocked_before_normalization(self):
        self._prepare_trace_key()
        probe_path = (
            "integrations/agentrust/examples/allow/../"
            "expected-outputs/allow/trace-level0.json"
        )
        with tempfile.TemporaryDirectory() as temp:
            evidence_path = Path(temp) / "blocked-traversal.jsonl"

            observed = self._run_private(
                self._private_success_args("allow"),
                evidence_path,
                {PROBE_ENV: probe_path},
            )

            self.assertEqual(6, observed.returncode)
            self.assertEqual(b"", observed.stdout)
            payload = json.loads(observed.stderr)
            self.assertEqual("runtime_read_boundary_violation", payload["code"])
            events = _read_events(evidence_path)
            self.assertIn(probe_path, _read_paths(events))

    def test_private_symlinked_runtime_root_is_rejected(self):
        self._prepare_trace_key()
        mutation_root = RUN_ROOT / ".mutations" / "mutated_signature" / "allow"
        mutation_root.mkdir(parents=True)
        (mutation_root / "bundle").symlink_to(
            ROOT / "integrations" / "agentrust" / "examples" / "allow",
            target_is_directory=True,
        )
        (mutation_root / "out").mkdir()
        (mutation_root / "trust-store.json").write_bytes(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
        )
        with tempfile.TemporaryDirectory() as temp:
            evidence_path = Path(temp) / "symlink-root.jsonl"

            observed = self._run_private(
                [
                    "verify",
                    "--bundle",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".mutations/mutated_signature/allow/bundle",
                    "--trust-store",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".mutations/mutated_signature/allow/trust-store.json",
                    "--trace-signing-key",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".trace-key/trace-exporter.test.ed25519.pem",
                    "--out-dir",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".mutations/mutated_signature/allow/out",
                    "--fixture",
                    "allow",
                    "--mutation",
                    "mutated_signature",
                ],
                evidence_path,
            )

            self.assertEqual(6, observed.returncode)
            payload = json.loads(observed.stderr)
            self.assertEqual("runtime_read_boundary_violation", payload["code"])

    def test_private_raw_path_grammar_rejects_invalid_segments(self):
        self._prepare_trace_key()
        cases = {
            "dot_segment": "integrations/agentrust/out/.verify-agentrust/"
            "agentrust-20260706T000000Z-abcdef123456/./.trace-key/trace-exporter.test.ed25519.pem",
            "repeated_separator": "integrations/agentrust/out/.verify-agentrust//"
            "agentrust-20260706T000000Z-abcdef123456/.trace-key/trace-exporter.test.ed25519.pem",
            "backslash": "integrations\\agentrust\\out\\.verify-agentrust\\"
            "agentrust-20260706T000000Z-abcdef123456\\.trace-key\\trace-exporter.test.ed25519.pem",
            "invalid_hidden": "integrations/agentrust/out/.verify-agentrust/"
            "agentrust-20260706T000000Z-abcdef123456/.bad/trace-exporter.test.ed25519.pem",
            "old_run_prefix": "integrations/agentrust/out/.verify-agentrust/"
            "run-abcdef1234567890/.trace-key/trace-exporter.test.ed25519.pem",
            "read_boundary_run": "integrations/agentrust/out/.verify-agentrust/"
            "read-boundary-private/.trace-key/trace-exporter.test.ed25519.pem",
            "uppercase_hex": "integrations/agentrust/out/.verify-agentrust/"
            "agentrust-20260706T000000Z-ABCDEF123456/.trace-key/trace-exporter.test.ed25519.pem",
        }
        for label, trace_path in cases.items():
            with self.subTest(label=label):
                args = self._private_success_args("allow")
                args[args.index("--trace-signing-key") + 1] = trace_path
                with tempfile.TemporaryDirectory() as temp:
                    observed = self._run_private(args, Path(temp) / "grammar.jsonl")

                self.assertEqual(2, observed.returncode)
                payload = json.loads(observed.stderr)
                self.assertEqual("cli_value_invalid", payload["code"])

    def test_private_cross_run_trace_key_is_rejected(self):
        self._prepare_trace_key()
        other_trace = (
            OUT_ROOT
            / ".verify-agentrust"
            / "agentrust-20260706T000010Z-abcdef123458"
            / ".trace-key"
            / "trace-exporter.test.ed25519.pem"
        )
        other_trace.parent.mkdir(parents=True, mode=0o700)
        other_trace.write_bytes(PUBLIC_KEY.read_bytes())
        args = self._private_success_args("allow")
        args[args.index("--trace-signing-key") + 1] = (
            "integrations/agentrust/out/.verify-agentrust/"
            "agentrust-20260706T000010Z-abcdef123458/"
            ".trace-key/trace-exporter.test.ed25519.pem"
        )
        with tempfile.TemporaryDirectory() as temp:
            observed = self._run_private(args, Path(temp) / "cross-run.jsonl")

        self.assertEqual(2, observed.returncode)
        payload = json.loads(observed.stderr)
        self.assertEqual("cli_value_invalid", payload["code"])

    def test_private_success_manifest_issuer_reuses_public_validator(self):
        self._prepare_trace_key()
        with tempfile.TemporaryDirectory() as temp:
            valid_args = [
                *self._private_success_args("allow"),
                "--manifest-issuer",
                "spiffe://agent.example/a/%7E",
            ]
            observed = self._run_private(valid_args, Path(temp) / "valid-issuer.jsonl")
        self.assertEqual(0, observed.returncode, observed.stderr)
        shutil.rmtree(RUN_ROOT / "demo-allow", ignore_errors=True)

        invalid_values = (
            "spiffe://Agent.example/a",
            "spiffe://agent.example/a/%7e",
            "spiffe://agent.example/a/%2F",
            "spiffe://agent.example/a/%20",
            "spiffe://agent.example/a?x=1",
            "spiffe://agent.example/a#x",
            "spiffe://agent.example/a\\b",
            "spiffe://agent.example/",
        )
        for issuer in invalid_values:
            with self.subTest(issuer=issuer):
                self._reset_runtime_roots()
                self._prepare_trace_key()
                args = [
                    *self._private_success_args("allow"),
                    "--manifest-issuer",
                    issuer,
                ]
                with tempfile.TemporaryDirectory() as temp:
                    observed = self._run_private(args, Path(temp) / "invalid-issuer.jsonl")

                self.assertEqual(2, observed.returncode)
                payload = json.loads(observed.stderr)
                self.assertEqual("cli_value_invalid", payload["code"])

    def test_private_mutation_path_grammar_rejects_mismatch(self):
        self._prepare_trace_key()
        mutation_root = RUN_ROOT / ".mutations" / "mutated_signature" / "allow"
        mutation_root.mkdir(parents=True)
        self._copy_fixture_bundle("allow", mutation_root / "bundle")
        (mutation_root / "out").mkdir()
        (mutation_root / "trust-store.json").write_bytes(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
        )
        with tempfile.TemporaryDirectory() as temp:
            observed = self._run_private(
                [
                    "verify",
                    "--bundle",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".mutations/mutated_signature/allow/bundle",
                    "--trust-store",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".mutations/mutated_signature/allow/trust-store.json",
                    "--trace-signing-key",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".trace-key/trace-exporter.test.ed25519.pem",
                    "--out-dir",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".mutations/mutated_signature/allow/out",
                    "--fixture",
                    "deny",
                    "--mutation",
                    "mutated_signature",
                ],
                Path(temp) / "mismatch.jsonl",
            )

        self.assertEqual(2, observed.returncode)
        payload = json.loads(observed.stderr)
        self.assertEqual("cli_value_invalid", payload["code"])

    def test_private_negative_rejects_manifest_issuer(self):
        self._prepare_trace_key()
        mutation_root = RUN_ROOT / ".mutations" / "mutated_signature" / "allow"
        mutation_root.mkdir(parents=True)
        self._copy_fixture_bundle("allow", mutation_root / "bundle")
        (mutation_root / "out").mkdir()
        (mutation_root / "trust-store.json").write_bytes(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
        )
        args = [
            "verify",
            "--bundle",
            "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
            ".mutations/mutated_signature/allow/bundle",
            "--trust-store",
            "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
            ".mutations/mutated_signature/allow/trust-store.json",
            "--trace-signing-key",
            "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
            ".trace-key/trace-exporter.test.ed25519.pem",
            "--out-dir",
            "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
            ".mutations/mutated_signature/allow/out",
            "--fixture",
            "allow",
            "--mutation",
            "mutated_signature",
            "--manifest-issuer",
            "spiffe://example.invalid/custom",
        ]
        with tempfile.TemporaryDirectory() as temp:
            observed = self._run_private(args, Path(temp) / "manifest-issuer.jsonl")

        self.assertEqual(2, observed.returncode)
        payload = json.loads(observed.stderr)
        self.assertEqual("cli_value_invalid", payload["code"])

    def test_private_mutation_write_symlink_escape_is_rejected(self):
        self._prepare_trace_key()
        mutation_root = RUN_ROOT / ".mutations" / "wrong_key" / "deny"
        mutation_root.mkdir(parents=True)
        self._copy_fixture_bundle("deny", mutation_root / "bundle")
        (mutation_root / "out").mkdir()
        original_trust = (
            ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json"
        ).read_bytes()
        with tempfile.TemporaryDirectory() as temp:
            external_trust = Path(temp) / "trust-store.json"
            external_trust.write_bytes(original_trust)
            (mutation_root / "trust-store.json").symlink_to(external_trust)
            evidence_path = Path(temp) / "symlink-write.jsonl"

            observed = self._run_private(
                [
                    "verify",
                    "--bundle",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".mutations/wrong_key/deny/bundle",
                    "--trust-store",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".mutations/wrong_key/deny/trust-store.json",
                    "--trace-signing-key",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".trace-key/trace-exporter.test.ed25519.pem",
                    "--out-dir",
                    "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
                    ".mutations/wrong_key/deny/out",
                    "--fixture",
                    "deny",
                    "--mutation",
                    "wrong_key",
                ],
                evidence_path,
            )

            self.assertEqual(6, observed.returncode)
            payload = json.loads(observed.stderr)
            self.assertEqual("runtime_read_boundary_violation", payload["code"])
            self.assertEqual(original_trust, external_trust.read_bytes())

    def test_private_mutation_write_hardlink_target_is_not_truncated(self):
        import spendguard_agentrust.verify as verify_module

        self._prepare_trace_key()
        mutation_root = RUN_ROOT / ".mutations" / "mutated_signature" / "allow"
        mutation_root.mkdir(parents=True)
        self._copy_fixture_bundle("allow", mutation_root / "bundle")
        (mutation_root / "out").mkdir()
        (mutation_root / "trust-store.json").write_bytes(
            (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
        )
        decision = mutation_root / "bundle" / "decision.json"
        original_decision = decision.read_bytes()
        hardlink_peer = mutation_root / "decision.peer.json"
        hardlink_peer.write_bytes(original_decision)
        decision.unlink()
        os.link(hardlink_peer, decision)
        values = {
            "bundle": "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
            ".mutations/mutated_signature/allow/bundle",
            "trust_store": "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
            ".mutations/mutated_signature/allow/trust-store.json",
            "trace_signing_key": "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
            ".trace-key/trace-exporter.test.ed25519.pem",
            "out_dir": "integrations/agentrust/out/.verify-agentrust/agentrust-20260706T000000Z-abcdef123456/"
            ".mutations/mutated_signature/allow/out",
        }
        boundary = verify_module._private_runtime_read_boundary(
            values,
            "allow",
            "demo-allow",
            "mutated_signature",
        )

        with self.assertRaises(verify_module.VerificationError) as caught:
            verify_module._runtime_write_bytes(
                Path(values["bundle"]) / "decision.json",
                b"{}",
                "mutation_write",
                boundary,
            )

        self.assertEqual("runtime_read_boundary_violation", caught.exception.diagnostic.code)
        self.assertEqual(original_decision, hardlink_peer.read_bytes())
        self.assertEqual(original_decision, decision.read_bytes())

    def test_verifier_runtime_has_no_unwrapped_file_reads(self):
        for path in RUNTIME_GUARD_FILES:
            with self.subTest(path=path.relative_to(ROOT).as_posix()):
                tree = ast.parse(path.read_text(encoding="utf-8"))
                visitor = _RuntimeReadGuard()
                visitor.visit(tree)
                self.assertEqual([], visitor.violations)

    def test_runtime_ast_guard_tracks_os_alias_low_level_io(self):
        source = """
import os as raw_os
from os import read as raw_read
from os import write as raw_write

def leak_read():
    raw_os.read(3, 1)
    raw_read(3, 1)

def leak_write():
    raw_os.write(3, b"x")
    raw_write(3, b"x")
"""
        visitor = _RuntimeReadGuard()
        visitor.visit(ast.parse(source))

        labels = {violation[2] for violation in visitor.violations}
        self.assertIn("raw_os.read", labels)
        self.assertIn("raw_read", labels)
        self.assertIn("raw_os.write", labels)
        self.assertIn("raw_write", labels)

    def test_runtime_ast_guard_tracks_file_read_alias_bypasses(self):
        source = """
import os
from builtins import open as raw_open
from pathlib import Path

def path_method_alias():
    reader = Path("integrations/agentrust/examples/allow/decision.json").read_text
    return reader()

def builtins_open_alias():
    return raw_open("integrations/agentrust/examples/allow/decision.json").read()

def os_fdopen_alias(fd):
    return os.fdopen(fd).read()
"""
        visitor = _RuntimeReadGuard()
        visitor.visit(ast.parse(source))

        labels = {violation[2] for violation in visitor.violations}
        self.assertIn("read_text_alias", labels)
        self.assertIn("raw_open", labels)
        self.assertIn("os.fdopen", labels)


class _RuntimeReadGuard(ast.NodeVisitor):
    def __init__(self):
        self._function_stack = []
        self.violations = []
        self._importlib_aliases = {"importlib"}
        self._os_aliases = {"os"}
        self._resource_aliases = {"resources"}
        self._forbidden_file_aliases = set()
        self._os_read_aliases = set()
        self._os_write_aliases = set()

    def visit_FunctionDef(self, node):
        self._function_stack.append(node.name)
        self.generic_visit(node)
        self._function_stack.pop()

    def visit_AsyncFunctionDef(self, node):
        self.visit_FunctionDef(node)

    def visit_Call(self, node):
        function_name = self._function_stack[-1] if self._function_stack else "<module>"
        access_path = _attribute_path(node.func)
        if access_path:
            if (
                len(access_path) == 2
                and access_path[0] in self._os_aliases
                and access_path[1] == "read"
                and function_name not in ALLOWED_DIRECT_READ_FUNCTIONS
            ):
                self.violations.append((node.lineno, function_name, ".".join(access_path)))
            if (
                len(access_path) == 2
                and access_path[0] in self._os_aliases
                and access_path[1] == "write"
                and function_name not in ALLOWED_DIRECT_WRITE_FUNCTIONS
            ):
                self.violations.append((node.lineno, function_name, ".".join(access_path)))
            if (
                len(access_path) == 2
                and access_path[0] in self._os_aliases
                and access_path[1] == "fdopen"
                and function_name not in ALLOWED_DIRECT_OPEN_FUNCTIONS
            ):
                self.violations.append((node.lineno, function_name, ".".join(access_path)))
            if (
                len(access_path) >= 2
                and access_path[0] in self._importlib_aliases
                and access_path[1] == "resources"
            ):
                self.violations.append((node.lineno, function_name, ".".join(access_path)))
            elif access_path[0] in self._resource_aliases:
                self.violations.append((node.lineno, function_name, ".".join(access_path)))
        if isinstance(node.func, ast.Attribute) and node.func.attr in FORBIDDEN_FILE_ATTRIBUTES:
            if _low_level_os_io(access_path, self._os_aliases):
                pass
            elif node.func.attr.startswith("read") and function_name not in ALLOWED_DIRECT_READ_FUNCTIONS:
                self.violations.append((node.lineno, function_name, node.func.attr))
            elif node.func.attr.startswith("write") and function_name not in ALLOWED_DIRECT_WRITE_FUNCTIONS:
                self.violations.append((node.lineno, function_name, node.func.attr))
            elif node.func.attr == "open" and function_name not in ALLOWED_DIRECT_OPEN_FUNCTIONS:
                self.violations.append((node.lineno, function_name, node.func.attr))
        if isinstance(node.func, ast.Name) and node.func.id == "open":
            if function_name not in ALLOWED_DIRECT_OPEN_FUNCTIONS:
                self.violations.append((node.lineno, function_name, node.func.id))
        if (
            isinstance(node.func, ast.Name)
            and node.func.id in self._os_read_aliases
            and function_name not in ALLOWED_DIRECT_READ_FUNCTIONS
        ):
            self.violations.append((node.lineno, function_name, node.func.id))
        if (
            isinstance(node.func, ast.Name)
            and node.func.id in self._os_write_aliases
            and function_name not in ALLOWED_DIRECT_WRITE_FUNCTIONS
        ):
            self.violations.append((node.lineno, function_name, node.func.id))
        if (
            isinstance(node.func, ast.Name)
            and node.func.id in {"_open_repo_file_no_follow", "_open_repo_dir_no_follow"}
            and function_name not in ALLOWED_REPO_OPEN_CALLERS
        ):
            self.violations.append((node.lineno, function_name, node.func.id))
        if isinstance(node.func, ast.Name) and node.func.id in self._forbidden_file_aliases:
            self.violations.append((node.lineno, function_name, node.func.id))
        if (
            isinstance(node.func, ast.Name)
            and node.func.id == "getattr"
            and len(node.args) >= 2
            and isinstance(node.args[1], ast.Constant)
            and node.args[1].value in FORBIDDEN_FILE_ATTRIBUTES
        ):
            self.violations.append((node.lineno, function_name, f"getattr.{node.args[1].value}"))
        self.generic_visit(node)

    def visit_Assign(self, node):
        if isinstance(node.value, ast.Name) and node.value.id == "open":
            function_name = self._function_stack[-1] if self._function_stack else "<module>"
            self.violations.append((node.lineno, function_name, "open_alias"))
        access_path = _attribute_path(node.value)
        if (
            (access_path and access_path[-1] in FORBIDDEN_FILE_ATTRIBUTES)
            or (
                isinstance(node.value, ast.Attribute)
                and node.value.attr in FORBIDDEN_FILE_ATTRIBUTES
            )
        ):
            function_name = self._function_stack[-1] if self._function_stack else "<module>"
            alias_name = access_path[-1] if access_path else node.value.attr
            for target in node.targets:
                if isinstance(target, ast.Name):
                    self._forbidden_file_aliases.add(target.id)
                    self.violations.append((node.lineno, function_name, f"{alias_name}_alias"))
        self.generic_visit(node)

    def visit_Import(self, node):
        for alias in node.names:
            if alias.name in FORBIDDEN_RESOURCE_IMPORTS:
                function_name = self._function_stack[-1] if self._function_stack else "<module>"
                self.violations.append((node.lineno, function_name, alias.name))
            if alias.name == "importlib":
                self._importlib_aliases.add(alias.asname or alias.name)
            if alias.name == "importlib.resources":
                self._resource_aliases.add(alias.asname or alias.name)
            if alias.name == "os":
                self._os_aliases.add(alias.asname or alias.name)
        self.generic_visit(node)

    def visit_ImportFrom(self, node):
        if node.module in FORBIDDEN_RESOURCE_IMPORTS:
            function_name = self._function_stack[-1] if self._function_stack else "<module>"
            self.violations.append((node.lineno, function_name, node.module))
        if node.module == "importlib":
            for alias in node.names:
                if alias.name == "resources":
                    function_name = self._function_stack[-1] if self._function_stack else "<module>"
                    self.violations.append((node.lineno, function_name, "importlib.resources"))
                    self._resource_aliases.add(alias.asname or alias.name)
        if node.module == "os":
            function_name = self._function_stack[-1] if self._function_stack else "<module>"
            for alias in node.names:
                imported = alias.asname or alias.name
                if alias.name == "read":
                    self._os_read_aliases.add(imported)
                    self.violations.append((node.lineno, function_name, imported))
                elif alias.name == "write":
                    self._os_write_aliases.add(imported)
                    self.violations.append((node.lineno, function_name, imported))
                elif alias.name in {"open", "fdopen"}:
                    self._forbidden_file_aliases.add(imported)
                    self.violations.append((node.lineno, function_name, imported))
        if node.module == "builtins":
            function_name = self._function_stack[-1] if self._function_stack else "<module>"
            for alias in node.names:
                if alias.name == "open":
                    imported = alias.asname or alias.name
                    self._forbidden_file_aliases.add(imported)
                    self.violations.append((node.lineno, function_name, imported))
        self.generic_visit(node)


def _attribute_path(node):
    parts = []
    current = node
    while isinstance(current, ast.Attribute):
        parts.append(current.attr)
        current = current.value
    if isinstance(current, ast.Name):
        parts.append(current.id)
        return list(reversed(parts))
    return None


def _low_level_os_io(access_path, os_aliases=frozenset({"os"})):
    return (
        access_path is not None
        and len(access_path) == 2
        and access_path[0] in os_aliases
        and access_path[1] in {"read", "write"}
    )


if __name__ == "__main__":
    unittest.main()
