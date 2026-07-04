from __future__ import annotations

import hashlib
import json
import secrets
import shutil
from pathlib import Path

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
        try:
            actual = hashlib.sha256((bundle_root / rel).read_bytes()).hexdigest()
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

    try:
        key_bytes = Path(values["trace_signing_key"]).read_bytes()
    except OSError as exc:
        raise VerificationError(Diagnostic("trace_key_invalid", 6, "phase-4")) from exc
    if b"-----BEGIN PRIVATE KEY-----" not in key_bytes:
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


def _read_json(path: Path, code: str, exit_code: int, phase: str):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise VerificationError(Diagnostic(code, exit_code, phase)) from exc


def _raise(code: str, exit_code: int, phase: str, pointer: str | None) -> None:
    raise VerificationError(Diagnostic(code, exit_code, phase, pointer))
