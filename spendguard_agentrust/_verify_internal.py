from __future__ import annotations

import os
import sys

from .diagnostics import Diagnostic, diagnostic_json
from .verify import VerificationError, _commit_outputs, _validate_inputs


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


def main(argv: list[str] | None = None) -> int:
    if os.environ.get("SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT") != "1":
        return _emit(Diagnostic("packaging_guard_failed", 6, "make-setup"))

    args = sys.argv[1:] if argv is None else argv
    try:
        values = _parse_private_args(args)
        mutation = values.get("mutation")
        if mutation is not None:
            return _emit(MUTATION_DIAGNOSTICS[mutation])
        fixture = values["fixture"]
        _validate_inputs(values, fixture, f"demo-{fixture}")
        _commit_outputs(values, fixture, f"demo-{fixture}")
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


def _raise_cli(code: str) -> None:
    raise VerificationError(Diagnostic(code, 2, "phase-1"))


def _emit(diagnostic: Diagnostic) -> int:
    sys.stderr.write(diagnostic_json(diagnostic))
    return diagnostic.exit_code


if __name__ == "__main__":
    raise SystemExit(main())
