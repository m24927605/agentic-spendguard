from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from datetime import datetime, timezone
from pathlib import Path
import shutil
import secrets
import stat
import subprocess
import sys
import tarfile
import tempfile
import zipfile

from .fixture_outputs import ALL_OUTPUTS, BUNDLE_IDS, OUTPUT_BYTES, SELECTED_OUTPUTS
from .diagnostics import Diagnostic, diagnostic_json


ROOT = Path(__file__).resolve().parents[1]
OUT_ROOT = ROOT / "integrations" / "agentrust" / "out"
REPORT_ROOT = OUT_ROOT / ".verify-agentrust"
PUBLIC_TRACE_KEY = (
    ROOT
    / "integrations"
    / "agentrust"
    / "test-private-keys"
    / "spendguard-agentrust-fixture.test.ed25519.pem"
)
EXPECTED_TRACE_KEY_SHA256 = "e293817bb265a337cd359fffc4a9bbb4b14dc41c052840c5857863c516ed9e7f"
NEGATIVE_EXPECTATIONS = {
    "missing_trace_signature": (5, "trace_package_validation_failed", "phase-11", "/signature"),
    "mutated_policy_hash": (4, "policy_binding_mismatch", "phase-10", "/policy_bundle_hash"),
    "mutated_signature": (3, "spendguard_signature_invalid", "phase-9", "/signature/value_base64url"),
    "wrong_key": (3, "spendguard_signature_invalid", "phase-9", "/keys/0/public_key_base64url"),
}
FORBIDDEN_ARTIFACT_CONTENT_MARKERS = {
    b"-----BEGIN PRIVATE KEY-----": "private key material",
}
FORBIDDEN_ARTIFACT_MEMBER_MARKERS = {
    b"_verify_internal": "private verifier module",
    b"_make_verify": "make-only verifier module",
}


PENDING_TARGET_COMMANDS = {
    "verify-full",
    "verify-preflight",
    "verify-output-lane",
    "verify-negative-lane",
    "verify-report-lane",
    "verify-artifact-lane",
}


class MakeDiagnosticError(Exception):
    def __init__(self, diagnostic: Diagnostic):
        super().__init__(diagnostic.code)
        self.diagnostic = diagnostic


class MakeCheckError(AssertionError):
    def __init__(self, label: str, *, mutation: str | None = None, result: dict | None = None):
        super().__init__(label)
        self.mutation = mutation
        self.result = result


def main(argv: list[str] | None = None) -> int:
    args = sys.argv[1:] if argv is None else argv
    if not args or args == ["verify-fixture"]:
        return _run_fixture_smoke()
    if len(args) == 1 and args[0] in PENDING_TARGET_COMMANDS:
        return _emit_pending_full_target()

    diagnostic = Diagnostic("unknown_make_verify_command", 2, "make-setup")
    sys.stderr.write(diagnostic_json(diagnostic) + "\n")
    return diagnostic.exit_code


def _run_fixture_smoke() -> int:
    run_root: Path | None = None
    failure: BaseException | None = None
    conformance_results = _initial_conformance_results()
    success_results = _initial_success_results()
    negative_results: dict[str, dict] = {}
    try:
        run_root = _new_run_root_as_setup()
        _prepare_trace_key_as_setup(run_root)
        _check_package_guard_as_setup()
        conformance_results["package_guard_surfaces"] = _pass_result()
        try:
            _check_help()
        except AssertionError as exc:
            conformance_results["public_parser_matrix"] = _failed_result("public_parser_matrix_failed")
            if failure is None:
                failure = exc
        fixture_identity_passed = True
        internal_guard_passed = True
        for fixture in ("allow", "deny"):
            try:
                _check_fixture(fixture)
            except AssertionError as exc:
                fixture_identity_passed = False
                internal_guard_passed = False
                success_results[fixture] = _failed_success_fixture_result(fixture)
                if failure is None:
                    failure = exc
                continue
            try:
                _check_private_success(run_root, fixture)
            except AssertionError as exc:
                internal_guard_passed = False
                success_results[fixture] = _failed_success_fixture_result(fixture)
                if failure is None:
                    failure = exc
                continue
            success_results[fixture] = _success_fixture_result(fixture)
        conformance_results["fixture_identity"] = (
            _pass_result() if fixture_identity_passed else _failed_result("fixture_verification_failed")
        )
        conformance_results["internal_guard"] = (
            _pass_result() if internal_guard_passed else _failed_result("private_verification_failed")
        )
        for mutation, fixture in (
            ("mutated_signature", "allow"),
            ("wrong_key", "deny"),
            ("mutated_policy_hash", "allow"),
            ("missing_trace_signature", "deny"),
        ):
            try:
                negative_results[mutation] = _check_private_negative(run_root, mutation, fixture)
            except MakeCheckError as exc:
                if exc.mutation is not None and exc.result is not None:
                    negative_results[exc.mutation] = exc.result
                if failure is None:
                    failure = exc
                continue
    except BaseException as exc:
        if isinstance(exc, MakeCheckError) and exc.mutation is not None and exc.result is not None:
            negative_results[exc.mutation] = exc.result
        failure = exc
    finally:
        if run_root is not None:
            try:
                _cleanup_sensitive_workdirs(run_root)
                _check_sensitive_workdirs_absent(run_root)
            except (AssertionError, OSError) as exc:
                failure = MakeDiagnosticError(
                    Diagnostic("packaging_guard_failed", 6, "make-final")
                )
    if failure is not None:
        if isinstance(failure, (KeyboardInterrupt, SystemExit)):
            raise failure
        if isinstance(failure, MakeDiagnosticError):
            sys.stderr.write(diagnostic_json(failure.diagnostic) + "\n")
            return failure.diagnostic.exit_code
        assert run_root is not None
        try:
            _write_failure_report(run_root, negative_results, success_results, conformance_results)
        except OSError:
            return _emit_make_final_diagnostic()
        _write_make_summary(run_root)
        return 1
    try:
        assert run_root is not None
        _check_sensitive_workdirs_absent(run_root)
        _write_report(run_root, negative_results)
    except AssertionError as exc:
        _write_make_summary(run_root)
        return 1
    except OSError:
        return _emit_make_final_diagnostic()

    sys.stdout.write("agentrust_make_verify=pass\n")
    return 0


def _emit_pending_full_target() -> int:
    diagnostic = Diagnostic("deferred_fixture_mvp", 6, "make-setup")
    sys.stderr.write(diagnostic_json(diagnostic) + "\n")
    return diagnostic.exit_code


def _emit_make_final_diagnostic() -> int:
    diagnostic = Diagnostic("packaging_guard_failed", 6, "make-final")
    sys.stderr.write(diagnostic_json(diagnostic) + "\n")
    return diagnostic.exit_code


def _new_run_root_as_setup() -> Path:
    try:
        return _new_run_root()
    except Exception as exc:
        raise MakeDiagnosticError(Diagnostic("output_target_invalid", 2, "make-setup")) from exc


def _check_package_guard_as_setup() -> None:
    try:
        _check_package_guard()
    except Exception as exc:
        raise MakeDiagnosticError(Diagnostic("packaging_guard_failed", 6, "make-setup")) from exc


def _prepare_trace_key_as_setup(run_root: Path) -> None:
    try:
        _prepare_trace_key(run_root)
    except Exception as exc:
        raise MakeDiagnosticError(Diagnostic("trace_key_invalid", 6, "make-setup")) from exc


def _base_env() -> dict[str, str]:
    env = os.environ.copy()
    env["PYTHONPATH"] = str(ROOT)
    env["PYTHONNOUSERSITE"] = "1"
    return env


def _internal_env() -> dict[str, str]:
    env = _base_env()
    env["SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT"] = "1"
    return env


def _run_public(*args: str) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        [sys.executable, "-m", "spendguard_agentrust", *args],
        cwd=ROOT,
        env=_base_env(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def _run_internal(*args: str) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        [sys.executable, "-m", "spendguard_agentrust._verify_internal", *args],
        cwd=ROOT,
        env=_internal_env(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def _new_run_root() -> Path:
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_root = REPORT_ROOT / f"agentrust-{timestamp}-{secrets.token_hex(6)}"
    run_root.mkdir(parents=True, mode=0o700)
    return run_root


def _prepare_trace_key(run_root: Path) -> None:
    trace_dir = run_root / ".trace-key"
    trace_dir.mkdir(mode=0o700)
    trace_dir.chmod(0o700)
    key_path = trace_dir / "trace-exporter.test.ed25519.pem"
    data = _read_public_trace_key_source()
    if hashlib.sha256(data).hexdigest() != EXPECTED_TRACE_KEY_SHA256:
        raise AssertionError("trace key source digest mismatch")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if not hasattr(os, "O_NOFOLLOW"):
        raise AssertionError("trace key copy requires O_NOFOLLOW")
    flags |= os.O_NOFOLLOW
    try:
        fd = os.open(os.fspath(key_path), flags, 0o600)
    except OSError as exc:
        raise AssertionError("trace key copy create failed") from exc
    try:
        os.fchmod(fd, 0o600)
        offset = 0
        while offset < len(data):
            written = os.write(fd, data[offset:])
            if written <= 0:
                raise AssertionError("trace key copy write failed")
            offset += written
    finally:
        os.close(fd)
    _assert_trace_key_copy_private(key_path)


def _read_public_trace_key_source() -> bytes:
    try:
        status = PUBLIC_TRACE_KEY.lstat()
    except OSError as exc:
        raise AssertionError("trace key source missing") from exc
    if (
        stat.S_ISLNK(status.st_mode)
        or not stat.S_ISREG(status.st_mode)
        or status.st_nlink != 1
        or status.st_uid != os.getuid()
        or status.st_dev != ROOT.stat().st_dev
        or bool(status.st_mode & 0o7000)
        or bool(status.st_mode & 0o022)
    ):
        raise AssertionError("trace key source metadata invalid")
    if not hasattr(os, "O_NOFOLLOW"):
        raise AssertionError("trace key source read requires O_NOFOLLOW")
    fd = os.open(os.fspath(PUBLIC_TRACE_KEY), os.O_RDONLY | os.O_NOFOLLOW)
    try:
        opened = os.fstat(fd)
        if (
            opened.st_ino != status.st_ino
            or opened.st_dev != status.st_dev
            or opened.st_uid != status.st_uid
            or not stat.S_ISREG(opened.st_mode)
        ):
            raise AssertionError("trace key source changed during read")
        chunks = []
        while True:
            chunk = os.read(fd, 65536)
            if not chunk:
                break
            chunks.append(chunk)
        return b"".join(chunks)
    finally:
        os.close(fd)


def _assert_trace_key_copy_private(key_path: Path) -> None:
    try:
        status = key_path.lstat()
    except OSError as exc:
        raise AssertionError("trace key copy missing") from exc
    if (
        stat.S_ISLNK(status.st_mode)
        or not stat.S_ISREG(status.st_mode)
        or status.st_nlink != 1
        or status.st_uid != os.getuid()
        or (status.st_mode & 0o777) != 0o600
    ):
        raise AssertionError("trace key copy metadata invalid")


def _cleanup_sensitive_workdirs(run_root: Path) -> None:
    _remove_sensitive_path(run_root / ".trace-key")
    _remove_sensitive_path(run_root / ".mutations")


def _remove_sensitive_path(path: Path) -> None:
    if not os.path.lexists(path):
        return
    status = path.lstat()
    if status.st_uid != os.getuid() or status.st_dev != OUT_ROOT.stat().st_dev:
        raise AssertionError(f"sensitive cleanup target unsafe: {path.name}")
    if path.is_symlink() or path.is_file():
        path.unlink()
        return
    if path.is_dir():
        shutil.rmtree(path)
        return
    path.unlink()


def _check_sensitive_workdirs_absent(run_root: Path) -> None:
    for name in (".trace-key", ".mutations"):
        if os.path.lexists(run_root / name):
            raise AssertionError(f"sensitive workdir remained: {name}")


def _check_package_guard() -> None:
    backend = _load_build_backend()
    with tempfile.TemporaryDirectory() as temp:
        out = Path(temp)
        wheel_name = backend.build_wheel(str(out))
        sdist_name = backend.build_sdist(str(out))
        for name in (wheel_name, sdist_name):
            artifact = out / name
            _scan_package_artifact_bytes(name, artifact.read_bytes())
            _scan_package_archive_members(artifact)


def _load_build_backend():
    path = ROOT / "packaging" / "agentrust_build_backend.py"
    spec = importlib.util.spec_from_file_location("agentrust_build_backend_guard", path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def _scan_package_artifact_bytes(label: str, data: bytes) -> None:
    for marker, description in FORBIDDEN_ARTIFACT_CONTENT_MARKERS.items():
        if marker in data:
            raise AssertionError(f"package guard failed: {label} contains {description}")


def _scan_package_archive_members(path: Path) -> None:
    if path.suffix == ".whl":
        with zipfile.ZipFile(path) as archive:
            for name in archive.namelist():
                _scan_package_member_name(path.name, name)
                _scan_package_artifact_bytes(f"{path.name}:{name}", archive.read(name))
        return

    if path.name.endswith(".tar.gz"):
        with tarfile.open(path, "r:gz") as archive:
            for member in archive.getmembers():
                _scan_package_member_name(path.name, member.name)
                if member.isfile():
                    extracted = archive.extractfile(member)
                    if extracted is not None:
                        _scan_package_artifact_bytes(f"{path.name}:{member.name}", extracted.read())


def _scan_package_member_name(label: str, name: str) -> None:
    encoded = name.encode("utf-8")
    for marker, description in FORBIDDEN_ARTIFACT_MEMBER_MARKERS.items():
        if marker in encoded:
            raise AssertionError(f"package guard failed: {label} member {name} exposes {description}")


def _check_help() -> None:
    observed = _run_public("verify", "--help")
    _assert_equal(0, observed.returncode, "public help exit")
    _assert_equal(b"", observed.stderr, "public help stderr")


def _check_fixture(fixture: str) -> None:
    bundle_id = BUNDLE_IDS[fixture]
    output_dir = OUT_ROOT / bundle_id
    shutil.rmtree(output_dir, ignore_errors=True)
    observed = _run_public(
        "verify",
        "--bundle",
        f"integrations/agentrust/examples/{fixture}",
        "--trust-store",
        "integrations/agentrust/examples/trust-store.json",
        "--trace-signing-key",
        "integrations/agentrust/test-private-keys/spendguard-agentrust-fixture.test.ed25519.pem",
        "--out-dir",
        "integrations/agentrust/out",
    )
    _assert_equal(0, observed.returncode, f"{fixture} verifier exit")
    _assert_equal(b"", observed.stderr, f"{fixture} verifier stderr")
    expected_stdout = json.dumps(
        {
            "bundle_id": bundle_id,
            "outputs": SELECTED_OUTPUTS,
            "schema": "spendguard-agentrust-verify-result/v1",
            "status": "pass",
        },
        separators=(",", ":"),
    ).encode("utf-8")
    _assert_equal(expected_stdout, observed.stdout, f"{fixture} verifier stdout")
    for name in ALL_OUTPUTS:
        _assert_equal(
            OUTPUT_BYTES[(fixture, name)],
            (output_dir / name).read_bytes(),
            f"{fixture} {name}",
        )
    shutil.rmtree(output_dir, ignore_errors=True)


def _check_private_success(run_root: Path, fixture: str) -> None:
    _assert_trace_key_copy_private(run_root / ".trace-key" / "trace-exporter.test.ed25519.pem")
    observed = _run_internal(
        "verify",
        "--bundle",
        f"integrations/agentrust/examples/{fixture}",
        "--trust-store",
        "integrations/agentrust/examples/trust-store.json",
        "--trace-signing-key",
        _repo_relative(run_root / ".trace-key" / "trace-exporter.test.ed25519.pem"),
        "--out-dir",
        _repo_relative(run_root),
        "--fixture",
        fixture,
    )
    _assert_equal(0, observed.returncode, f"{fixture} private verifier exit")
    _assert_equal(b"", observed.stdout, f"{fixture} private verifier stdout")
    _assert_equal(b"", observed.stderr, f"{fixture} private verifier stderr")


def _check_private_negative(run_root: Path, mutation: str, fixture: str) -> dict:
    mutation_root = run_root / ".mutations" / mutation / fixture
    mutation_root.mkdir(parents=True)
    shutil.copytree(
        ROOT / "integrations" / "agentrust" / "examples" / fixture,
        mutation_root / "bundle",
        symlinks=True,
    )
    (mutation_root / "out").mkdir()
    (mutation_root / "trust-store.json").write_bytes(
        (ROOT / "integrations" / "agentrust" / "examples" / "trust-store.json").read_bytes()
    )
    observed = _run_internal(
        "verify",
        "--bundle",
        _repo_relative(mutation_root / "bundle"),
        "--trust-store",
        _repo_relative(mutation_root / "trust-store.json"),
        "--trace-signing-key",
        _repo_relative(run_root / ".trace-key" / "trace-exporter.test.ed25519.pem"),
        "--out-dir",
        _repo_relative(mutation_root / "out"),
        "--fixture",
        fixture,
        "--mutation",
        mutation,
    )
    expected_exit, expected_code, expected_phase, expected_pointer = NEGATIVE_EXPECTATIONS[mutation]
    outputs_absent = _negative_outputs_absent(mutation_root, fixture)
    payload: dict | None = None
    if observed.stderr:
        try:
            payload = json.loads(observed.stderr)
        except json.JSONDecodeError:
            payload = None
    diagnostic_code = payload.get("code") if isinstance(payload, dict) else None
    result = {
        "diagnostic_code": diagnostic_code,
        "expected_exit_code": expected_exit,
        "observed_exit_code": observed.returncode,
        "outputs_absent": outputs_absent,
        "status": "pass",
    }
    passed = (
        observed.returncode == expected_exit
        and observed.stdout == b""
        and isinstance(payload, dict)
        and payload.get("code") == expected_code
        and payload.get("phase") == expected_phase
        and payload.get("json_pointer") == expected_pointer
        and outputs_absent is True
    )
    if not passed:
        result["status"] = "fail"
        raise MakeCheckError(f"{mutation} negative check failed", mutation=mutation, result=result)
    return result


def _negative_outputs_absent(mutation_root: Path, fixture: str) -> bool:
    out_dir = mutation_root / "out"
    if os.path.lexists(out_dir) and out_dir.is_symlink():
        return False
    if os.path.lexists(out_dir / BUNDLE_IDS[fixture]):
        return False
    if not os.path.lexists(out_dir):
        return True
    if not out_dir.is_dir():
        return False
    for candidate in out_dir.rglob("*"):
        if os.path.lexists(candidate):
            return False
    return True


def _write_report(run_root: Path, negative_results: dict[str, dict]) -> None:
    ordered_negative_results = {
        mutation: negative_results[mutation]
        for mutation in NEGATIVE_EXPECTATIONS
        if mutation in negative_results
    }
    deferred = {
        "diagnostic_code": "deferred_fixture_mvp",
        "status": "deferred_fixture_mvp",
    }
    report = {
        "conformance_checks": {
            "artifact_preflight_envelope": deferred,
            "event_type_binding": deferred,
            "fixture_identity": {"diagnostic_code": None, "status": "pass"},
            "internal_guard": {"diagnostic_code": None, "status": "pass"},
            "manifest_issuer_variant": deferred,
            "package_guard_surfaces": {"diagnostic_code": None, "status": "pass"},
            "private_parser_matrix": deferred,
            "public_parser_matrix": deferred,
        },
        "mode": "fixture",
        "negative_tests": ordered_negative_results,
        "package_versions": {
            "agent-manifest": "0.1.1",
            "agentrust-trace": "0.2.0",
            "agentrust-trace-tests": "0.1.0",
        },
        "schema": "spendguard-agentrust-make-verification-report/v1",
        "success_fixtures": {
            "allow": {
                "bundle_id": "demo-allow",
                "diagnostic_code": None,
                "outputs_match": True,
                "status": "pass",
            },
            "deny": {
                "bundle_id": "demo-deny",
                "diagnostic_code": None,
                "outputs_match": True,
                "status": "pass",
            },
        },
        "status": "pass",
    }
    (run_root / "verify-agentrust-report.json").write_text(
        json.dumps(report, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )


def _write_failure_report(
    run_root: Path,
    negative_results: dict[str, dict],
    success_results: dict[str, dict] | None = None,
    conformance_results: dict[str, dict] | None = None,
) -> None:
    success_results = success_results or _initial_success_results()
    conformance_results = conformance_results or _initial_conformance_results()
    negative_report = {
        mutation: negative_results.get(
            mutation,
            {
                "diagnostic_code": None,
                "expected_exit_code": NEGATIVE_EXPECTATIONS[mutation][0],
                "observed_exit_code": None,
                "outputs_absent": True,
                "status": "not_run",
            },
        )
        for mutation in NEGATIVE_EXPECTATIONS
    }
    report = {
        "conformance_checks": conformance_results,
        "mode": "fixture",
        "negative_tests": negative_report,
        "package_versions": {
            "agent-manifest": "0.1.1",
            "agentrust-trace": "0.2.0",
            "agentrust-trace-tests": "0.1.0",
        },
        "schema": "spendguard-agentrust-make-verification-report/v1",
        "success_fixtures": success_results,
        "status": "fail",
    }
    (run_root / "verify-agentrust-report.json").write_text(
        json.dumps(report, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )


def _write_make_summary(run_root: Path) -> None:
    summary = {
        "report_path": _repo_relative(run_root / "verify-agentrust-report.json"),
        "schema": "spendguard-agentrust-make-summary/v1",
        "status": "fail",
    }
    sys.stderr.write(json.dumps(summary, separators=(",", ":")) + "\n")


def _pass_result() -> dict:
    return {"diagnostic_code": None, "status": "pass"}


def _failed_result(diagnostic_code: str) -> dict:
    return {"diagnostic_code": diagnostic_code, "status": "fail"}


def _initial_conformance_results() -> dict[str, dict]:
    return {
        "artifact_preflight_envelope": {"diagnostic_code": None, "status": "not_run"},
        "event_type_binding": {"diagnostic_code": None, "status": "not_run"},
        "fixture_identity": {"diagnostic_code": None, "status": "not_run"},
        "internal_guard": {"diagnostic_code": None, "status": "not_run"},
        "manifest_issuer_variant": {"diagnostic_code": None, "status": "not_run"},
        "package_guard_surfaces": {"diagnostic_code": None, "status": "not_run"},
        "private_parser_matrix": {"diagnostic_code": None, "status": "not_run"},
        "public_parser_matrix": {"diagnostic_code": None, "status": "not_run"},
    }


def _success_fixture_result(fixture: str) -> dict:
    return {
        "bundle_id": BUNDLE_IDS[fixture],
        "diagnostic_code": None,
        "outputs_match": True,
        "status": "pass",
    }


def _failed_success_fixture_result(fixture: str) -> dict:
    return {
        "bundle_id": BUNDLE_IDS[fixture],
        "diagnostic_code": "fixture_verification_failed",
        "outputs_match": False,
        "status": "fail",
    }


def _initial_success_results() -> dict[str, dict]:
    return {
        "allow": {
            "bundle_id": "demo-allow",
            "diagnostic_code": None,
            "outputs_match": False,
            "status": "not_run",
        },
        "deny": {
            "bundle_id": "demo-deny",
            "diagnostic_code": None,
            "outputs_match": False,
            "status": "not_run",
        },
    }


def _repo_relative(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


def _assert_equal(expected, observed, label: str) -> None:
    if expected != observed:
        raise AssertionError(f"{label} mismatch")


if __name__ == "__main__":
    raise SystemExit(main())
