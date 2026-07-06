from __future__ import annotations

import base64
import binascii
import contextlib
import ctypes
from dataclasses import dataclass
from datetime import datetime, timezone
import errno as errno_module
import hashlib
import json
import os
import platform
import re
import secrets
import shutil
from pathlib import Path
import stat
import subprocess
import sys
import tempfile

from .diagnostics import Diagnostic
from .fixture_outputs import (
    ALL_OUTPUTS,
    BUNDLE_IDS,
    FIXTURE_BY_BUNDLE_PATH,
    OUTPUT_BYTES,
    SELECTED_OUTPUTS,
)


def _resolve_repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


DEFAULT_ISSUER = "spiffe://spendguard.local/agentrust-exporter"
EXPECTED_EVIDENCE_ROOTS = {
    "allow": "sha256:242768282ddb2f490b73ccc2ae10be70d27ef9f9bbf5a3da698c717e9c7d3b85",
    "deny": "sha256:13ccd9d4af951fbcc46ef3838a5f98acffdcc455f86079bcc31519fe8bc5bfa2",
}
EXPECTED_TRUST_PUBLIC_KEY = "QwRr_kCSs-lJlOraFdzCDYqqB7ZY_TlU644O-4vcpd4"
ALLOWED_BUNDLE_ENTRY_PATHS = {
    "decision.json",
    "outcome.json",
    "policy-bundle.json",
    "tool-catalog.json",
    "build-provenance.json",
}
EXPECTED_BUNDLE_FILE_SETS = {
    "allow": [
        "decision.json",
        "outcome.json",
        "policy-bundle.json",
        "tool-catalog.json",
        "build-provenance.json",
    ],
    "deny": [
        "decision.json",
        "policy-bundle.json",
        "tool-catalog.json",
        "build-provenance.json",
    ],
}
EXPECTED_BUNDLE_MEDIA_TYPES = {
    "build-provenance.json": "application/vnd.spendguard.build-provenance+json",
    "decision.json": "application/vnd.spendguard.audit.decision+json",
    "outcome.json": "application/vnd.spendguard.audit.outcome+json",
    "policy-bundle.json": "application/vnd.spendguard.policy-bundle+json",
    "tool-catalog.json": "application/vnd.spendguard.tool-catalog+json",
}
ALLOWED_DIR_MODES = {0o700, 0o750, 0o755}
ED25519_SPKI_PREFIX = bytes.fromhex("302a300506032b6570032100")
PRIVATE_KEY_PEM_SENTINEL = ("-----BEGIN " + "PRIVATE KEY-----").encode("ascii")
PRIVATE_KEY_BEGIN = ("-----BEGIN " + "PRIVATE KEY-----").encode("ascii")
PRIVATE_KEY_END = ("-----END " + "PRIVATE KEY-----").encode("ascii")
MAX_PRODUCER_SEQUENCE = 9223372036854775807
PROTO_VECTOR_EXPECTED_SHA256 = "ec56bfefad76b988400d7d1b21db3076d7b21c201b9933c58066fa652d3092c8"
PROTO_VECTOR_EXPECTED_HEX_PREFIX = (
    "0a03312e3012197370656e6467756172642e61756469742e6465636973696f6e"
)
PROTO_VECTOR_DATA_B64 = (
    "eyJkYXRhX2NsYXNzIjoiaW50ZXJuYWwiLCJtYXRjaGVkX3J1bGVzIjpbXSwibW9kZWwiOnsibW9kZWxfaWQiOiJn"
    "cHQtNG8tbWluaSIsInByb3ZpZGVyIjoic3BlbmRndWFyZCIsInZlcnNpb24iOiJ1bmtub3duIn0sInJlYXNvbl9j"
    "b2RlcyI6W10sInNuYXBzaG90X2hhc2giOiJzaGEyNTY6YWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFh"
    "YWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYSIsInNwZW5kZ3VhcmQiOnsiYnVpbGQiOnsiY29udGFpbmVy"
    "X2ltYWdlX2RpZ2VzdCI6InNoYTI1Njo0NDQ0NDQ0NDQ0NDQ0NDQ0NDQ0NDQ0NDQ0NDQ0NDQ0NDQ0NDQ0NDQ0NDQ0"
    "NDQ0NDQ0NDQ0NDQ0NDQ0NDQ0NDQ0IiwicmVsZWFzZV9idW5kbGVfZGlnZXN0IjpudWxsLCJzYm9tX2RpZ2VzdCI6"
    "bnVsbCwic291cmNlX2NvbW1pdF9kaWdlc3QiOm51bGx9LCJwb2xpY3lfYnVuZGxlX2hhc2giOiJzaGEyNTY6MjIy"
    "MjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMiIsInRv"
    "b2xfY2F0YWxvZ19oYXNoIjoic2hhMjU2OjMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMz"
    "MzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMifX0="
)
PROTO_VECTOR_CLOUDEVENT = {
    "data_b64": PROTO_VECTOR_DATA_B64,
    "data_json": json.loads(base64.b64decode(PROTO_VECTOR_DATA_B64).decode("utf-8")),
    "datacontenttype": "application/json",
    "decision_id": "019f1af9-b400-7000-8000-000000000102",
    "id": "019f1af9-b400-7000-8000-000000000101",
    "producer_id": "sidecar:agentrust-fixture",
    "producer_sequence": 7,
    "run_id": "run-demo",
    "schema_bundle_id": "spendguard-agentrust-v1",
    "source": "spiffe://spendguard.local/exporter/test",
    "specversion": "1.0",
    "tenant_id": "00000000-0000-4000-8000-000000000001",
    "time": "2026-07-01T00:00:00.000Z",
    "type": "spendguard.audit.decision",
}
ROOT = _resolve_repo_root()
ROOT_DEVICE = ROOT.stat().st_dev
RUNTIME_READ_EVIDENCE_ENV = "SPENDGUARD_AGENTRUST_RUNTIME_READ_EVIDENCE"
RUNTIME_READ_EVIDENCE_SCHEMA = "spendguard-agentrust-runtime-read-evidence/v1"
PUBLIC_TRUST_STORE_PATH = "integrations/agentrust/examples/trust-store.json"
PUBLIC_TRACE_KEY_PATH = (
    "integrations/agentrust/test-private-keys/"
    "spendguard-agentrust-fixture.test.ed25519.pem"
)
PUBLIC_OUTPUT_ROOT = "integrations/agentrust/out"
PRIVATE_RUNTIME_ROOT = "integrations/agentrust/out/.verify-agentrust/"
TRACE_KEY_COPY_SUFFIX = "/.trace-key/trace-exporter.test.ed25519.pem"
ALLOWED_RAW_HIDDEN_SEGMENTS = {".verify-agentrust", ".trace-key", ".mutations"}
RUN_ID_RE = re.compile(r"^agentrust-[0-9]{8}T[0-9]{6}Z-[0-9a-f]{12}$")
BASE64URL_RE = re.compile(r"^[A-Za-z0-9_-]+$")
SHA256_DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
UUID4_RE = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
AMOUNT_ATOMIC_RE = re.compile(r"^[1-9][0-9]{0,38}$")
RFC3339_MILLIS_RE = re.compile(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{3}Z$")
LINUX_RENAMEAT2_SYSCALLS = {"x86_64": 316, "aarch64": 276, "arm64": 276}
RENAME_NOREPLACE = 1
RENAME_EXCL = 0x00000004
AT_FDCWD = -100
OUTPUT_RUNTIME_OPERATIONS = {"output_readback", "output_temp_dir", "output_write"}


@dataclass(frozen=True)
class RuntimeReadBoundary:
    mode: str
    fixture: str
    bundle_id: str
    bundle_root: str
    trust_store: str
    trace_signing_key: str
    out_dir: str


class VerificationError(Exception):
    def __init__(self, diagnostic: Diagnostic):
        super().__init__(diagnostic.code)
        self.diagnostic = diagnostic


class DuplicateJsonKeyError(ValueError):
    pass


def _repo_abs(repo_path: Path | str) -> Path:
    return ROOT / os.fspath(repo_path)


def _unsafe_dir_metadata(status: os.stat_result) -> bool:
    return (
        not stat.S_ISDIR(status.st_mode)
        or status.st_uid != os.getuid()
        or status.st_dev != ROOT_DEVICE
        or bool(status.st_mode & 0o7000)
        or (status.st_mode & 0o777) not in ALLOWED_DIR_MODES
        or bool(status.st_mode & 0o022)
    )


def _unsafe_file_metadata(status: os.stat_result) -> bool:
    return (
        not stat.S_ISREG(status.st_mode)
        or status.st_nlink != 1
        or status.st_uid != os.getuid()
        or status.st_dev != ROOT_DEVICE
        or bool(status.st_mode & 0o7000)
        or bool(status.st_mode & 0o022)
    )


def _unsafe_public_trace_key_metadata(status: os.stat_result) -> bool:
    return (
        not stat.S_ISREG(status.st_mode)
        or status.st_nlink != 1
        or status.st_uid != os.getuid()
        or status.st_dev != ROOT_DEVICE
        or bool(status.st_mode & 0o7000)
        or bool(status.st_mode & 0o022)
    )


def verify_public(values: dict[str, str]) -> bytes:
    bundle_path = _runtime_arg_path(values["bundle"])
    fixture = FIXTURE_BY_BUNDLE_PATH.get(bundle_path)
    if fixture is None:
        _runtime_boundary_violation()
    bundle_id = BUNDLE_IDS[fixture]
    boundary = _public_runtime_read_boundary(values, fixture, bundle_id)
    _record_runtime_read_run("public")
    output_preflight = _preflight_output_target(boundary)
    _validate_inputs(values, fixture, bundle_id, boundary)
    _commit_outputs(values, fixture, bundle_id, boundary, output_preflight)

    return json.dumps(
        {
            "bundle_id": bundle_id,
            "outputs": SELECTED_OUTPUTS,
            "schema": "spendguard-agentrust-verify-result/v1",
            "status": "pass",
        },
        separators=(",", ":"),
    ).encode("utf-8")


def _validate_inputs(
    values: dict[str, str],
    fixture: str,
    bundle_id: str,
    boundary: RuntimeReadBoundary,
    *,
    strict_fixture_root: bool = True,
) -> None:
    bundle_root = Path(values["bundle"])
    try:
        bundle_bytes = _runtime_read_bytes(bundle_root / "bundle.json", "json_input", boundary)
    except OSError as exc:
        raise VerificationError(Diagnostic("bundle_index_invalid", 2, "phase-2")) from exc
    bundle = _read_bundle_index(bundle_bytes, bundle_id)
    files = bundle["files"]
    _validate_bundle_file_set(bundle_root, files, fixture)
    bundle = _validate_bundle_schema(bundle_bytes, bundle_id)
    files = bundle["files"]
    _validate_bundle_media_types(files)
    _check_cloudevent_canonicalization_probe()

    for index, entry in enumerate(files):
        rel = entry["path"]
        expected = entry["sha256"]
        path = _bundle_entry_path(bundle_root, rel, index)
        try:
            actual = hashlib.sha256(
                _runtime_read_bytes(path, "bundle_entry_hash", boundary)
            ).hexdigest()
        except OSError as exc:
            raise VerificationError(
                Diagnostic("file_hash_mismatch", 4, "phase-6", f"/files/{index}/path")
            ) from exc
        if actual != expected:
            _raise("file_hash_mismatch", 4, "phase-6", f"/files/{index}/sha256")

    preimage = dict(bundle)
    expected_root = preimage.pop("evidence_bundle_hash", "")
    actual_root = hashlib.sha256(
        json.dumps(preimage, sort_keys=True, separators=(",", ":")).encode("utf-8")
    ).hexdigest()
    if expected_root != f"sha256:{actual_root}":
        _raise("evidence_bundle_hash_mismatch", 4, "phase-6", "/evidence_bundle_hash")
    if strict_fixture_root and expected_root != EXPECTED_EVIDENCE_ROOTS[fixture]:
        _raise("evidence_bundle_hash_mismatch", 4, "phase-6", "/evidence_bundle_hash")

    trust_store = _read_json(
        Path(values["trust_store"]),
        "canonical_json_invalid",
        2,
        "phase-7",
        boundary,
    )
    _validate_trust_store_shape(trust_store)

    decision = _read_json(
        bundle_root / "decision.json",
        "canonical_json_invalid",
        2,
        "phase-7",
        boundary,
    )
    _verify_signed_event(decision, trust_store)
    policy_bundle = _read_json(
        bundle_root / "policy-bundle.json",
        "canonical_json_invalid",
        2,
        "phase-7",
        boundary,
    )
    tool_catalog = _read_json(
        bundle_root / "tool-catalog.json",
        "canonical_json_invalid",
        2,
        "phase-7",
        boundary,
    )
    build_provenance = _read_json(
        bundle_root / "build-provenance.json",
        "canonical_json_invalid",
        2,
        "phase-7",
        boundary,
    )
    _verify_policy_binding(decision, policy_bundle)
    _verify_tool_binding(decision, tool_catalog)
    _verify_build_binding(decision, build_provenance)
    if fixture == "allow":
        outcome = _read_json(
            bundle_root / "outcome.json",
            "canonical_json_invalid",
            2,
            "phase-7",
            boundary,
        )
        _verify_signed_event(outcome, trust_store)
    else:
        outcome = None
    _verify_lifecycle(bundle, decision, outcome, fixture)

    try:
        key_bytes = _runtime_read_trace_key_bytes(
            Path(values["trace_signing_key"]),
            boundary,
        )
    except OSError as exc:
        raise VerificationError(Diagnostic("trace_key_invalid", 6, "phase-4")) from exc
    if PRIVATE_KEY_PEM_SENTINEL not in key_bytes:
        _raise("trace_key_invalid", 6, "phase-4", None)
    _verify_trace_signing_key(key_bytes, fixture)


def _commit_outputs(
    values: dict[str, str],
    fixture: str,
    bundle_id: str,
    boundary: RuntimeReadBoundary,
    preflight: tuple[Path, Path, Path] | None = None,
) -> None:
    output_root_rel, _, output_dir_abs = (
        preflight if preflight is not None else _preflight_output_target(boundary)
    )

    outputs = {}
    for name in ALL_OUTPUTS:
        data = _render_output(fixture, name, values["manifest_issuer"])
        _validate_output_bytes(name, data)
        outputs[name] = data

    temp_dir_rel = output_root_rel / f".tmp-{bundle_id}-{secrets.token_hex(16)}"
    temp_dir_abs = _repo_abs(temp_dir_rel)
    try:
        _mkdir_output_temp_dir(temp_dir_rel, boundary)
        for name, data in outputs.items():
            target = temp_dir_rel / name
            _runtime_write_bytes(target, data, "output_write", boundary)
            if _runtime_read_bytes(target, "output_readback", boundary) != data:
                _raise("output_target_invalid", 2, "phase-11", None)
        _rename_dir_no_replace(temp_dir_abs, output_dir_abs)
    except VerificationError:
        shutil.rmtree(temp_dir_abs, ignore_errors=True)
        raise
    except OSError as exc:
        shutil.rmtree(temp_dir_abs, ignore_errors=True)
        raise VerificationError(Diagnostic("output_target_invalid", 2, "phase-11")) from exc


def _preflight_output_target(boundary: RuntimeReadBoundary) -> tuple[Path, Path, Path]:
    output_root_rel = Path(boundary.out_dir)
    output_root_abs = _repo_abs(boundary.out_dir)
    output_diagnostic = Diagnostic("output_target_invalid", 2, "phase-2")
    _reject_argument_symlinks(boundary.out_dir, output_diagnostic)
    _validate_runtime_path_components(boundary.out_dir, output_diagnostic, require_leaf=False)
    _reject_argument_symlinks(boundary.out_dir, output_diagnostic)
    _validate_runtime_path_components(boundary.out_dir, output_diagnostic)

    output_dir_abs = output_root_abs / boundary.bundle_id
    if output_root_abs.is_symlink() or not output_root_abs.is_dir():
        _raise("output_target_invalid", 2, "phase-2", None)
    if output_dir_abs.exists() or output_dir_abs.is_symlink():
        _raise("output_target_invalid", 2, "phase-2", None)
    _reject_stale_output_temp_entries(boundary)
    return output_root_rel, output_root_abs, output_dir_abs


def _reject_stale_output_temp_entries(boundary: RuntimeReadBoundary) -> None:
    root_fd = _open_repo_dir_no_follow(boundary.out_dir, "output_temp_dir")
    prefix = f".tmp-{boundary.bundle_id}-"
    try:
        try:
            names = os.listdir(root_fd)
        except OSError as exc:
            raise VerificationError(Diagnostic("output_target_invalid", 2, "phase-2")) from exc
        for name in names:
            suffix = name.removeprefix(prefix)
            if name == suffix or len(suffix) != 32 or any(char not in "0123456789abcdef" for char in suffix):
                continue
            try:
                status = os.stat(name, dir_fd=root_fd, follow_symlinks=False)
            except OSError as exc:
                raise VerificationError(Diagnostic("output_target_invalid", 2, "phase-2")) from exc
            if not stat.S_ISDIR(status.st_mode) or _unsafe_dir_metadata(status):
                raise VerificationError(Diagnostic("output_target_invalid", 2, "phase-2"))
            temp_abs = _repo_abs(Path(boundary.out_dir) / name)
            for child in temp_abs.rglob("*"):
                try:
                    child_status = child.lstat()
                except OSError as exc:
                    raise VerificationError(Diagnostic("output_target_invalid", 2, "phase-2")) from exc
                if (
                    child.parent != temp_abs
                    or child.name not in ALL_OUTPUTS
                    or stat.S_ISLNK(child_status.st_mode)
                    or _unsafe_file_metadata(child_status)
                ):
                    raise VerificationError(Diagnostic("output_target_invalid", 2, "phase-2"))
            try:
                shutil.rmtree(temp_abs)
            except OSError as exc:
                raise VerificationError(Diagnostic("output_target_invalid", 2, "phase-2")) from exc
    finally:
        os.close(root_fd)


def _render_output(fixture: str, name: str, issuer: str) -> bytes:
    data = OUTPUT_BYTES[(fixture, name)]
    if name == "agent-manifest.json" and issuer != DEFAULT_ISSUER:
        payload = json.loads(data)
        payload["issuer"] = issuer
        return json.dumps(payload, separators=(",", ":")).encode("utf-8")
    return data


def _validate_trace_package_bytes(data: bytes) -> None:
    try:
        trace = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise VerificationError(
            Diagnostic("trace_package_validation_failed", 5, "phase-11")
        ) from exc
    signature = trace.get("signature") if isinstance(trace, dict) else None
    if not isinstance(signature, str) or not signature:
        _raise("trace_package_validation_failed", 5, "phase-11", "/signature")


def _validate_output_bytes(name: str, data: bytes) -> None:
    if name == "trace-level0.json":
        _validate_trace_package_bytes(data)
        return

    try:
        payload = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        code = (
            "manifest_package_validation_failed"
            if name == "agent-manifest.json"
            else "generated_output_schema_invalid"
        )
        raise VerificationError(Diagnostic(code, 5, "phase-11")) from exc
    if not isinstance(payload, dict):
        code = (
            "manifest_package_validation_failed"
            if name == "agent-manifest.json"
            else "generated_output_schema_invalid"
        )
        _raise(code, 5, "phase-11", None)

    if name == "agent-manifest.json":
        if payload.get("@type") != "AgentManifest":
            _raise("manifest_package_validation_failed", 5, "phase-11", "/@type")
        if set(payload) != {
            "@context",
            "@type",
            "agent_id",
            "artifacts",
            "crypto_profile",
            "expires_at",
            "issued_at",
            "issuer",
            "manifest_id",
            "version",
        }:
            _raise("manifest_package_validation_failed", 5, "phase-11", _first_extra_pointer(payload, {
                "@context",
                "@type",
                "agent_id",
                "artifacts",
                "crypto_profile",
                "expires_at",
                "issued_at",
                "issuer",
                "manifest_id",
                "version",
            }))
        if payload.get("version") != "0.1":
            _raise("manifest_package_validation_failed", 5, "phase-11", "/version")
        issuer = payload.get("issuer")
        if not isinstance(issuer, str) or not issuer.startswith("spiffe://"):
            _raise("manifest_package_validation_failed", 5, "phase-11", "/issuer")
        if not isinstance(payload.get("manifest_id"), str) or not payload["manifest_id"]:
            _raise("manifest_layout_invalid", 5, "phase-11", "/manifest_id")
        if not isinstance(payload.get("artifacts"), dict):
            _raise("manifest_layout_invalid", 5, "phase-11", "/artifacts")
        return

    if name == "spendguard-artifacts.json":
        if set(payload) != {"schema", "artifacts"}:
            _raise(
                "generated_output_schema_invalid",
                5,
                "phase-11",
                _first_extra_pointer(payload, {"schema", "artifacts"}),
            )
        if payload.get("schema") != "spendguard-agentrust-artifacts/v1":
            _raise("generated_output_schema_invalid", 5, "phase-11", "/schema")
        artifacts = payload.get("artifacts")
        if not isinstance(artifacts, list) or not artifacts:
            _raise("generated_output_schema_invalid", 5, "phase-11", "/artifacts")
        for index, artifact in enumerate(artifacts):
            if not isinstance(artifact, dict):
                _raise("generated_output_schema_invalid", 5, "phase-11", f"/artifacts/{index}")
            for key in ("id", "media_type", "source_file", "source_file_digest", "binding_digest"):
                if not isinstance(artifact.get(key), str) or not artifact[key]:
                    _raise("generated_output_schema_invalid", 5, "phase-11", f"/artifacts/{index}/{key}")
        return

    if name == "verification-report.json":
        expected_report_keys = {
            "agent_manifest",
            "bundle_id",
            "digests",
            "filesystem",
            "inputs",
            "mode",
            "negative_tests",
            "negative_tests_scope",
            "outputs",
            "package_versions",
            "replay_cache",
            "schema",
            "spendguard_verification",
            "status",
            "trace_exporter",
        }
        if payload.get("schema") != "spendguard-agentrust-verification-report/v1":
            _raise("generated_output_schema_invalid", 5, "phase-11", "/schema")
        if payload.get("status") != "pass":
            _raise("generated_output_schema_invalid", 5, "phase-11", "/status")
        trace_exporter = payload.get("trace_exporter")
        if not isinstance(trace_exporter, dict):
            _raise("generated_output_schema_invalid", 5, "phase-11", "/trace_exporter")
        if trace_exporter.get("signature_status") != "trace_package_signed":
            _raise(
                "generated_output_schema_invalid",
                5,
                "phase-11",
                "/trace_exporter/signature_status",
            )
        if set(payload) != expected_report_keys:
            _raise(
                "generated_output_schema_invalid",
                5,
                "phase-11",
                _first_extra_pointer(payload, expected_report_keys),
            )
        return

    _raise("generated_output_schema_invalid", 5, "phase-11", None)


def _rename_dir_no_replace(source: Path, target: Path) -> None:
    try:
        _platform_rename_no_replace(source, target)
    except FileExistsError:
        _raise("output_target_invalid", 2, "phase-11", None)
    except OSError as exc:
        raise VerificationError(Diagnostic("output_target_invalid", 2, "phase-11")) from exc


def _platform_rename_no_replace(source: Path, target: Path) -> None:
    if sys.platform == "darwin":
        libc = ctypes.CDLL(None, use_errno=True)
        result = libc.renamex_np(
            os.fsencode(source),
            os.fsencode(target),
            ctypes.c_uint(RENAME_EXCL),
        )
        if result != 0:
            error_number = ctypes.get_errno()
            if error_number == errno_module.EEXIST:
                raise FileExistsError(error_number, os.strerror(error_number), str(target))
            raise OSError(error_number, os.strerror(error_number), str(target))
        return

    if sys.platform.startswith("linux"):
        syscall_number = LINUX_RENAMEAT2_SYSCALLS.get(platform.machine())
        if syscall_number is not None:
            libc = ctypes.CDLL(None, use_errno=True)
            result = libc.syscall(
                ctypes.c_long(syscall_number),
                ctypes.c_int(AT_FDCWD),
                os.fsencode(source),
                ctypes.c_int(AT_FDCWD),
                os.fsencode(target),
                ctypes.c_uint(RENAME_NOREPLACE),
            )
            if result != 0:
                error_number = ctypes.get_errno()
                if error_number == errno_module.EEXIST:
                    raise FileExistsError(error_number, os.strerror(error_number), str(target))
                raise OSError(error_number, os.strerror(error_number), str(target))
            return

    raise OSError(
        errno_module.ENOTSUP,
        "atomic no-replace directory rename is unavailable on this platform",
        str(target),
    )


def _bundle_entry_path(bundle_root: Path, rel: str, index: int) -> Path:
    if rel not in ALLOWED_BUNDLE_ENTRY_PATHS:
        _raise("path_safety_error", 2, "phase-2", f"/files/{index}/path")
    target = bundle_root / rel
    try:
        status = _repo_abs(target).lstat()
    except OSError:
        return target
    if stat.S_ISLNK(status.st_mode) or _unsafe_file_metadata(status):
        _raise("path_safety_error", 2, "phase-2", f"/files/{index}/path")
    return target


def _read_bundle_index(bundle_bytes: bytes, bundle_id: str) -> dict:
    diagnostic = Diagnostic("bundle_index_invalid", 2, "phase-2")
    bundle = _loads_json(bundle_bytes, diagnostic)
    if not isinstance(bundle, dict):
        raise VerificationError(diagnostic)
    if bundle.get("bundle_id") != bundle_id:
        _raise("bundle_index_invalid", 2, "phase-2", "/bundle_id")
    files = bundle.get("files")
    if not isinstance(files, list):
        _raise("bundle_index_invalid", 2, "phase-2", "/files")
    for index, entry in enumerate(files):
        if not isinstance(entry, dict):
            _raise("bundle_index_invalid", 2, "phase-2", f"/files/{index}")
        rel = entry.get("path")
        media_type = entry.get("media_type")
        expected = entry.get("sha256")
        if not isinstance(rel, str):
            _raise("bundle_index_invalid", 2, "phase-2", f"/files/{index}/path")
        if not isinstance(media_type, str):
            _raise("bundle_index_invalid", 2, "phase-2", f"/files/{index}/media_type")
        if not isinstance(expected, str):
            _raise("bundle_index_invalid", 2, "phase-2", f"/files/{index}/sha256")
        if rel not in ALLOWED_BUNDLE_ENTRY_PATHS:
            _raise("path_safety_error", 2, "phase-2", f"/files/{index}/path")
    return bundle


def _validate_bundle_schema(bundle_bytes: bytes, bundle_id: str) -> dict:
    bundle = _loads_canonical_json(
        bundle_bytes,
        Diagnostic("canonical_json_invalid", 2, "phase-5", "/bundle.json"),
    )
    expected_keys = {
        "bundle_id",
        "created_at",
        "decision_id",
        "decision_path",
        "evidence_bundle_hash",
        "files",
        "run_id",
        "tenant_id",
        "version",
    }
    _require_keys(bundle, expected_keys, "schema_invalid", 2, "phase-5", "/bundle.json")
    if bundle.get("version") != "spendguard-agentrust-evidence/v1":
        _raise("schema_invalid", 2, "phase-5", "/version")
    if bundle.get("bundle_id") != bundle_id:
        _raise("schema_invalid", 2, "phase-5", "/bundle_id")
    for field in ("created_at", "decision_id", "run_id", "tenant_id"):
        if not isinstance(bundle.get(field), str) or not bundle[field]:
            _raise("schema_invalid", 2, "phase-5", f"/{field}")
    if bundle.get("decision_path") not in {"allow", "deny"}:
        _raise("schema_invalid", 2, "phase-5", "/decision_path")
    if not isinstance(bundle.get("evidence_bundle_hash"), str) or SHA256_DIGEST_RE.fullmatch(
        bundle["evidence_bundle_hash"]
    ) is None:
        _raise("schema_invalid", 2, "phase-5", "/evidence_bundle_hash")
    files = bundle.get("files")
    if not isinstance(files, list) or not files:
        _raise("schema_invalid", 2, "phase-5", "/files")
    seen_paths = set()
    for index, entry in enumerate(files):
        expected_file_keys = {"media_type", "path", "sha256"}
        _require_keys(entry, expected_file_keys, "schema_invalid", 2, "phase-5", f"/files/{index}")
        rel = entry["path"]
        media_type = entry["media_type"]
        digest = entry["sha256"]
        if rel in seen_paths:
            _raise("schema_invalid", 2, "phase-5", f"/files/{index}/path")
        seen_paths.add(rel)
        if rel not in ALLOWED_BUNDLE_ENTRY_PATHS:
            _raise("path_safety_error", 2, "phase-5", f"/files/{index}/path")
        if not isinstance(media_type, str) or not media_type:
            _raise("schema_invalid", 2, "phase-5", f"/files/{index}/media_type")
        if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
            _raise("schema_invalid", 2, "phase-5", f"/files/{index}/sha256")
    return bundle


def _validate_bundle_media_types(files: list) -> None:
    for index, entry in enumerate(files):
        expected = EXPECTED_BUNDLE_MEDIA_TYPES.get(entry["path"])
        if entry["media_type"] != expected:
            _raise("media_type_invalid", 2, "phase-7", f"/files/{index}/media_type")


def _validate_bundle_file_set(bundle_root: Path, files: list, fixture: str) -> None:
    expected_paths = EXPECTED_BUNDLE_FILE_SETS[fixture]
    listed_paths = [entry.get("path") if isinstance(entry, dict) else None for entry in files]
    if listed_paths != expected_paths:
        _raise("file_set_invalid", 2, "phase-3", "/files")

    repo_path = _runtime_read_repo_path(bundle_root, "bundle_file_set")
    root_fd = _open_repo_dir_no_follow(repo_path, "bundle_file_set")
    try:
        try:
            physical = sorted(os.listdir(root_fd))
        except OSError as exc:
            raise VerificationError(Diagnostic("output_target_invalid", 2, "phase-2")) from exc
        for name in physical:
            try:
                status = os.stat(name, dir_fd=root_fd, follow_symlinks=False)
            except OSError as exc:
                raise VerificationError(Diagnostic("path_safety_error", 2, "phase-3")) from exc
            if stat.S_ISLNK(status.st_mode):
                _raise("path_safety_error", 2, "phase-3", f"/{name}")
        expected_physical = sorted(["bundle.json", *expected_paths])
        if physical != expected_physical:
            _raise("file_set_invalid", 2, "phase-3", "/files")
    finally:
        os.close(root_fd)
    _write_runtime_read_evidence(
        {
            "schema": RUNTIME_READ_EVIDENCE_SCHEMA,
            "event": "read",
            "operation": "bundle_file_set",
            "path": repo_path,
            "bytes": 0,
        }
    )


def _first_extra_pointer(payload: dict, expected: set[str]) -> str | None:
    extras = sorted(set(payload) - expected)
    if extras:
        return f"/{extras[0]}"
    missing = sorted(expected - set(payload))
    if missing:
        return f"/{missing[0]}"
    return None


def _require_keys(payload: object, expected: set[str], code: str, exit_code: int, phase: str, pointer: str) -> dict:
    if not isinstance(payload, dict) or set(payload) != expected:
        actual = payload if isinstance(payload, dict) else {}
        _raise(code, exit_code, phase, _first_extra_pointer(actual, expected) or pointer)
    return payload


def _require_sha256_or_null(value: object, code: str, exit_code: int, phase: str, pointer: str) -> None:
    if value is not None and (not isinstance(value, str) or SHA256_DIGEST_RE.fullmatch(value) is None):
        _raise(code, exit_code, phase, pointer)


def _validate_trust_store_shape(trust_store: dict) -> None:
    _require_keys(trust_store, {"version", "keys"}, "trust_store_invalid", 2, "phase-7", "")
    if trust_store.get("version") != "spendguard-agentrust-trust-store/v1":
        _raise("trust_store_invalid", 2, "phase-7", "/version")
    keys = trust_store.get("keys") if isinstance(trust_store, dict) else None
    if not isinstance(keys, list) or not keys:
        _raise("trust_store_invalid", 2, "phase-7", "/keys")
    seen_key_ids = set()
    for index, key in enumerate(keys):
        if not isinstance(key, dict):
            _raise("trust_store_invalid", 2, "phase-7", f"/keys/{index}")
        expected_key_fields = {
            "algorithm",
            "allowed_producer_ids",
            "allowed_subject_prefixes",
            "allowed_tenant_ids",
            "environment",
            "key_id",
            "not_after",
            "not_before",
            "public_key_base64url",
            "revoked_at",
        }
        if set(key) != expected_key_fields:
            _raise(
                "trust_store_invalid",
                2,
                "phase-7",
                f"/keys/{index}{_first_extra_pointer(key, expected_key_fields) or ''}",
            )
        key_id = key.get("key_id")
        if not isinstance(key_id, str) or key_id in seen_key_ids:
            _raise("trust_store_invalid", 2, "phase-7", f"/keys/{index}/key_id")
        seen_key_ids.add(key_id)
        if key.get("algorithm") != "Ed25519":
            _raise("trust_store_invalid", 2, "phase-7", f"/keys/{index}/algorithm")
        if key.get("environment") != "fixture":
            _raise("trust_store_invalid", 2, "phase-7", f"/keys/{index}/environment")
        if key.get("revoked_at") is not None:
            _raise("trust_store_invalid", 2, "phase-7", f"/keys/{index}/revoked_at")
        for field in ("allowed_producer_ids", "allowed_subject_prefixes", "allowed_tenant_ids"):
            values = key.get(field)
            if (
                not isinstance(values, list)
                or not values
                or not all(isinstance(value, str) and value for value in values)
            ):
                _raise("trust_store_invalid", 2, "phase-7", f"/keys/{index}/{field}")
            if field == "allowed_subject_prefixes" and not all(
                value.startswith("spiffe://") and value.endswith("/") for value in values
            ):
                _raise("trust_store_invalid", 2, "phase-7", f"/keys/{index}/{field}")
        for field in ("not_before", "not_after", "public_key_base64url"):
            if not isinstance(key.get(field), str) or not key[field]:
                _raise("trust_store_invalid", 2, "phase-7", f"/keys/{index}/{field}")
        _parse_rfc3339_seconds(key["not_before"], f"/keys/{index}/not_before")
        _parse_rfc3339_seconds(key["not_after"], f"/keys/{index}/not_after")
        public_key = _decode_base64url(
            key["public_key_base64url"],
            Diagnostic("trust_store_invalid", 2, "phase-7", f"/keys/{index}/public_key_base64url"),
        )
        if len(public_key) != 32:
            _raise("trust_store_invalid", 2, "phase-7", f"/keys/{index}/public_key_base64url")


def _select_trust_key(
    trust_store: dict,
    signature: dict,
    cloudevent: dict,
) -> tuple[int, dict]:
    key_id = signature.get("key_id")
    if not isinstance(key_id, str):
        _raise("trust_key_not_found", 3, "phase-9", "/signature/key_id")

    keys = trust_store.get("keys") if isinstance(trust_store, dict) else None
    if not isinstance(keys, list):
        _raise("trust_store_invalid", 2, "phase-7", "/keys")

    matches = [
        (index, key)
        for index, key in enumerate(keys)
        if isinstance(key, dict) and key.get("key_id") == key_id
    ]
    if not matches:
        _raise("trust_key_not_found", 3, "phase-9", "/signature/key_id")
    if len(matches) > 1:
        _raise("trust_store_invalid", 2, "phase-7", "/keys")

    key_index, key = matches[0]
    if key.get("revoked_at") is not None:
        _raise("trust_authorization_failed", 3, "phase-9", f"/keys/{key_index}/revoked_at")
    producer_id = _canonical_string(cloudevent, "producer_id")
    tenant_id = _canonical_string(cloudevent, "tenant_id")
    if producer_id not in key.get("allowed_producer_ids", []):
        _raise("trust_authorization_failed", 3, "phase-9", f"/keys/{key_index}/allowed_producer_ids")
    if tenant_id not in key.get("allowed_tenant_ids", []):
        _raise("trust_authorization_failed", 3, "phase-9", f"/keys/{key_index}/allowed_tenant_ids")
    subject_prefixes = key.get("allowed_subject_prefixes", [])
    run_id = _canonical_string(cloudevent, "run_id")
    derived_subject = f"spiffe://spendguard.local/tenant/{tenant_id}/run/{run_id}"
    if "subject" in cloudevent:
        _raise("trust_authorization_failed", 3, "phase-9", "/cloudevent/subject")
    if not any(derived_subject.startswith(prefix) for prefix in subject_prefixes):
        _raise("trust_authorization_failed", 3, "phase-9", f"/keys/{key_index}/allowed_subject_prefixes")

    event_time = _parse_rfc3339_millis(_canonical_string(cloudevent, "time"))[0]
    not_before = _parse_rfc3339_seconds(key["not_before"], f"/keys/{key_index}/not_before")
    not_after = _parse_rfc3339_seconds(key["not_after"], f"/keys/{key_index}/not_after")
    if event_time < not_before or event_time >= not_after:
        _raise("trust_key_time_mismatch", 4, "phase-9", f"/keys/{key_index}")
    return key_index, key


def _validate_signed_event_shape(event: dict) -> None:
    _require_keys(
        event,
        {"schema", "cloudevent", "canonical_bytes_sha256", "signature"},
        "schema_invalid",
        2,
        "phase-7",
        "",
    )
    cloudevent_keys = {
        "data_b64",
        "data_json",
        "datacontenttype",
        "decision_id",
        "id",
        "producer_id",
        "producer_sequence",
        "run_id",
        "schema_bundle_id",
        "source",
        "specversion",
        "tenant_id",
        "time",
        "type",
    }
    cloudevent = event.get("cloudevent")
    if not isinstance(cloudevent, dict) or set(cloudevent) != cloudevent_keys:
        actual = cloudevent if isinstance(cloudevent, dict) else {}
        _raise("schema_invalid", 2, "phase-7", _first_extra_pointer(actual, cloudevent_keys) or "/cloudevent")
    signature = _require_keys(
        event.get("signature"),
        {"algorithm", "key_id", "value_base64url"},
        "schema_invalid",
        2,
        "phase-7",
        "/signature",
    )
    if signature.get("algorithm") != "Ed25519":
        _raise("trust_key_not_found", 3, "phase-9", "/signature/key_id")
    for field in ("key_id", "value_base64url"):
        if not isinstance(signature.get(field), str) or not signature[field]:
            _raise("schema_invalid", 2, "phase-7", f"/signature/{field}")
    required_strings = (
        "data_b64",
        "datacontenttype",
        "decision_id",
        "id",
        "producer_id",
        "run_id",
        "schema_bundle_id",
        "source",
        "specversion",
        "tenant_id",
        "time",
        "type",
    )
    for field in required_strings:
        if not isinstance(cloudevent.get(field), str) or not cloudevent[field]:
            _raise("schema_invalid", 2, "phase-7", f"/cloudevent/{field}")
    if cloudevent.get("datacontenttype") != "application/json":
        _raise("schema_invalid", 2, "phase-7", "/cloudevent/datacontenttype")
    if cloudevent.get("specversion") != "1.0":
        _raise("schema_invalid", 2, "phase-7", "/cloudevent/specversion")
    producer_sequence = cloudevent.get("producer_sequence")
    if (
        not isinstance(producer_sequence, int)
        or isinstance(producer_sequence, bool)
        or producer_sequence < 0
        or producer_sequence > MAX_PRODUCER_SEQUENCE
    ):
        _raise("schema_invalid", 2, "phase-7", "/cloudevent/producer_sequence")
    if not isinstance(cloudevent.get("data_json"), dict):
        _raise("schema_invalid", 2, "phase-7", "/cloudevent/data_json")
    event_type = cloudevent.get("type")
    schema = event.get("schema")
    if not isinstance(schema, str) or not schema:
        _raise("schema_invalid", 2, "phase-7", "/schema")
    if event_type == "spendguard.audit.decision":
        if schema != "spendguard-agentrust-decision-event/v1":
            _raise("schema_invalid", 2, "phase-7", "/schema")
        _validate_decision_payload(cloudevent.get("data_json"))
    elif event_type == "spendguard.audit.outcome":
        if schema != "spendguard-agentrust-outcome-event/v1":
            _raise("schema_invalid", 2, "phase-7", "/schema")
        _validate_outcome_payload(cloudevent.get("data_json"))
    else:
        _raise("schema_invalid", 2, "phase-7", "/cloudevent/type")
    if not isinstance(cloudevent.get("run_id"), str) or "/" in cloudevent["run_id"]:
        _raise("schema_invalid", 2, "phase-7", "/cloudevent/run_id")
    if not isinstance(event.get("canonical_bytes_sha256"), str) or not re.fullmatch(
        r"[0-9a-f]{64}", event["canonical_bytes_sha256"]
    ):
        _raise("schema_invalid", 2, "phase-7", "/canonical_bytes_sha256")


def _validate_decision_payload(payload: object) -> None:
    payload = _require_keys(
        payload,
        {"data_class", "matched_rules", "model", "reason_codes", "snapshot_hash", "spendguard"},
        "schema_invalid",
        2,
        "phase-7",
        "/cloudevent/data_json",
    )
    if not isinstance(payload.get("reason_codes"), list) or not all(
        isinstance(value, str) and value for value in payload["reason_codes"]
    ):
        _raise("schema_invalid", 2, "phase-7", "/cloudevent/data_json/reason_codes")
    if not isinstance(payload.get("matched_rules"), list) or not all(
        isinstance(value, str) and value for value in payload["matched_rules"]
    ):
        _raise("schema_invalid", 2, "phase-7", "/cloudevent/data_json/matched_rules")
    if not isinstance(payload.get("snapshot_hash"), str) or SHA256_DIGEST_RE.fullmatch(payload["snapshot_hash"]) is None:
        _raise("schema_invalid", 2, "phase-7", "/cloudevent/data_json/snapshot_hash")
    if not isinstance(payload.get("data_class"), str) or not payload["data_class"]:
        _raise("schema_invalid", 2, "phase-7", "/cloudevent/data_json/data_class")
    model = _require_keys(
        payload.get("model"),
        {"model_id", "provider", "version"},
        "schema_invalid",
        2,
        "phase-7",
        "/cloudevent/data_json/model",
    )
    for field in ("model_id", "provider", "version"):
        if not isinstance(model.get(field), str) or not model[field]:
            _raise("schema_invalid", 2, "phase-7", f"/cloudevent/data_json/model/{field}")
    spendguard = _require_keys(
        payload.get("spendguard"),
        {"build", "policy_bundle_hash", "tool_catalog_hash"},
        "schema_invalid",
        2,
        "phase-7",
        "/cloudevent/data_json/spendguard",
    )
    for field in ("policy_bundle_hash", "tool_catalog_hash"):
        if not isinstance(spendguard.get(field), str) or SHA256_DIGEST_RE.fullmatch(spendguard[field]) is None:
            _raise("schema_invalid", 2, "phase-7", f"/cloudevent/data_json/spendguard/{field}")
    build = _require_keys(
        spendguard.get("build"),
        {"container_image_digest", "release_bundle_digest", "sbom_digest", "source_commit_digest"},
        "schema_invalid",
        2,
        "phase-7",
        "/cloudevent/data_json/spendguard/build",
    )
    for field in ("container_image_digest", "release_bundle_digest", "sbom_digest", "source_commit_digest"):
        _require_sha256_or_null(
            build.get(field),
            "schema_invalid",
            2,
            "phase-7",
            f"/cloudevent/data_json/spendguard/build/{field}",
        )


def _validate_outcome_payload(payload: object) -> None:
    payload = _require_keys(
        payload,
        {"decision_id", "estimated_amount_atomic", "kind", "reservation_id"},
        "schema_invalid",
        2,
        "phase-7",
        "/cloudevent/data_json",
    )
    for field in ("decision_id", "kind", "reservation_id", "estimated_amount_atomic"):
        if not isinstance(payload.get(field), str) or not payload[field]:
            _raise("schema_invalid", 2, "phase-7", f"/cloudevent/data_json/{field}")


def _verify_signed_event(event: dict, trust_store: dict) -> None:
    if not isinstance(event, dict):
        _raise("canonical_json_invalid", 2, "phase-8", None)
    _validate_signed_event_shape(event)
    signature = event.get("signature") if isinstance(event, dict) else None
    if not isinstance(signature, dict) or signature.get("algorithm") != "Ed25519":
        _raise("trust_key_not_found", 3, "phase-9", "/signature/key_id")

    cloudevent = event.get("cloudevent")
    if not isinstance(cloudevent, dict):
        _raise("canonical_json_invalid", 2, "phase-8", "/cloudevent")
    key_index, key = _select_trust_key(trust_store, signature, cloudevent)
    public_key_value = key.get("public_key_base64url")
    if not isinstance(public_key_value, str):
        _raise("trust_store_invalid", 2, "phase-7", f"/keys/{key_index}/public_key_base64url")
    public_key = _decode_base64url(
        public_key_value,
        Diagnostic("trust_store_invalid", 2, "phase-7", f"/keys/{key_index}/public_key_base64url"),
    )
    if len(public_key) != 32:
        _raise("trust_store_invalid", 2, "phase-7", f"/keys/{key_index}/public_key_base64url")

    canonical = _ledger_canonical_bytes(event)
    actual_hash = hashlib.sha256(canonical).hexdigest()
    if event.get("canonical_bytes_sha256") != actual_hash:
        _raise("cloudevent_canonical_hash_mismatch", 4, "phase-8", "/canonical_bytes_sha256")

    signature_value = signature.get("value_base64url")
    if not isinstance(signature_value, str):
        _raise("spendguard_signature_invalid", 3, "phase-9", "/signature/value_base64url")
    signature_bytes = _decode_base64url(
        signature_value,
        Diagnostic("spendguard_signature_invalid", 3, "phase-9", "/signature/value_base64url"),
    )
    if len(signature_bytes) != 64:
        _raise("spendguard_signature_invalid", 3, "phase-9", "/signature/value_base64url")

    try:
        verified = _verify_ed25519_signature(public_key, signature_bytes, canonical)
    except RuntimeError as exc:
        raise VerificationError(Diagnostic("package_probe_drift", 6, "phase-4")) from exc
    if not verified:
        pointer = (
            f"/keys/{key_index}/public_key_base64url"
            if public_key_value != EXPECTED_TRUST_PUBLIC_KEY
            else "/signature/value_base64url"
        )
        _raise("spendguard_signature_invalid", 3, "phase-9", pointer)


def _verify_policy_binding(decision: dict, policy_bundle: dict) -> None:
    _require_keys(
        policy_bundle,
        {"enforcement_mode", "policy_bundle_hash", "schema", "system_prompt_hash"},
        "schema_invalid",
        2,
        "phase-7",
        "/policy-bundle.json",
    )
    if policy_bundle.get("schema") != "spendguard-policy-bundle/v1":
        _raise("schema_invalid", 2, "phase-7", "/policy-bundle.json/schema")
    if policy_bundle.get("enforcement_mode") != "enforce":
        _raise("schema_invalid", 2, "phase-7", "/policy-bundle.json/enforcement_mode")
    _require_sha256_or_null(policy_bundle.get("system_prompt_hash"), "schema_invalid", 2, "phase-7", "/policy-bundle.json/system_prompt_hash")
    policy_hash = policy_bundle.get("policy_bundle_hash") if isinstance(policy_bundle, dict) else None
    cloudevent = decision.get("cloudevent") if isinstance(decision, dict) else None
    data = cloudevent.get("data_json") if isinstance(cloudevent, dict) else None
    spendguard = data.get("spendguard") if isinstance(data, dict) else None
    signed_policy_hash = (
        spendguard.get("policy_bundle_hash") if isinstance(spendguard, dict) else None
    )
    if not isinstance(policy_hash, str) or policy_hash != signed_policy_hash:
        _raise("policy_binding_mismatch", 4, "phase-10", "/policy_bundle_hash")


def _verify_tool_binding(decision: dict, tool_catalog: dict) -> None:
    _require_keys(
        tool_catalog,
        {"catalog_hash", "schema", "tools_hash"},
        "schema_invalid",
        2,
        "phase-7",
        "/tool-catalog.json",
    )
    if tool_catalog.get("schema") != "spendguard-tool-catalog/v1":
        _raise("schema_invalid", 2, "phase-7", "/tool-catalog.json/schema")
    _require_sha256_or_null(tool_catalog.get("tools_hash"), "schema_invalid", 2, "phase-7", "/tool-catalog.json/tools_hash")
    cloudevent = decision.get("cloudevent") if isinstance(decision, dict) else None
    data = cloudevent.get("data_json") if isinstance(cloudevent, dict) else None
    spendguard = data.get("spendguard") if isinstance(data, dict) else None
    signed_tool_hash = (
        spendguard.get("tool_catalog_hash") if isinstance(spendguard, dict) else None
    )
    if tool_catalog.get("catalog_hash") != signed_tool_hash:
        _raise("tool_binding_mismatch", 4, "phase-10", "/tool_catalog_hash")


def _verify_build_binding(decision: dict, build_provenance: dict) -> None:
    fields = ("container_image_digest", "release_bundle_digest", "sbom_digest", "source_commit_digest")
    _require_keys(
        build_provenance,
        {"builder", *fields},
        "schema_invalid",
        2,
        "phase-7",
        "/build-provenance.json",
    )
    if not isinstance(build_provenance.get("builder"), str) or not build_provenance["builder"]:
        _raise("schema_invalid", 2, "phase-7", "/build-provenance.json/builder")
    for field in fields:
        _require_sha256_or_null(
            build_provenance.get(field),
            "schema_invalid",
            2,
            "phase-7",
            f"/build-provenance.json/{field}",
        )
    cloudevent = decision.get("cloudevent") if isinstance(decision, dict) else None
    data = cloudevent.get("data_json") if isinstance(cloudevent, dict) else None
    spendguard = data.get("spendguard") if isinstance(data, dict) else None
    signed_build = spendguard.get("build") if isinstance(spendguard, dict) else None
    if not isinstance(signed_build, dict):
        _raise("build_binding_mismatch", 4, "phase-10", "/build")
    selected = None
    for field in fields:
        signed_value = signed_build.get(field)
        wrapper_value = build_provenance.get(field)
        if signed_value != wrapper_value:
            _raise("build_binding_mismatch", 4, "phase-10", f"/build/{field}")
        if selected is None and isinstance(signed_value, str):
            selected = signed_value
    if selected is None:
        _raise("build_binding_mismatch", 4, "phase-10", "/build")


def _verify_lifecycle(bundle: dict, decision: dict, outcome: dict | None, fixture: str) -> None:
    cloudevent = decision["cloudevent"]
    if bundle.get("tenant_id") != cloudevent.get("tenant_id"):
        _raise("lifecycle_mismatch", 4, "phase-10", "/tenant_id")
    if bundle.get("run_id") != cloudevent.get("run_id"):
        _raise("lifecycle_mismatch", 4, "phase-10", "/run_id")
    if bundle.get("decision_id") != cloudevent.get("decision_id"):
        _raise("lifecycle_mismatch", 4, "phase-10", "/decision_id")
    if bundle.get("created_at") != cloudevent.get("time"):
        _raise("lifecycle_mismatch", 4, "phase-10", "/created_at")
    data = cloudevent["data_json"]
    reason_codes = data["reason_codes"]
    derived = (
        "deny"
        if "spendguard.deny" in reason_codes or bool(data.get("matched_rules"))
        else "allow"
    )
    if bundle.get("decision_path") != derived or fixture != derived:
        _raise("lifecycle_mismatch", 4, "phase-10", "/decision_path")
    if derived == "deny":
        if not data["matched_rules"]:
            _raise("lifecycle_mismatch", 4, "phase-10", "/cloudevent/data_json/matched_rules")
        if SHA256_DIGEST_RE.fullmatch(data["snapshot_hash"]) is None:
            _raise("lifecycle_mismatch", 4, "phase-10", "/cloudevent/data_json/snapshot_hash")
        if outcome is not None:
            _raise("lifecycle_mismatch", 4, "phase-10", "/outcome")
        return
    if outcome is None:
        _raise("lifecycle_mismatch", 4, "phase-10", "/outcome")
    outcome_event = outcome["cloudevent"]
    if outcome_event.get("type") != "spendguard.audit.outcome":
        _raise("lifecycle_mismatch", 4, "phase-10", "/cloudevent/type")
    for field in ("tenant_id", "run_id", "decision_id"):
        if outcome_event.get(field) != cloudevent.get(field):
            _raise("lifecycle_mismatch", 4, "phase-10", f"/cloudevent/{field}")
    if outcome["signature"].get("key_id") != decision["signature"].get("key_id"):
        _raise("lifecycle_mismatch", 4, "phase-10", "/signature/key_id")
    decision_time = _parse_rfc3339_millis(cloudevent["time"])[0]
    outcome_time = _parse_rfc3339_millis(outcome_event["time"])[0]
    if outcome_time < decision_time:
        _raise("lifecycle_mismatch", 4, "phase-10", "/cloudevent/time")
    outcome_data = outcome_event["data_json"]
    if outcome_data.get("kind") != "reservation_created":
        _raise("lifecycle_mismatch", 4, "phase-10", "/cloudevent/data_json/kind")
    if outcome_data.get("decision_id") != cloudevent.get("decision_id"):
        _raise("lifecycle_mismatch", 4, "phase-10", "/cloudevent/data_json/decision_id")
    if UUID4_RE.fullmatch(outcome_data.get("reservation_id", "")) is None:
        _raise("lifecycle_mismatch", 4, "phase-10", "/cloudevent/data_json/reservation_id")
    if AMOUNT_ATOMIC_RE.fullmatch(outcome_data.get("estimated_amount_atomic", "")) is None:
        _raise("lifecycle_mismatch", 4, "phase-10", "/cloudevent/data_json/estimated_amount_atomic")


def _ledger_canonical_bytes(event: dict) -> bytes:
    cloudevent = event.get("cloudevent") if isinstance(event, dict) else None
    signature = event.get("signature") if isinstance(event, dict) else None
    if not isinstance(cloudevent, dict) or not isinstance(signature, dict):
        _raise("canonical_json_invalid", 2, "phase-8", None)
    _validate_cloudevent_data_b64(cloudevent)
    producer_id = _canonical_string(cloudevent, "producer_id")
    if not producer_id.startswith("ledger:"):
        return _cloudevent_proto_canonical_bytes(event)

    seconds, nanos = _parse_rfc3339_millis(_canonical_string(cloudevent, "time"))
    payload = {
        "specversion": _canonical_string(cloudevent, "specversion"),
        "type": _canonical_string(cloudevent, "type"),
        "source": _canonical_string(cloudevent, "source"),
        "id": _canonical_string(cloudevent, "id"),
        "time_seconds": seconds,
        "time_nanos": nanos,
        "datacontenttype": _canonical_string(cloudevent, "datacontenttype"),
        "data_b64": _canonical_string(cloudevent, "data_b64"),
        "tenantid": _canonical_string(cloudevent, "tenant_id"),
        "runid": _canonical_string(cloudevent, "run_id"),
        "decisionid": _canonical_string(cloudevent, "decision_id"),
        "schema_bundle_id": _canonical_string(cloudevent, "schema_bundle_id"),
        "producer_id": producer_id,
        "producer_sequence": cloudevent.get("producer_sequence"),
        "signing_key_id": _canonical_string(signature, "key_id"),
    }
    if not isinstance(payload["producer_sequence"], int) or payload["producer_sequence"] < 0:
        _raise("canonical_json_invalid", 2, "phase-8", "/cloudevent/producer_sequence")
    return json.dumps(payload, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def _cloudevent_proto_canonical_bytes(event: dict) -> bytes:
    cloudevent = event.get("cloudevent") if isinstance(event, dict) else None
    signature = event.get("signature") if isinstance(event, dict) else None
    if not isinstance(cloudevent, dict) or not isinstance(signature, dict):
        _raise("canonical_json_invalid", 2, "phase-8", None)
    _validate_cloudevent_data_b64(cloudevent)
    seconds, nanos = _parse_rfc3339_millis(_canonical_string(cloudevent, "time"))
    producer_sequence = cloudevent.get("producer_sequence")
    if (
        not isinstance(producer_sequence, int)
        or isinstance(producer_sequence, bool)
        or producer_sequence < 0
        or producer_sequence > MAX_PRODUCER_SEQUENCE
    ):
        _raise("canonical_json_invalid", 2, "phase-8", "/cloudevent/producer_sequence")
    return b"".join(
        (
            _proto_string_field(1, _canonical_string(cloudevent, "specversion")),
            _proto_string_field(2, _canonical_string(cloudevent, "type")),
            _proto_string_field(3, _canonical_string(cloudevent, "source")),
            _proto_string_field(4, _canonical_string(cloudevent, "id")),
            _proto_timestamp_field(5, seconds, nanos),
            _proto_string_field(6, _canonical_string(cloudevent, "datacontenttype")),
            _proto_bytes_field(7, _decode_data_b64(_canonical_string(cloudevent, "data_b64"))),
            _proto_string_field(100, _canonical_string(cloudevent, "tenant_id")),
            _proto_string_field(101, _canonical_string(cloudevent, "run_id")),
            _proto_string_field(102, _canonical_string(cloudevent, "decision_id")),
            _proto_string_field(103, _canonical_string(cloudevent, "schema_bundle_id")),
            _proto_string_field(200, _canonical_string(cloudevent, "producer_id")),
            _proto_varint_field(201, producer_sequence),
            _proto_string_field(203, _canonical_string(signature, "key_id")),
        )
    )


def _check_cloudevent_canonicalization_probe() -> None:
    event = {
        "cloudevent": PROTO_VECTOR_CLOUDEVENT,
        "signature": {"key_id": "sg-ed25519-2026-06"},
    }
    observed = _cloudevent_proto_canonical_bytes(event)
    if (
        hashlib.sha256(observed).hexdigest() != PROTO_VECTOR_EXPECTED_SHA256
        or not observed.hex().startswith(PROTO_VECTOR_EXPECTED_HEX_PREFIX)
    ):
        _raise("package_probe_drift", 6, "phase-4", "/cloudevent/proto")


def _proto_string_field(field_number: int, value: str) -> bytes:
    return _proto_bytes_field(field_number, value.encode("utf-8"))


def _proto_bytes_field(field_number: int, value: bytes) -> bytes:
    if not value:
        return b""
    return _varint((field_number << 3) | 2) + _varint(len(value)) + value


def _proto_varint_field(field_number: int, value: int) -> bytes:
    if value == 0:
        return b""
    return _varint(field_number << 3) + _varint(value)


def _proto_timestamp_field(field_number: int, seconds: int, nanos: int) -> bytes:
    payload = _proto_varint_field(1, seconds) + _proto_varint_field(2, nanos)
    return _proto_bytes_field(field_number, payload)


def _varint(value: int) -> bytes:
    if value < 0:
        _raise("canonical_json_invalid", 2, "phase-8", None)
    output = bytearray()
    while value > 0x7F:
        output.append((value & 0x7F) | 0x80)
        value >>= 7
    output.append(value)
    return bytes(output)


def _validate_cloudevent_data_b64(cloudevent: dict) -> None:
    data_json = cloudevent.get("data_json")
    if not isinstance(data_json, dict):
        _raise("canonical_json_invalid", 2, "phase-8", "/cloudevent/data_json")
    data_b64 = _canonical_string(cloudevent, "data_b64")
    decoded = _decode_data_b64(data_b64)
    encoded_json = json.dumps(
        data_json,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")
    if decoded != encoded_json:
        _raise("data_b64_mismatch", 2, "phase-8", "/cloudevent/data_b64")


def _decode_data_b64(value: str) -> bytes:
    if "-" in value or "_" in value or len(value) % 4 != 0:
        _raise("data_b64_mismatch", 2, "phase-8", "/cloudevent/data_b64")
    try:
        return base64.b64decode(value, validate=True)
    except (binascii.Error, ValueError) as exc:
        raise VerificationError(
            Diagnostic("data_b64_mismatch", 2, "phase-8", "/cloudevent/data_b64")
        ) from exc


def _verify_trace_signing_key(key_bytes: bytes, fixture: str) -> None:
    _validate_trace_private_key_pem_format(key_bytes)
    try:
        from cryptography.exceptions import InvalidSignature
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

        key = serialization.load_pem_private_key(key_bytes, password=None)
        if not isinstance(key, Ed25519PrivateKey):
            _raise("trace_key_invalid", 6, "phase-4", None)
        public_key = key.public_key()
        public_bytes = public_key.public_bytes(Encoding.Raw, PublicFormat.Raw)
        public_value = base64.urlsafe_b64encode(public_bytes).rstrip(b"=").decode("ascii")
        if public_value != EXPECTED_TRUST_PUBLIC_KEY:
            _raise("trace_key_invalid", 6, "phase-4", None)
        trace = json.loads(_render_output(fixture, "trace-level0.json", DEFAULT_ISSUER))
        signature_value = trace.pop("signature", None)
        if not isinstance(signature_value, str):
            _raise("trace_package_validation_failed", 5, "phase-11", "/signature")
        signature = _decode_base64url(
            signature_value,
            Diagnostic("trace_package_validation_failed", 5, "phase-11", "/signature"),
        )
        message = json.dumps(trace, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
        try:
            public_key.verify(signature, message)
        except InvalidSignature:
            _raise("trace_key_invalid", 6, "phase-4", None)
    except ImportError as exc:
        raise VerificationError(Diagnostic("package_missing", 6, "phase-4")) from exc
    except (TypeError, ValueError) as exc:
        raise VerificationError(Diagnostic("trace_key_invalid", 6, "phase-4")) from exc


def _validate_trace_private_key_pem_format(key_bytes: bytes) -> None:
    stripped = key_bytes.strip()
    if stripped != key_bytes.rstrip() or not stripped:
        _raise("trace_key_invalid", 6, "phase-4", None)
    if stripped.count(PRIVATE_KEY_BEGIN) != 1 or stripped.count(PRIVATE_KEY_END) != 1:
        _raise("trace_key_invalid", 6, "phase-4", None)
    if stripped.count(b"-----BEGIN ") != 1 or stripped.count(b"-----END ") != 1:
        _raise("trace_key_invalid", 6, "phase-4", None)
    if not stripped.startswith(PRIVATE_KEY_BEGIN + b"\n") or not stripped.endswith(
        b"\n" + PRIVATE_KEY_END
    ):
        _raise("trace_key_invalid", 6, "phase-4", None)
    body = stripped[len(PRIVATE_KEY_BEGIN) + 1 : -len(PRIVATE_KEY_END) - 1]
    if not body or b"\n\n" in body or b"\r" in body:
        _raise("trace_key_invalid", 6, "phase-4", None)
    for line in body.split(b"\n"):
        if not line or re.fullmatch(rb"[A-Za-z0-9+/=]+", line) is None:
            _raise("trace_key_invalid", 6, "phase-4", None)


def _verify_ed25519_signature(public_key: bytes, signature: bytes, message: bytes) -> bool:
    try:
        from cryptography.exceptions import InvalidSignature
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

        try:
            Ed25519PublicKey.from_public_bytes(public_key).verify(signature, message)
            return True
        except InvalidSignature:
            return False
    except ImportError:
        return _verify_ed25519_signature_with_openssl(public_key, signature, message)


def _verify_ed25519_signature_with_openssl(public_key: bytes, signature: bytes, message: bytes) -> bool:
    pem = _public_key_pem(public_key)
    with tempfile.TemporaryDirectory() as temp:
        temp_dir = Path(temp)
        pub_path = temp_dir / "ed25519.pub.pem"
        sig_path = temp_dir / "signature.bin"
        msg_path = temp_dir / "message.bin"
        pub_path.write_bytes(pem)
        sig_path.write_bytes(signature)
        msg_path.write_bytes(message)
        try:
            observed = subprocess.run(
                [
                    "openssl",
                    "pkeyutl",
                    "-verify",
                    "-pubin",
                    "-inkey",
                    str(pub_path),
                    "-sigfile",
                    str(sig_path),
                    "-rawin",
                    "-in",
                    str(msg_path),
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
        except OSError as exc:
            raise RuntimeError("openssl unavailable for Ed25519 verification") from exc
    if observed.returncode == 0:
        return True
    if b"Signature Verification Failure" in observed.stdout + observed.stderr:
        return False
    raise RuntimeError("openssl Ed25519 verification failed unexpectedly")


def _public_key_pem(public_key: bytes) -> bytes:
    der = ED25519_SPKI_PREFIX + public_key
    body = base64.encodebytes(der).replace(b"\n", b"")
    lines = [body[index : index + 64] for index in range(0, len(body), 64)]
    return b"-----BEGIN PUBLIC KEY-----\n" + b"\n".join(lines) + b"\n-----END PUBLIC KEY-----\n"


def _decode_base64url(value: str, diagnostic: Diagnostic) -> bytes:
    if "=" in value or BASE64URL_RE.fullmatch(value) is None:
        raise VerificationError(diagnostic)
    try:
        return base64.urlsafe_b64decode(value + "=" * ((4 - len(value) % 4) % 4))
    except (binascii.Error, ValueError) as exc:
        raise VerificationError(diagnostic) from exc


def _canonical_string(container: dict, key: str) -> str:
    value = container.get(key)
    if not isinstance(value, str):
        _raise("canonical_json_invalid", 2, "phase-8", f"/cloudevent/{key}")
    for char in value:
        codepoint = ord(char)
        if codepoint < 0x20 or codepoint > 0x7E or char in {'"', "\\"}:
            _raise("canonical_json_invalid", 2, "phase-8", f"/cloudevent/{key}")
    return value


def _parse_rfc3339_millis(value: str) -> tuple[int, int]:
    if RFC3339_MILLIS_RE.fullmatch(value) is None:
        _raise("canonical_json_invalid", 2, "phase-8", "/cloudevent/time")
    try:
        parsed = datetime.strptime(value, "%Y-%m-%dT%H:%M:%S.%fZ").replace(tzinfo=timezone.utc)
    except ValueError as exc:
        raise VerificationError(Diagnostic("canonical_json_invalid", 2, "phase-8", "/cloudevent/time")) from exc
    if parsed.microsecond % 1000 != 0:
        _raise("canonical_json_invalid", 2, "phase-8", "/cloudevent/time")
    return int(parsed.timestamp()), parsed.microsecond * 1000


def _parse_rfc3339_seconds(value: str, pointer: str) -> int:
    try:
        parsed = datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
    except ValueError as exc:
        raise VerificationError(Diagnostic("trust_store_invalid", 2, "phase-7", pointer)) from exc
    return int(parsed.timestamp())


def _read_json(
    path: Path,
    code: str,
    exit_code: int,
    phase: str,
    boundary: RuntimeReadBoundary,
):
    diagnostic = Diagnostic(code, exit_code, phase)
    try:
        return _loads_canonical_json(
            _runtime_read_bytes(path, "json_input", boundary),
            diagnostic,
        )
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, DuplicateJsonKeyError) as exc:
        raise VerificationError(diagnostic) from exc


def _loads_json(data: bytes, diagnostic: Diagnostic):
    if data.startswith(b"\xef\xbb\xbf"):
        raise VerificationError(diagnostic)

    def no_duplicate_object(pairs):
        observed = set()
        result = {}
        for key, value in pairs:
            if key in observed:
                raise DuplicateJsonKeyError(key)
            observed.add(key)
            result[key] = value
        return result

    def reject_json_constant(value: str):
        raise ValueError(value)

    try:
        payload = json.loads(
            data.decode("utf-8"),
            object_pairs_hook=no_duplicate_object,
            parse_constant=reject_json_constant,
        )
    except (DuplicateJsonKeyError, ValueError) as exc:
        raise VerificationError(diagnostic) from exc
    return payload


def _loads_canonical_json(data: bytes, diagnostic: Diagnostic):
    payload = _loads_json(data, diagnostic)
    try:
        canonical = json.dumps(
            payload,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=False,
            allow_nan=False,
        ).encode("utf-8")
    except ValueError as exc:
        raise VerificationError(diagnostic) from exc
    if data != canonical:
        raise VerificationError(diagnostic)
    return payload


def _public_runtime_read_boundary(
    values: dict[str, str],
    fixture: str,
    bundle_id: str,
) -> RuntimeReadBoundary:
    boundary = _runtime_read_boundary("public", values, fixture, bundle_id)
    if boundary.bundle_root != f"integrations/agentrust/examples/{fixture}":
        _runtime_boundary_violation()
    if boundary.trust_store != PUBLIC_TRUST_STORE_PATH:
        _runtime_boundary_violation()
    if boundary.trace_signing_key != PUBLIC_TRACE_KEY_PATH:
        _runtime_boundary_violation()
    if boundary.out_dir != PUBLIC_OUTPUT_ROOT:
        _runtime_boundary_violation()
    _reject_argument_symlinks(
        boundary.bundle_root,
        Diagnostic("path_safety_error", 2, "phase-2", "/bundle"),
    )
    _validate_runtime_path_components(
        boundary.bundle_root,
        Diagnostic("path_safety_error", 2, "phase-2", "/bundle"),
    )
    _reject_argument_symlinks(
        boundary.trust_store,
        Diagnostic("path_safety_error", 2, "phase-2", "/trust-store"),
    )
    _validate_runtime_path_components(
        boundary.trust_store,
        Diagnostic("path_safety_error", 2, "phase-2", "/trust-store"),
    )
    _reject_argument_symlinks(
        boundary.trace_signing_key,
        Diagnostic("trace_key_invalid", 6, "phase-4"),
    )
    _validate_runtime_path_components(
        boundary.trace_signing_key,
        Diagnostic("trace_key_invalid", 6, "phase-4"),
    )
    _reject_argument_symlinks(
        boundary.out_dir,
        Diagnostic("output_target_invalid", 2, "phase-2"),
    )
    _validate_runtime_path_components(
        boundary.out_dir,
        Diagnostic("output_target_invalid", 2, "phase-2"),
        require_leaf=False,
    )
    return boundary


def _private_runtime_read_boundary(
    values: dict[str, str],
    fixture: str,
    bundle_id: str,
    mutation: str | None = None,
) -> RuntimeReadBoundary:
    boundary = _runtime_read_boundary("private", values, fixture, bundle_id, private=True)
    run_root = _private_run_root(boundary.trace_signing_key)
    if mutation is None:
        _validate_private_success_boundary(boundary, fixture, run_root)
    else:
        _validate_private_mutation_boundary(boundary, fixture, mutation, run_root)
    _reject_argument_symlinks(boundary.bundle_root)
    _validate_runtime_path_components(boundary.bundle_root)
    _reject_argument_symlinks(boundary.trust_store)
    _validate_runtime_path_components(boundary.trust_store)
    _reject_argument_symlinks(
        boundary.trace_signing_key,
        Diagnostic("trace_key_invalid", 6, "phase-4"),
    )
    _validate_runtime_path_components(
        boundary.trace_signing_key,
        Diagnostic("trace_key_invalid", 6, "phase-4"),
    )
    _reject_argument_symlinks(boundary.out_dir)
    _validate_runtime_path_components(boundary.out_dir)
    return boundary


def _runtime_read_boundary(
    mode: str,
    values: dict[str, str],
    fixture: str,
    bundle_id: str,
    *,
    private: bool = False,
) -> RuntimeReadBoundary:
    return RuntimeReadBoundary(
        mode=mode,
        fixture=fixture,
        bundle_id=bundle_id,
        bundle_root=_runtime_arg_path(values["bundle"], private=private),
        trust_store=_runtime_arg_path(values["trust_store"], private=private),
        trace_signing_key=_runtime_arg_path(values["trace_signing_key"], private=private),
        out_dir=_runtime_arg_path(values["out_dir"], private=private),
    )


def _record_runtime_read_run(mode: str) -> None:
    _write_runtime_read_evidence(
        {
            "schema": RUNTIME_READ_EVIDENCE_SCHEMA,
            "event": "run",
            "mode": mode,
        }
    )


def _runtime_read_text(path: Path, operation: str, boundary: RuntimeReadBoundary) -> str:
    return _runtime_read_bytes(path, operation, boundary).decode("utf-8")


def _runtime_read_bytes(path: Path | str, operation: str, boundary: RuntimeReadBoundary) -> bytes:
    repo_path = _runtime_read_repo_path(path, operation)
    _reject_symlink_components(repo_path, operation)
    if not _runtime_read_allowed(repo_path, operation, boundary):
        _block_runtime_read(repo_path, operation)

    data = _read_regular_file_no_follow(repo_path, operation)
    _write_runtime_read_evidence(
        {
            "schema": RUNTIME_READ_EVIDENCE_SCHEMA,
            "event": "read",
            "operation": operation,
            "path": repo_path,
            "bytes": len(data),
        }
    )
    return data


def _runtime_read_trace_key_bytes(path: Path | str, boundary: RuntimeReadBoundary) -> bytes:
    operation = "trace_signing_key"
    repo_path = _runtime_read_repo_path(path, operation)
    _reject_symlink_components(repo_path, operation)
    if not _runtime_read_allowed(repo_path, operation, boundary):
        _block_runtime_read(repo_path, operation)
    _validate_trace_key_metadata(repo_path, boundary)
    data = _read_regular_file_no_follow(repo_path, operation)
    _write_runtime_read_evidence(
        {
            "schema": RUNTIME_READ_EVIDENCE_SCHEMA,
            "event": "read",
            "operation": operation,
            "path": repo_path,
            "bytes": len(data),
        }
    )
    return data


def _validate_trace_key_metadata(repo_path: str, boundary: RuntimeReadBoundary) -> None:
    abs_path = _repo_abs(repo_path)
    try:
        status = abs_path.lstat()
    except OSError as exc:
        _trace_key_invalid(repo_path)
    mode = status.st_mode
    if stat.S_ISLNK(mode) or _unsafe_public_trace_key_metadata(status):
        _trace_key_invalid(repo_path)
    if boundary.mode == "private" and (mode & 0o777) != 0o600:
        _trace_key_invalid(repo_path)


def _trace_key_invalid(repo_path: str) -> None:
    _write_runtime_read_evidence(
        {
            "schema": RUNTIME_READ_EVIDENCE_SCHEMA,
            "event": "blocked",
            "operation": "trace_signing_key",
            "path": repo_path,
            "result": "trace_key_invalid",
        }
    )
    raise VerificationError(Diagnostic("trace_key_invalid", 6, "phase-4"))


def _runtime_write_bytes(
    path: Path | str,
    data: bytes,
    operation: str,
    boundary: RuntimeReadBoundary,
) -> None:
    repo_path = _runtime_read_repo_path(path, operation)
    _reject_symlink_components(repo_path, operation)
    if not _runtime_write_allowed(repo_path, operation, boundary):
        _block_runtime_read(repo_path, operation)

    if operation == "mutation_write":
        _runtime_replace_bytes(repo_path, data, operation)
        _write_runtime_read_evidence(
            {
                "schema": RUNTIME_READ_EVIDENCE_SCHEMA,
                "event": "write",
                "operation": operation,
                "path": repo_path,
                "bytes": len(data),
            }
        )
        return

    create_output = operation == "output_write"
    flags = os.O_WRONLY | (os.O_CREAT | os.O_EXCL if create_output else os.O_TRUNC)
    try:
        fd = _open_repo_file_no_follow(repo_path, operation, flags)
    except OSError:
        _block_runtime_filesystem(repo_path, operation)
    try:
        try:
            offset = 0
            while offset < len(data):
                written = os.write(fd, data[offset:])
                if written <= 0:
                    _block_runtime_filesystem(repo_path, operation)
                offset += written
        except OSError:
            _block_runtime_filesystem(repo_path, operation)
    finally:
        os.close(fd)
    _write_runtime_read_evidence(
        {
            "schema": RUNTIME_READ_EVIDENCE_SCHEMA,
            "event": "write",
            "operation": operation,
            "path": repo_path,
            "bytes": len(data),
        }
    )


def _runtime_replace_bytes(repo_path: str, data: bytes, operation: str) -> None:
    existing_fd = _open_repo_file_no_follow(repo_path, operation, os.O_RDONLY)
    os.close(existing_fd)
    parent_fd, leaf = _open_repo_parent_dir_no_follow(repo_path, operation)
    temp_leaf = f"{leaf}.tmp-{secrets.token_hex(8)}"
    temp_abs = _repo_abs(Path(repo_path).parent / temp_leaf)
    try:
        try:
            fd = os.open(
                temp_leaf,
                os.O_WRONLY | os.O_CREAT | os.O_EXCL | _open_no_follow_flag(operation),
                0o600,
                dir_fd=parent_fd,
            )
        except OSError:
            _block_runtime_read(repo_path, operation)
        try:
            offset = 0
            while offset < len(data):
                written = os.write(fd, data[offset:])
                if written <= 0:
                    _block_runtime_read(repo_path, operation)
                offset += written
        finally:
            os.close(fd)
        os.replace(temp_leaf, leaf, src_dir_fd=parent_fd, dst_dir_fd=parent_fd)
    except VerificationError:
        with contextlib.suppress(OSError):
            temp_abs.unlink()
        raise
    except OSError:
        with contextlib.suppress(OSError):
            temp_abs.unlink()
        _block_runtime_read(repo_path, operation)
    finally:
        os.close(parent_fd)


def _private_run_root(trace_signing_key: str) -> str:
    if not trace_signing_key.endswith(TRACE_KEY_COPY_SUFFIX):
        _private_cli_value_invalid()
    run_root = trace_signing_key[: -len(TRACE_KEY_COPY_SUFFIX)]
    parts = run_root.split("/")
    if (
        len(parts) != 5
        or parts[:4] != ["integrations", "agentrust", "out", ".verify-agentrust"]
        or RUN_ID_RE.fullmatch(parts[4]) is None
    ):
        _private_cli_value_invalid()
    return run_root


def _validate_private_success_boundary(
    boundary: RuntimeReadBoundary,
    fixture: str,
    run_root: str,
) -> None:
    if boundary.bundle_root != f"integrations/agentrust/examples/{fixture}":
        _private_cli_value_invalid()
    if boundary.trust_store != PUBLIC_TRUST_STORE_PATH:
        _private_cli_value_invalid()
    if boundary.out_dir != run_root:
        _private_cli_value_invalid()


def _validate_private_mutation_boundary(
    boundary: RuntimeReadBoundary,
    fixture: str,
    mutation: str,
    run_root: str,
) -> None:
    mutation_root = f"{run_root}/.mutations/{mutation}/{fixture}"
    if boundary.bundle_root != f"{mutation_root}/bundle":
        _private_cli_value_invalid()
    if boundary.trust_store != f"{mutation_root}/trust-store.json":
        _private_cli_value_invalid()
    if boundary.out_dir != f"{mutation_root}/out":
        _private_cli_value_invalid()


def _runtime_arg_path(value: str, *, private: bool = False) -> str:
    if private:
        return _raw_arg_path(
            value,
            allowed_hidden_segments=ALLOWED_RAW_HIDDEN_SEGMENTS,
            invalid=_private_cli_value_invalid,
        )
    return _raw_arg_path(value, allowed_hidden_segments=set(), invalid=_runtime_boundary_violation)


def _raw_arg_path(value: str, *, allowed_hidden_segments: set[str], invalid) -> str:
    if (
        not value
        or value.startswith("/")
        or "\x00" in value
        or "\\" in value
        or "//" in value
    ):
        invalid()
    parts = value.split("/")
    for part in parts:
        if part in {"", ".", ".."}:
            invalid()
        if len(part) == 2 and part[1] == ":":
            invalid()
        for char in part:
            codepoint = ord(char)
            if codepoint < 0x21 or codepoint > 0x7E or 0xD800 <= codepoint <= 0xDFFF:
                invalid()
        if part.startswith(".") and part not in allowed_hidden_segments:
            invalid()
    return value


def _read_regular_file_no_follow(repo_path: str, operation: str) -> bytes:
    fd = _open_repo_file_no_follow(repo_path, operation, os.O_RDONLY)
    try:
        chunks = []
        while True:
            chunk = os.read(fd, 1024 * 1024)
            if not chunk:
                break
            chunks.append(chunk)
        return b"".join(chunks)
    finally:
        os.close(fd)


def _mkdir_output_temp_dir(path: Path, boundary: RuntimeReadBoundary) -> None:
    operation = "output_temp_dir"
    repo_path = _runtime_read_repo_path(path, operation)
    _reject_symlink_components(repo_path, operation)
    if not _output_temp_dir_allowed(repo_path, boundary):
        _block_runtime_read(repo_path, operation)
    parent_fd, leaf = _open_repo_parent_dir_no_follow(repo_path, operation)
    try:
        try:
            os.mkdir(leaf, 0o700, dir_fd=parent_fd)
        except OSError:
            _block_runtime_filesystem(repo_path, operation)
    finally:
        os.close(parent_fd)
    dir_fd = _open_repo_dir_no_follow(repo_path, operation)
    os.close(dir_fd)


def _open_repo_file_no_follow(
    repo_path: str,
    operation: str,
    flags: int,
    mode: int = 0o600,
) -> int:
    parent_fd, leaf = _open_repo_parent_dir_no_follow(repo_path, operation)
    try:
        try:
            fd = os.open(leaf, flags | _open_no_follow_flag(operation), mode, dir_fd=parent_fd)
        except OSError as exc:
            if not flags & os.O_WRONLY and exc.errno != errno_module.ELOOP:
                raise
            _block_runtime_filesystem(repo_path, operation)
    finally:
        os.close(parent_fd)

    try:
        _validate_opened_file_metadata(os.fstat(fd), repo_path, operation)
    except Exception:
        os.close(fd)
        raise
    return fd


def _open_repo_parent_dir_no_follow(repo_path: str, operation: str) -> tuple[int, str]:
    parts = repo_path.split("/")
    if not parts or not parts[-1]:
        _block_runtime_read(repo_path, operation)
    dir_path = "/".join(parts[:-1])
    return _open_repo_dir_no_follow(dir_path, operation), parts[-1]


def _open_repo_dir_no_follow(repo_path: str, operation: str) -> int:
    fd = _open_root_dir_no_follow(operation)
    if not repo_path:
        return fd
    current_parts = []
    for part in repo_path.split("/"):
        current_parts.append(part)
        current_repo_path = "/".join(current_parts)
        try:
            next_fd = os.open(part, _directory_open_flags(operation), dir_fd=fd)
        except OSError:
            os.close(fd)
            _block_runtime_read(current_repo_path, operation)
        os.close(fd)
        fd = next_fd
        try:
            _validate_opened_dir_metadata(os.fstat(fd), current_repo_path, operation)
        except Exception:
            os.close(fd)
            raise
    return fd


def _open_root_dir_no_follow(operation: str) -> int:
    try:
        fd = os.open(os.fspath(ROOT), _directory_open_flags(operation))
    except OSError:
        _block_runtime_read(os.fspath(ROOT), operation)
    try:
        _validate_opened_dir_metadata(os.fstat(fd), os.fspath(ROOT), operation)
    except Exception:
        os.close(fd)
        raise
    return fd


def _directory_open_flags(operation: str) -> int:
    flags = os.O_RDONLY | _open_no_follow_flag(operation)
    if hasattr(os, "O_DIRECTORY"):
        flags |= os.O_DIRECTORY
    return flags


def _open_no_follow_flag(operation: str) -> int:
    if not hasattr(os, "O_NOFOLLOW"):
        _block_runtime_read("", operation)
    return os.O_NOFOLLOW


def _validate_opened_dir_metadata(status: os.stat_result, repo_path: str, operation: str) -> None:
    if _unsafe_dir_metadata(status):
        _block_runtime_read(repo_path, operation)


def _validate_opened_file_metadata(status: os.stat_result, repo_path: str, operation: str) -> None:
    if _unsafe_file_metadata(status):
        _block_runtime_read(repo_path, operation)


def _reject_argument_symlinks(repo_path: str, diagnostic: Diagnostic | None = None) -> None:
    current = Path()
    for part in repo_path.split("/"):
        current = current / part
        if _repo_abs(current).is_symlink():
            if diagnostic is not None:
                raise VerificationError(diagnostic)
            _runtime_boundary_violation()


def _validate_runtime_path_components(
    repo_path: str,
    diagnostic: Diagnostic | None = None,
    *,
    require_leaf: bool = True,
) -> None:
    current = Path()
    parts = repo_path.split("/")
    for index, part in enumerate(parts):
        current = current / part
        abs_current = _repo_abs(current)
        try:
            status = abs_current.stat()
        except OSError as exc:
            if isinstance(exc, FileNotFoundError) and index == len(parts) - 1 and not require_leaf:
                return
            if diagnostic is not None:
                raise VerificationError(diagnostic) from exc
            _runtime_boundary_violation()
        unsafe = (
            _unsafe_dir_metadata(status)
            if stat.S_ISDIR(status.st_mode)
            else _unsafe_file_metadata(status)
        )
        if unsafe:
            if diagnostic is not None:
                raise VerificationError(diagnostic)
            _runtime_boundary_violation()


def _runtime_read_repo_path(path: Path | str, operation: str) -> str:
    repo_path = os.fspath(path)
    if not repo_path:
        _block_runtime_read(repo_path, operation)
    if repo_path.startswith("/") or "\x00" in repo_path or "\\" in repo_path or "//" in repo_path:
        _block_runtime_read(repo_path, operation)
    for part in repo_path.split("/"):
        if part in {"", ".", ".."}:
            _block_runtime_read(repo_path, operation)
        for char in part:
            codepoint = ord(char)
            if codepoint < 0x21 or codepoint > 0x7E or 0xD800 <= codepoint <= 0xDFFF:
                _block_runtime_read(repo_path, operation)
        allowed_hidden = part in ALLOWED_RAW_HIDDEN_SEGMENTS or (
            operation in {"output_readback", "output_write", "output_temp_dir"}
            and part.startswith(".tmp-")
        )
        if part.startswith(".") and not allowed_hidden:
            _block_runtime_read(repo_path, operation)
    return repo_path


def _reject_symlink_components(repo_path: str, operation: str) -> None:
    current = Path()
    for part in repo_path.split("/"):
        current = current / part
        if _repo_abs(current).is_symlink():
            _block_runtime_read(repo_path, operation)


def _runtime_read_allowed(
    repo_path: str,
    operation: str,
    boundary: RuntimeReadBoundary,
) -> bool:
    bundle_paths = {f"{boundary.bundle_root}/bundle.json"} | {
        f"{boundary.bundle_root}/{entry}" for entry in EXPECTED_BUNDLE_FILE_SETS[boundary.fixture]
    }
    if repo_path in bundle_paths:
        return True
    if repo_path == boundary.trust_store and operation in {"json_input", "mutation_input"}:
        return True
    if repo_path == boundary.trace_signing_key and operation == "trace_signing_key":
        return True
    if operation == "output_readback":
        return _output_readback_allowed(repo_path, boundary)
    return False


def _output_readback_allowed(repo_path: str, boundary: RuntimeReadBoundary) -> bool:
    prefix = f"{boundary.out_dir}/.tmp-{boundary.bundle_id}-"
    if not repo_path.startswith(prefix):
        return False
    relative = repo_path[len(boundary.out_dir) + 1 :]
    parts = relative.split("/")
    if len(parts) != 2 or parts[1] not in ALL_OUTPUTS:
        return False
    temp_name = parts[0]
    suffix = temp_name.removeprefix(f".tmp-{boundary.bundle_id}-")
    return (
        temp_name != suffix
        and len(suffix) == 32
        and all(char in "0123456789abcdef" for char in suffix)
    )


def _output_temp_dir_allowed(repo_path: str, boundary: RuntimeReadBoundary) -> bool:
    prefix = f"{boundary.out_dir}/.tmp-{boundary.bundle_id}-"
    if not repo_path.startswith(prefix):
        return False
    relative = repo_path[len(boundary.out_dir) + 1 :]
    parts = relative.split("/")
    if len(parts) != 1:
        return False
    temp_name = parts[0]
    suffix = temp_name.removeprefix(f".tmp-{boundary.bundle_id}-")
    return (
        temp_name != suffix
        and len(suffix) == 32
        and all(char in "0123456789abcdef" for char in suffix)
    )


def _runtime_write_allowed(
    repo_path: str,
    operation: str,
    boundary: RuntimeReadBoundary,
) -> bool:
    if operation == "output_write":
        return _output_readback_allowed(repo_path, boundary)
    if operation != "mutation_write":
        return False
    writable_bundle_paths = {
        f"{boundary.bundle_root}/bundle.json",
        f"{boundary.bundle_root}/decision.json",
        f"{boundary.bundle_root}/policy-bundle.json",
    }
    if _is_private_runtime_path(boundary.bundle_root) and repo_path in writable_bundle_paths:
        return True
    if _is_private_runtime_path(boundary.trust_store) and repo_path == boundary.trust_store:
        return True
    return False


def _is_private_runtime_path(repo_path: str) -> bool:
    return repo_path.startswith(PRIVATE_RUNTIME_ROOT) and repo_path != PRIVATE_RUNTIME_ROOT.rstrip("/")


def _block_runtime_read(repo_path: str, operation: str) -> None:
    _write_runtime_read_evidence(
        {
            "schema": RUNTIME_READ_EVIDENCE_SCHEMA,
            "event": "blocked",
            "operation": operation,
            "path": repo_path,
            "result": "runtime_read_boundary_violation",
        }
    )
    _runtime_boundary_violation()


def _block_runtime_filesystem(repo_path: str, operation: str) -> None:
    if operation in OUTPUT_RUNTIME_OPERATIONS:
        _write_runtime_read_evidence(
            {
                "schema": RUNTIME_READ_EVIDENCE_SCHEMA,
                "event": "blocked",
                "operation": operation,
                "path": repo_path,
                "result": "output_target_invalid",
            }
        )
        raise VerificationError(Diagnostic("output_target_invalid", 2, "phase-11"))
    _block_runtime_read(repo_path, operation)


def _runtime_boundary_violation() -> None:
    raise VerificationError(Diagnostic("runtime_read_boundary_violation", 6, "phase-4"))


def _private_cli_value_invalid() -> None:
    raise VerificationError(Diagnostic("cli_value_invalid", 2, "phase-1"))


def _write_runtime_read_evidence(payload: dict) -> None:
    if os.environ.get("SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT") != "1":
        return
    evidence_path = os.environ.get(RUNTIME_READ_EVIDENCE_ENV)
    if not evidence_path:
        return
    try:
        with open(evidence_path, "a", encoding="utf-8") as evidence:
            evidence.write(json.dumps(payload, separators=(",", ":"), ensure_ascii=False) + "\n")
    except OSError as exc:
        raise VerificationError(
            Diagnostic("runtime_read_evidence_write_failed", 6, "make-setup")
        ) from exc


def _raise(code: str, exit_code: int, phase: str, pointer: str | None) -> None:
    raise VerificationError(Diagnostic(code, exit_code, phase, pointer))
