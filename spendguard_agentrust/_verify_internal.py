from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import sys

from .cli import _valid_manifest_issuer
from .diagnostics import Diagnostic, diagnostic_json
from .verify import (
    VerificationError,
    _commit_outputs,
    _private_runtime_read_boundary,
    _record_runtime_read_run,
    _render_output,
    _runtime_read_bytes,
    _runtime_write_bytes,
    _loads_canonical_json,
    _validate_trace_package_bytes,
    _validate_inputs,
)


INTERNAL_READ_PROBE_ENV = "SPENDGUARD_AGENTRUST_INTERNAL_READ_PROBE_PATH"


PRIVATE_FLAGS = {
    "--bundle",
    "--trust-store",
    "--trace-signing-key",
    "--out-dir",
    "--fixture",
    "--manifest-issuer",
    "--mutation",
}
REQUIRED_FLAGS = {
    "--bundle",
    "--trust-store",
    "--trace-signing-key",
    "--out-dir",
    "--fixture",
}
MUTATION_DIAGNOSTICS = {
    "mutated_signature": Diagnostic(
        "spendguard_signature_invalid",
        3,
        "phase-9",
        "/signature/value_base64url",
    ),
    "wrong_key": Diagnostic(
        "spendguard_signature_invalid",
        3,
        "phase-9",
        "/keys/0/public_key_base64url",
    ),
    "mutated_policy_hash": Diagnostic(
        "policy_binding_mismatch",
        4,
        "phase-10",
        "/policy_bundle_hash",
    ),
    "missing_trace_signature": Diagnostic(
        "trace_package_validation_failed",
        5,
        "phase-11",
        "/signature",
    ),
}
MUTATION_FIXTURES = {
    "missing_trace_signature": "deny",
    "mutated_policy_hash": "allow",
    "mutated_signature": "allow",
    "wrong_key": "deny",
}


def main(argv: list[str] | None = None) -> int:
    if os.environ.get("SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT") != "1":
        return _emit(Diagnostic("packaging_guard_failed", 6, "make-setup"))

    args = sys.argv[1:] if argv is None else argv
    try:
        values = _parse_private_args(args)
        fixture = values["fixture"]
        mutation = values.get("mutation")
        boundary = _private_runtime_read_boundary(values, fixture, f"demo-{fixture}", mutation)
        _record_runtime_read_run("private")
        probe = os.environ.get(INTERNAL_READ_PROBE_ENV)
        if probe:
            _runtime_read_bytes(probe, "probe", boundary)
        if mutation is not None:
            return _run_mutation(values, fixture, mutation, boundary)
        _validate_inputs(values, fixture, f"demo-{fixture}", boundary)
        _commit_outputs(values, fixture, f"demo-{fixture}", boundary)
    except VerificationError as exc:
        return _emit(exc.diagnostic)
    return 0


def _parse_private_args(args: list[str]) -> dict[str, str]:
    if not args or args[0] != "verify":
        _raise_cli("cli_parse_error")

    values: dict[str, str] = {}
    index = 1
    while index < len(args):
        flag = args[index]
        if flag not in PRIVATE_FLAGS or "=" in flag:
            _raise_cli("cli_parse_error")
        if flag in values:
            _raise_cli("cli_parse_error")
        if index + 1 >= len(args) or args[index + 1].startswith("--"):
            _raise_cli("cli_missing_value")
        values[flag] = args[index + 1]
        index += 2

    if any(flag not in values for flag in REQUIRED_FLAGS):
        _raise_cli("cli_missing_required_flag")

    fixture = values["--fixture"]
    if fixture not in {"allow", "deny"}:
        _raise_cli("cli_value_invalid")

    mutation = values.get("--mutation")
    if mutation is not None and mutation not in MUTATION_DIAGNOSTICS:
        _raise_cli("cli_value_invalid")
    if mutation is not None and fixture != MUTATION_FIXTURES[mutation]:
        _raise_cli("cli_value_invalid")
    if mutation is not None and "--manifest-issuer" in values:
        _raise_cli("cli_value_invalid")
    if mutation is None and "--manifest-issuer" in values:
        if not _valid_manifest_issuer(values["--manifest-issuer"]):
            _raise_cli("cli_value_invalid")

    return {
        "bundle": values["--bundle"],
        "trust_store": values["--trust-store"],
        "trace_signing_key": values["--trace-signing-key"],
        "out_dir": values["--out-dir"],
        "fixture": fixture,
        "manifest_issuer": values.get(
            "--manifest-issuer",
            "spiffe://spendguard.local/agentrust-exporter",
        ),
        **({"mutation": mutation} if mutation is not None else {}),
    }


def _run_mutation(values: dict[str, str], fixture: str, mutation: str, boundary) -> int:
    expected = MUTATION_DIAGNOSTICS[mutation]
    try:
        _validate_inputs(values, fixture, f"demo-{fixture}", boundary)
    except VerificationError as exc:
        if _same_diagnostic(expected, exc.diagnostic):
            return _emit(Diagnostic("negative_mutation_setup_invalid", 6, "make-setup"))
        raise
    try:
        _apply_input_mutation(values, mutation, boundary)
        _validate_inputs(
            values,
            fixture,
            f"demo-{fixture}",
            boundary,
            strict_fixture_root=mutation not in {"mutated_signature", "mutated_policy_hash"},
        )
        if mutation == "missing_trace_signature":
            _validate_missing_trace_signature(values, fixture)
    except VerificationError as exc:
        if _same_diagnostic(expected, exc.diagnostic):
            return _emit(exc.diagnostic)
        raise
    return _emit(Diagnostic("negative_mutation_not_detected", 6, "make-setup"))


def _apply_input_mutation(values: dict[str, str], mutation: str, boundary) -> None:
    try:
        if mutation in {"mutated_signature", "mutated_policy_hash"}:
            _read_mutation_json(
                Path(values["bundle"]) / "bundle.json",
                boundary,
                Diagnostic("bundle_index_invalid", 2, "phase-2", "/files"),
            )
        if mutation == "mutated_signature":
            decision_path = Path(values["bundle"]) / "decision.json"
            decision = _read_mutation_json(decision_path, boundary)
            signature = decision["signature"]["value_base64url"]
            if not isinstance(signature, str) or not signature:
                raise VerificationError(
                    Diagnostic("canonical_json_invalid", 2, "phase-7", "/signature/value_base64url")
                )
            decision["signature"]["value_base64url"] = _flip_base64url_prefix(signature)
            decision_bytes = _json_bytes(decision)
            _loads_canonical_json(decision_bytes, Diagnostic("canonical_json_invalid", 2, "phase-7"))
            _runtime_write_bytes(decision_path, decision_bytes, "mutation_write", boundary)
            _update_bundle_entry(
                Path(values["bundle"]) / "bundle.json",
                "decision.json",
                decision_bytes,
                boundary,
            )
        elif mutation == "mutated_policy_hash":
            policy_path = Path(values["bundle"]) / "policy-bundle.json"
            policy = _read_mutation_json(policy_path, boundary)
            policy["policy_bundle_hash"] = "sha256:" + "9" * 64
            policy_bytes = _json_bytes(policy)
            _loads_canonical_json(policy_bytes, Diagnostic("canonical_json_invalid", 2, "phase-7"))
            _runtime_write_bytes(policy_path, policy_bytes, "mutation_write", boundary)
            _update_bundle_entry(
                Path(values["bundle"]) / "bundle.json",
                "policy-bundle.json",
                policy_bytes,
                boundary,
            )
        elif mutation == "wrong_key":
            trust_path = Path(values["trust_store"])
            trust_store = _read_mutation_json(trust_path, boundary)
            trust_store["keys"][0]["public_key_base64url"] = (
                "GX9rI-FshTLGq8g4-s1ep4m-DHaykgM0A5v6iz02jWE"
            )
            trust_bytes = _json_bytes(trust_store)
            _loads_canonical_json(trust_bytes, Diagnostic("canonical_json_invalid", 2, "phase-7"))
            _runtime_write_bytes(
                trust_path,
                trust_bytes,
                "mutation_write",
                boundary,
            )
    except (KeyError, IndexError, TypeError, json.JSONDecodeError, UnicodeDecodeError) as exc:
        raise VerificationError(Diagnostic("canonical_json_invalid", 2, "phase-7")) from exc


def _update_bundle_entry(
    bundle_path: Path,
    entry_path: str,
    entry_bytes: bytes,
    boundary,
) -> None:
    bundle = _read_mutation_json(
        bundle_path,
        boundary,
        Diagnostic("bundle_index_invalid", 2, "phase-2", "/files"),
    )
    files = bundle.get("files") if isinstance(bundle, dict) else None
    if not isinstance(files, list):
        raise VerificationError(Diagnostic("bundle_index_invalid", 2, "phase-2", "/files"))
    for entry in files:
        if isinstance(entry, dict) and entry.get("path") == entry_path:
            entry["sha256"] = hashlib.sha256(entry_bytes).hexdigest()
            break
    else:
        raise VerificationError(Diagnostic("bundle_index_invalid", 2, "phase-2", "/files"))

    preimage = dict(bundle)
    preimage.pop("evidence_bundle_hash", None)
    bundle["evidence_bundle_hash"] = "sha256:" + hashlib.sha256(
        _json_bytes(preimage, sort_keys=True)
    ).hexdigest()
    _runtime_write_bytes(bundle_path, _json_bytes(bundle, sort_keys=True), "mutation_write", boundary)


def _validate_missing_trace_signature(values: dict[str, str], fixture: str) -> None:
    trace = json.loads(_render_output(fixture, "trace-level0.json", values["manifest_issuer"]))
    trace.pop("signature", None)
    _validate_trace_package_bytes(_json_bytes(trace))


def _read_mutation_json(path: Path, boundary, diagnostic: Diagnostic | None = None):
    expected = diagnostic or Diagnostic("canonical_json_invalid", 2, "phase-7")
    try:
        return _loads_canonical_json(
            _runtime_read_bytes(path, "mutation_input", boundary),
            expected,
        )
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise VerificationError(expected) from exc


def _json_bytes(payload: dict, *, trailing_lf: bool = False, sort_keys: bool = False) -> bytes:
    rendered = json.dumps(
        payload,
        sort_keys=sort_keys,
        separators=(",", ":"),
        ensure_ascii=False,
    )
    if trailing_lf:
        rendered += "\n"
    return rendered.encode("utf-8")


def _flip_base64url_prefix(value: str) -> str:
    if not value or value[0] != "B":
        raise VerificationError(Diagnostic("negative_mutation_setup_invalid", 6, "make-setup"))
    return "A" + value[1:]


def _same_diagnostic(expected: Diagnostic, observed: Diagnostic) -> bool:
    return (
        observed.exit_code == expected.exit_code
        and observed.phase == expected.phase
        and observed.code == expected.code
        and observed.json_pointer == expected.json_pointer
    )


def _raise_cli(code: str) -> None:
    raise VerificationError(Diagnostic(code, 2, "phase-1"))


def _emit(diagnostic: Diagnostic) -> int:
    sys.stderr.write(diagnostic_json(diagnostic))
    return diagnostic.exit_code


if __name__ == "__main__":
    raise SystemExit(main())
