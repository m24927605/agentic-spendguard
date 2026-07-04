from __future__ import annotations

from dataclasses import dataclass
import re
import sys
from urllib.parse import unquote_to_bytes

from .diagnostics import Diagnostic, diagnostic_json
from .verify import VerificationError, verify_public


HELP_STDOUT = (
    "usage: python -m spendguard_agentrust verify --bundle BUNDLE "
    "--trust-store TRUST_STORE --trace-signing-key TRACE_SIGNING_KEY "
    "--out-dir OUT_DIR [--mode fixture] [--fixture-profile upstream] "
    "[--manifest-issuer SPIFFE_URI]\n"
)

DEFAULT_ISSUER = "spiffe://spendguard.local/agentrust-exporter"
SUPPORTED_FLAGS = {
    "--bundle",
    "--trust-store",
    "--trace-signing-key",
    "--out-dir",
    "--mode",
    "--fixture-profile",
    "--manifest-issuer",
}
REQUIRED_FLAGS = ("--bundle", "--trust-store", "--trace-signing-key", "--out-dir")
FLAG_KEYS = {
    "--bundle": "bundle",
    "--trust-store": "trust_store",
    "--trace-signing-key": "trace_signing_key",
    "--out-dir": "out_dir",
    "--mode": "mode",
    "--fixture-profile": "fixture_profile",
    "--manifest-issuer": "manifest_issuer",
}
LITERAL_VALUES = {
    "--bundle": {
        "integrations/agentrust/examples/allow",
        "integrations/agentrust/examples/deny",
    },
    "--trust-store": {"integrations/agentrust/examples/trust-store.json"},
    "--trace-signing-key": {
        "integrations/agentrust/test-private-keys/"
        "spendguard-agentrust-fixture.test.ed25519.pem"
    },
    "--out-dir": {"integrations/agentrust/out"},
    "--mode": {"fixture"},
    "--fixture-profile": {"upstream"},
}
UNSUPPORTED_MODE_VALUES = {"live", "historical"}
UNSUPPORTED_PROFILE_VALUES = {"local", "=local"}
ISSUER_RE = re.compile(r"^spiffe://([a-z0-9.-]+)/(.+)$")
ISSUER_SEGMENT_RE = re.compile(r"^(?:[A-Za-z0-9._~!$&'()*+,;=:@-]|%[0-9A-F]{2})+$")


@dataclass(frozen=True)
class ParseResult:
    kind: str
    values: dict[str, str]
    diagnostic: Diagnostic | None = None


def parse_public_args(argv: list[str] | tuple[str, ...]) -> ParseResult:
    args = list(argv)
    if args == ["verify", "--help"]:
        return ParseResult("help", {})

    if not args or args[0] != "verify":
        return _diagnostic("cli_parse_error")

    tokens = args[1:]
    row1 = _missing_value_candidates(tokens)
    if row1:
        return _diagnostic("cli_missing_value")

    unsupported_index = _first_unsupported_capability(tokens)
    row2 = _structural_parse_error_candidates(tokens)
    eligible_row2 = [
        index for index in row2 if unsupported_index is None or index < unsupported_index
    ]
    if eligible_row2:
        return _diagnostic("cli_parse_error")
    if unsupported_index is not None:
        return _diagnostic("unsupported_capability")

    if _unknown_flag_candidates(tokens):
        return _diagnostic("cli_parse_error")
    if _duplicate_supported_flag(tokens):
        return _diagnostic("cli_parse_error")
    if _supported_equals_candidates(tokens):
        return _diagnostic("cli_equals_form_unsupported")

    values = _collect_values(tokens)
    if any(flag not in values for flag in REQUIRED_FLAGS):
        return _diagnostic("cli_missing_required_flag")

    if _invalid_value(values):
        return _diagnostic("cli_value_invalid")

    return ParseResult(
        "ok",
        {
            "bundle": values["--bundle"],
            "trust_store": values["--trust-store"],
            "trace_signing_key": values["--trace-signing-key"],
            "out_dir": values["--out-dir"],
            "mode": values.get("--mode", "fixture"),
            "fixture_profile": values.get("--fixture-profile", "upstream"),
            "manifest_issuer": values.get("--manifest-issuer", DEFAULT_ISSUER),
        },
    )


def main(argv: list[str] | None = None) -> int:
    parsed = parse_public_args(sys.argv[1:] if argv is None else argv)
    if parsed.kind == "help":
        sys.stdout.write(HELP_STDOUT)
        return 0
    if parsed.diagnostic is not None:
        sys.stderr.write(diagnostic_json(parsed.diagnostic))
        return parsed.diagnostic.exit_code

    try:
        sys.stdout.buffer.write(verify_public(parsed.values))
    except VerificationError as exc:
        sys.stderr.write(diagnostic_json(exc.diagnostic))
        return exc.diagnostic.exit_code
    return 0


def _diagnostic(code: str) -> ParseResult:
    return ParseResult("diagnostic", {}, Diagnostic(code))


def _missing_value_candidates(tokens: list[str]) -> list[int]:
    candidates: list[int] = []
    for index, token in enumerate(tokens):
        if token in SUPPORTED_FLAGS and (index + 1 == len(tokens) or tokens[index + 1].startswith("--")):
            candidates.append(index)
    return candidates


def _first_unsupported_capability(tokens: list[str]) -> int | None:
    indexes: list[int] = []
    for index, token in enumerate(tokens):
        next_token = tokens[index + 1] if index + 1 < len(tokens) else None
        if token == "--force" or token.startswith("--force="):
            indexes.append(index)
        elif token.startswith("--cmcp-") or token.startswith("--mcp-"):
            indexes.append(index)
        elif token == "--tested-against" or token.startswith("--tested-against="):
            indexes.append(index)
        elif token == "--mode" and next_token in UNSUPPORTED_MODE_VALUES:
            indexes.append(index)
        elif token.startswith("--mode=") and token.split("=", 1)[1] in UNSUPPORTED_MODE_VALUES:
            indexes.append(index)
        elif token == "--fixture-profile" and next_token in UNSUPPORTED_PROFILE_VALUES:
            indexes.append(index)
        elif token == "--fixture-profile=local":
            indexes.append(index)
    return min(indexes) if indexes else None


def _structural_parse_error_candidates(tokens: list[str]) -> list[int]:
    candidates: list[int] = []
    value_indexes = _supported_value_indexes(tokens) | _unsupported_pair_value_indexes(tokens)
    for index, token in enumerate(tokens):
        if index in value_indexes:
            continue
        if token == "--help" or token.startswith("--help="):
            candidates.append(index)
        elif _bad_token(token):
            candidates.append(index)
        elif not token.startswith("--"):
            candidates.append(index)
    return candidates


def _unknown_flag_candidates(tokens: list[str]) -> list[int]:
    candidates: list[int] = []
    value_indexes = _supported_value_indexes(tokens) | _unsupported_pair_value_indexes(tokens)
    for index, token in enumerate(tokens):
        if index in value_indexes:
            continue
        if not token.startswith("--") or token.startswith("--help"):
            continue
        if token in SUPPORTED_FLAGS or any(token.startswith(f"{flag}=") for flag in SUPPORTED_FLAGS):
            continue
        candidates.append(index)
    return candidates


def _duplicate_supported_flag(tokens: list[str]) -> bool:
    seen: set[str] = set()
    for token in tokens:
        if token in SUPPORTED_FLAGS:
            if token in seen:
                return True
            seen.add(token)
    return False


def _supported_equals_candidates(tokens: list[str]) -> list[int]:
    return [
        index
        for index, token in enumerate(tokens)
        if any(token.startswith(f"{flag}=") for flag in SUPPORTED_FLAGS)
    ]


def _collect_values(tokens: list[str]) -> dict[str, str]:
    values: dict[str, str] = {}
    index = 0
    while index < len(tokens):
        token = tokens[index]
        if token in SUPPORTED_FLAGS and index + 1 < len(tokens):
            values[token] = tokens[index + 1]
            index += 2
            continue
        index += 1
    return values


def _invalid_value(values: dict[str, str]) -> bool:
    for flag, value in values.items():
        if _bad_token(value):
            return True
        allowed = LITERAL_VALUES.get(flag)
        if allowed is not None and value not in allowed:
            return True
        if flag == "--manifest-issuer" and not _valid_manifest_issuer(value):
            return True
    return False


def _supported_value_indexes(tokens: list[str]) -> set[int]:
    indexes: set[int] = set()
    for index, token in enumerate(tokens[:-1]):
        if token in SUPPORTED_FLAGS and not tokens[index + 1].startswith("--"):
            indexes.add(index + 1)
    return indexes


def _unsupported_pair_value_indexes(tokens: list[str]) -> set[int]:
    indexes: set[int] = set()
    for index, token in enumerate(tokens[:-1]):
        next_token = tokens[index + 1]
        if token == "--tested-against" and not next_token.startswith("--"):
            indexes.add(index + 1)
        elif token == "--mode" and next_token in UNSUPPORTED_MODE_VALUES:
            indexes.add(index + 1)
        elif token == "--fixture-profile" and next_token in UNSUPPORTED_PROFILE_VALUES:
            indexes.add(index + 1)
    return indexes


def _bad_token(token: str) -> bool:
    if token == "":
        return True
    for char in token:
        value = ord(char)
        if value > 0x7E or value < 0x20 or 0xD800 <= value <= 0xDFFF:
            return True
    return False


def _valid_manifest_issuer(value: str) -> bool:
    if len(value) > 256 or len(value.encode("ascii", "ignore")) != len(value):
        return False
    match = ISSUER_RE.fullmatch(value)
    if not match:
        return False
    host, path = match.groups()
    if host.startswith(".") or host.endswith(".") or ".." in host:
        return False
    for label in host.split("."):
        if not label or label.startswith("-") or label.endswith("-"):
            return False
    if "?" in path or "#" in path or "\\" in path or "//" in path:
        return False
    for segment in path.split("/"):
        if not segment or not ISSUER_SEGMENT_RE.fullmatch(segment):
            return False
        try:
            decoded = unquote_to_bytes(segment)
        except ValueError:
            return False
        if any(byte < 0x21 or byte > 0x7E or byte in (0x2F, 0x3F, 0x23, 0x5C) for byte in decoded):
            return False
    return True
