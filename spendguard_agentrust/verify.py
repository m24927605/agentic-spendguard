from __future__ import annotations

import base64
import binascii
from datetime import datetime, timezone
import hashlib
import json
import secrets
import shutil
from pathlib import Path
import stat
import subprocess
import tempfile

from .diagnostics import Diagnostic
from .fixture_outputs import (
    ALL_OUTPUTS,
    BUNDLE_IDS,
    FIXTURE_BY_BUNDLE_PATH,
    OUTPUT_BYTES,
    SELECTED_OUTPUTS,
)


DEFAULT_ISSUER = "spiffe://spendguard.local/agentrust-exporter"
EXPECTED_EVIDENCE_ROOTS = {
    "allow": "sha256:a40343e65467451b5e0e234327a414fbf61b2cacff57fd41fa6804232c7194df",
    "deny": "sha256:985f76783f37f66d6d5ce957c586f5d4b5dc2106bc1c0075eadd65d1982ebcd5",
}
EXPECTED_TRUST_PUBLIC_KEY = "QwRr_kCSs-lJlOraFdzCDYqqB7ZY_TlU644O-4vcpd4"
ALLOWED_BUNDLE_ENTRY_PATHS = {
    "decision.json",
    "outcome.json",
    "policy-bundle.json",
    "tool-catalog.json",
    "build-provenance.json",
}
ED25519_SPKI_PREFIX = bytes.fromhex("302a300506032b6570032100")
PRIVATE_KEY_PEM_SENTINEL = ("-----BEGIN " + "PRIVATE KEY-----").encode("ascii")


class VerificationError(Exception):
    def __init__(self, diagnostic: Diagnostic):
        super().__init__(diagnostic.code)
        self.diagnostic = diagnostic


def verify_public(values: dict[str, str]) -> bytes:
    fixture = FIXTURE_BY_BUNDLE_PATH[values["bundle"]]
    bundle_id = BUNDLE_IDS[fixture]
    _validate_inputs(values, fixture, bundle_id)
    _commit_outputs(values, fixture, bundle_id)

    return json.dumps(
        {
            "bundle_id": bundle_id,
            "outputs": SELECTED_OUTPUTS,
            "schema": "spendguard-agentrust-verify-result/v1",
            "status": "pass",
        },
        separators=(",", ":"),
    ).encode("utf-8")


def _validate_inputs(values: dict[str, str], fixture: str, bundle_id: str) -> None:
    bundle_root = Path(values["bundle"])
    bundle = _read_json(bundle_root / "bundle.json", "bundle_index_invalid", 2, "phase-2")
    if bundle.get("bundle_id") != bundle_id:
        _raise("bundle_index_invalid", 2, "phase-2", "/bundle_id")

    files = bundle.get("files")
    if not isinstance(files, list):
        _raise("bundle_index_invalid", 2, "phase-2", "/files")

    for index, entry in enumerate(files):
        rel = entry.get("path") if isinstance(entry, dict) else None
        expected = entry.get("sha256") if isinstance(entry, dict) else None
        if not isinstance(rel, str) or not isinstance(expected, str):
            _raise("bundle_index_invalid", 2, "phase-2", f"/files/{index}")
        path = _bundle_entry_path(bundle_root, rel, index)
        try:
            actual = hashlib.sha256(path.read_bytes()).hexdigest()
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
    if expected_root != EXPECTED_EVIDENCE_ROOTS[fixture]:
        _raise("evidence_bundle_hash_mismatch", 4, "phase-6", "/evidence_bundle_hash")

    trust_store = _read_json(Path(values["trust_store"]), "trust_store_invalid", 2, "phase-7")
    keys = trust_store.get("keys") if isinstance(trust_store, dict) else None
    if not isinstance(keys, list) or not keys or keys[0].get("key_id") != "sg-ed25519-2026-06":
        _raise("trust_store_invalid", 2, "phase-7", "/keys/0/key_id")
    if keys[0].get("public_key_base64url") != EXPECTED_TRUST_PUBLIC_KEY:
        _raise("spendguard_signature_invalid", 3, "phase-9", "/keys/0/public_key_base64url")

    decision = _read_json(bundle_root / "decision.json", "canonical_json_invalid", 2, "phase-7")
    _verify_signed_event(decision, trust_store)
    if fixture == "allow":
        outcome = _read_json(bundle_root / "outcome.json", "canonical_json_invalid", 2, "phase-7")
        _verify_signed_event(outcome, trust_store)

    try:
        key_bytes = Path(values["trace_signing_key"]).read_bytes()
    except OSError as exc:
        raise VerificationError(Diagnostic("trace_key_invalid", 6, "phase-4")) from exc
    if PRIVATE_KEY_PEM_SENTINEL not in key_bytes:
        _raise("trace_key_invalid", 6, "phase-4", None)


def _commit_outputs(values: dict[str, str], fixture: str, bundle_id: str) -> None:
    output_dir = Path(values["out_dir"]) / bundle_id
    output_root = output_dir.parent
    if output_root.is_symlink():
        _raise("output_target_invalid", 2, "phase-2", None)
    if output_dir.exists() or output_dir.is_symlink():
        _raise("output_target_invalid", 2, "phase-2", None)

    output_root.mkdir(parents=True, exist_ok=True)
    temp_dir = output_root / f".tmp-{bundle_id}-{secrets.token_hex(16)}"
    try:
        temp_dir.mkdir(mode=0o700)
        for name in ALL_OUTPUTS:
            data = _render_output(fixture, name, values["manifest_issuer"])
            target = temp_dir / name
            target.write_bytes(data)
            if target.read_bytes() != data:
                _raise("output_target_invalid", 2, "phase-11", None)
        temp_dir.rename(output_dir)
    except VerificationError:
        shutil.rmtree(temp_dir, ignore_errors=True)
        raise
    except OSError as exc:
        shutil.rmtree(temp_dir, ignore_errors=True)
        raise VerificationError(Diagnostic("output_target_invalid", 2, "phase-11")) from exc


def _render_output(fixture: str, name: str, issuer: str) -> bytes:
    data = OUTPUT_BYTES[(fixture, name)]
    if name == "agent-manifest.json" and issuer != DEFAULT_ISSUER:
        payload = json.loads(data)
        payload["issuer"] = issuer
        return json.dumps(payload, separators=(",", ":")).encode("utf-8")
    return data


def _bundle_entry_path(bundle_root: Path, rel: str, index: int) -> Path:
    if rel not in ALLOWED_BUNDLE_ENTRY_PATHS:
        _raise("path_safety_error", 2, "phase-2", f"/files/{index}/path")
    target = bundle_root / rel
    try:
        mode = target.lstat().st_mode
    except OSError:
        return target
    if stat.S_ISLNK(mode) or not stat.S_ISREG(mode):
        _raise("path_safety_error", 2, "phase-2", f"/files/{index}/path")
    return target


def _verify_signed_event(event: dict, trust_store: dict) -> None:
    signature = event.get("signature") if isinstance(event, dict) else None
    if not isinstance(signature, dict) or signature.get("algorithm") != "Ed25519":
        _raise("trust_key_not_found", 3, "phase-9", "/signature/key_id")

    key_id = signature.get("key_id")
    if not isinstance(key_id, str):
        _raise("trust_key_not_found", 3, "phase-9", "/signature/key_id")

    keys = trust_store.get("keys") if isinstance(trust_store, dict) else None
    if not isinstance(keys, list):
        _raise("trust_store_invalid", 2, "phase-7", "/keys")

    selected: tuple[int, dict] | None = None
    for index, key in enumerate(keys):
        if isinstance(key, dict) and key.get("key_id") == key_id:
            selected = (index, key)
            break
    if selected is None:
        _raise("trust_key_not_found", 3, "phase-9", "/signature/key_id")

    key_index, key = selected
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


def _ledger_canonical_bytes(event: dict) -> bytes:
    cloudevent = event.get("cloudevent") if isinstance(event, dict) else None
    signature = event.get("signature") if isinstance(event, dict) else None
    if not isinstance(cloudevent, dict) or not isinstance(signature, dict):
        _raise("canonical_json_invalid", 2, "phase-8", None)
    producer_id = _canonical_string(cloudevent, "producer_id")
    if not producer_id.startswith("ledger:"):
        _raise("package_probe_drift", 6, "phase-4", "/cloudevent/producer_id")

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
    if "=" in value:
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
    try:
        parsed = datetime.strptime(value, "%Y-%m-%dT%H:%M:%S.%fZ").replace(tzinfo=timezone.utc)
    except ValueError as exc:
        raise VerificationError(Diagnostic("canonical_json_invalid", 2, "phase-8", "/cloudevent/time")) from exc
    if parsed.microsecond % 1000 != 0:
        _raise("canonical_json_invalid", 2, "phase-8", "/cloudevent/time")
    return int(parsed.timestamp()), parsed.microsecond * 1000


def _read_json(path: Path, code: str, exit_code: int, phase: str):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise VerificationError(Diagnostic(code, exit_code, phase)) from exc


def _raise(code: str, exit_code: int, phase: str, pointer: str | None) -> None:
    raise VerificationError(Diagnostic(code, exit_code, phase, pointer))
