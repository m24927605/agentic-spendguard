"""SpendGuard predictor-client SVID validation helpers."""

from __future__ import annotations

from uuid import UUID

SVID_PREFIX = "spiffe://spendguard.platform/predictor-client/"


def expected_svid_subject(tenant_id: str) -> str:
    """Return the exact URI SAN SpendGuard must present for ``tenant_id``."""
    return f"{SVID_PREFIX}{UUID(tenant_id)}"


def tenant_from_svid_subject(subject: str) -> str:
    if not subject.startswith(SVID_PREFIX):
        raise ValueError("SVID subject has wrong prefix")
    return str(UUID(subject[len(SVID_PREFIX) :]))


def _normalise_auth_value(value: bytes | str) -> str:
    text = value.decode("utf-8", errors="replace") if isinstance(value, bytes) else value
    if text.startswith("URI:"):
        return text[4:]
    return text


def extract_spiffe_uri_from_auth_context(auth_context: dict[str | bytes, list[bytes]]) -> str | None:
    """Extract the SpendGuard SPIFFE URI SAN from ``grpc.ServicerContext`` auth data."""
    normalised: dict[str, list[bytes]] = {}
    for key, values in auth_context.items():
        key_text = key.decode("utf-8", errors="replace") if isinstance(key, bytes) else key
        normalised.setdefault(key_text, []).extend(values)
    uri_values: list[str] = []
    for key in ("x509_subject_alternative_name", "x509_common_name"):
        for raw in normalised.get(key, []):
            value = _normalise_auth_value(raw)
            if value.startswith("spiffe://"):
                uri_values.append(value)
    unique = sorted(set(uri_values))
    if len(unique) > 1:
        raise ValueError("multiple SVID URI subjects presented")
    if not unique:
        return None
    if not unique[0].startswith(SVID_PREFIX):
        raise ValueError("SVID subject has wrong prefix")
    return unique[0]


def validate_auth_context_tenant(
    *,
    auth_context: dict[str, list[bytes]],
    tenant_id: str,
    require_svid: bool,
) -> None:
    """Fail closed unless the peer cert SVID subject matches ``tenant_id``."""
    subject = extract_spiffe_uri_from_auth_context(auth_context)
    if not subject:
        if require_svid:
            raise ValueError("missing SpendGuard predictor-client SVID")
        return
    expected = expected_svid_subject(tenant_id)
    if subject != expected:
        raise ValueError(f"SVID tenant mismatch: expected {expected}, got {subject}")
