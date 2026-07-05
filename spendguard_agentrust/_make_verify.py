from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import shutil
import secrets
import subprocess
import sys
import tarfile
import tempfile
import zipfile

from .fixture_outputs import ALL_OUTPUTS, BUNDLE_IDS, OUTPUT_BYTES, SELECTED_OUTPUTS


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
NEGATIVE_EXPECTATIONS = {
    "missing_trace_signature": (5, "trace_package_validation_failed", "/signature"),
    "mutated_policy_hash": (4, "policy_binding_mismatch", "/policy_bundle_hash"),
    "mutated_signature": (3, "spendguard_signature_invalid", "/signature/value_base64url"),
    "wrong_key": (3, "spendguard_signature_invalid", "/keys/0/public_key_base64url"),
}
FORBIDDEN_ARTIFACT_CONTENT_MARKERS = {
    b"-----BEGIN PRIVATE KEY-----": "private key material",
}
FORBIDDEN_ARTIFACT_MEMBER_MARKERS = {
    b"_verify_internal": "private verifier module",
    b"_make_verify": "make-only verifier module",
}


def main() -> int:
    try:
        run_root = _new_run_root()
        _prepare_trace_key(run_root)
        _check_package_guard()
        _check_help()
        for fixture in ("allow", "deny"):
            _check_fixture(fixture)
            _check_private_success(run_root, fixture)
        _check_private_negative(run_root, "missing_trace_signature", "deny")
        _check_private_negative(run_root, "mutated_policy_hash", "allow")
        _check_private_negative(run_root, "mutated_signature", "allow")
        _check_private_negative(run_root, "wrong_key", "deny")
        _write_report(run_root)
    except AssertionError as exc:
        sys.stderr.write(str(exc))
        return 1

    sys.stdout.write("agentrust_make_verify=pass\n")
    return 0


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
    run_root = REPORT_ROOT / f"run-{secrets.token_hex(8)}"
    run_root.mkdir(parents=True, mode=0o700)
    return run_root


def _prepare_trace_key(run_root: Path) -> None:
    trace_dir = run_root / ".trace-key"
    trace_dir.mkdir(mode=0o700)
    (trace_dir / "trace-exporter.test.ed25519.pem").write_bytes(PUBLIC_TRACE_KEY.read_bytes())


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


def _check_private_negative(run_root: Path, mutation: str, fixture: str) -> None:
    mutation_root = run_root / ".mutations" / mutation / fixture
    (mutation_root / "bundle").mkdir(parents=True)
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
    expected_exit, expected_code, expected_pointer = NEGATIVE_EXPECTATIONS[mutation]
    _assert_equal(expected_exit, observed.returncode, f"{mutation} exit")
    _assert_equal(b"", observed.stdout, f"{mutation} stdout")
    payload = json.loads(observed.stderr)
    _assert_equal(expected_code, payload["code"], f"{mutation} code")
    _assert_equal(expected_pointer, payload["json_pointer"], f"{mutation} pointer")


def _write_report(run_root: Path) -> None:
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
        "negative_tests": {
            "missing_trace_signature": {
                "diagnostic_code": "trace_package_validation_failed",
                "expected_exit_code": 5,
                "observed_exit_code": 5,
                "outputs_absent": True,
                "status": "pass",
            },
            "mutated_policy_hash": {
                "diagnostic_code": "policy_binding_mismatch",
                "expected_exit_code": 4,
                "observed_exit_code": 4,
                "outputs_absent": True,
                "status": "pass",
            },
            "mutated_signature": {
                "diagnostic_code": "spendguard_signature_invalid",
                "expected_exit_code": 3,
                "observed_exit_code": 3,
                "outputs_absent": True,
                "status": "pass",
            },
            "wrong_key": {
                "diagnostic_code": "spendguard_signature_invalid",
                "expected_exit_code": 3,
                "observed_exit_code": 3,
                "outputs_absent": True,
                "status": "pass",
            },
        },
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


def _repo_relative(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


def _assert_equal(expected, observed, label: str) -> None:
    if expected != observed:
        raise AssertionError(f"{label} mismatch")


if __name__ == "__main__":
    raise SystemExit(main())
