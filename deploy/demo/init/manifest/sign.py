"""Generate the demo endpoint catalog manifest + signing keypair.

Layout produced under /manifest (mounted into nginx as the HTTPS root +
into the sidecar container as the verify-key path):

  /manifest/v1/catalog/manifest                — signed Manifest JSON
  /manifest/v1/catalog/<version_id>            — catalog body JSON
  /manifest/manifest_verify_key.pub.pem        — ed25519 public key (PEM)
  /manifest/manifest_signing_key.priv.pem      — ed25519 private key (PEM, demo only)

The signature scheme MUST match
services/sidecar/src/bootstrap/catalog.rs::verify_manifest_signature:
  - Recursive key-sort (publisher serializes with sort_keys behavior).
  - `tenant_overrides` field omitted when empty (skip-if-empty serde).
  - sign(canonical_utf8) → base64 → manifest.signature.

Idempotent: skips regeneration when verify-key + manifest already exist.
"""

import json
import os
import sys
import time
from datetime import datetime, timedelta, timezone
from hashlib import sha256
from pathlib import Path

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
    Ed25519PublicKey,
)
import base64


def canonical_sort(value):
    """Recursive key-sort matching the sidecar's verify_manifest_signature."""
    if isinstance(value, dict):
        return {k: canonical_sort(value[k]) for k in sorted(value)}
    if isinstance(value, list):
        return [canonical_sort(v) for v in value]
    return value


def main() -> int:
    out = Path("/manifest")
    catalog_dir = out / "v1" / "catalog"
    catalog_dir.mkdir(parents=True, exist_ok=True)

    version_id = os.environ.get("CATALOG_VERSION_ID", "ctlg-demo-rev1")
    serve_host = os.environ.get("CATALOG_SERVE_HOST", "endpoint-catalog")
    serve_port = os.environ.get("CATALOG_SERVE_PORT", "8443")
    valid_hours = int(os.environ.get("MANIFEST_VALID_HOURS", "24"))

    verify_key_path = out / "manifest_verify_key.pub.pem"
    sign_key_path = out / "manifest_signing_key.priv.pem"
    manifest_path = catalog_dir / "manifest"
    body_path = catalog_dir / version_id

    if (
        verify_key_path.exists()
        and manifest_path.exists()
        and body_path.exists()
    ):
        # Manifest validity is short-lived; re-issue when older than half
        # the valid window so the sidecar's catalog refresh always sees
        # a fresh manifest (sidecar fail-closes when valid_until lapses).
        with manifest_path.open() as f:
            existing = json.load(f)
        # The persisted manifest uses Z suffix; convert to +00:00 so
        # Python's strict fromisoformat (pre-3.11) accepts it.
        valid_until = datetime.fromisoformat(
            existing["valid_until"].replace("Z", "+00:00")
        )
        now = datetime.now(tz=timezone.utc)
        half_window = timedelta(hours=valid_hours / 2)
        if valid_until > now + half_window:
            print(
                f"[manifest] existing manifest valid_until={valid_until} is fresh, skipping"
            )
            return 0
        print(
            f"[manifest] existing manifest valid_until={valid_until} stale, re-signing"
        )

    # 1. Keypair (re-use if signing key already present, so the public
    #    key the sidecar pinned at first compose-up still verifies).
    if sign_key_path.exists():
        with sign_key_path.open("rb") as f:
            priv = serialization.load_pem_private_key(f.read(), password=None)
        if not isinstance(priv, Ed25519PrivateKey):
            print("[manifest] existing signing key is not ed25519", file=sys.stderr)
            return 1
    else:
        priv = Ed25519PrivateKey.generate()
        sign_key_path.write_bytes(
            priv.private_bytes(
                encoding=serialization.Encoding.PEM,
                format=serialization.PrivateFormat.PKCS8,
                encryption_algorithm=serialization.NoEncryption(),
            )
        )
        sign_key_path.chmod(0o640)
        verify_key_path.write_bytes(
            priv.public_key().public_bytes(
                encoding=serialization.Encoding.PEM,
                format=serialization.PublicFormat.SubjectPublicKeyInfo,
            )
        )
        verify_key_path.chmod(0o644)
        print("[manifest] minted new ed25519 keypair")

    # 2. Catalog body — opaque JSON; sidecar caches by sha256 hash.
    body = {
        "version_id": version_id,
        "issued_at": datetime.now(tz=timezone.utc).isoformat(timespec="seconds"),
        "endpoints": {
            "ledger": {
                "regions": {
                    "demo": {
                        "url": "https://ledger:50051",
                        "sni": "ledger.spendguard.internal",
                    }
                }
            },
            "canonical_ingest": {
                "regions": {
                    "demo": {
                        "url": "https://canonical-ingest:50052",
                        "sni": "canonical-ingest.spendguard.internal",
                    }
                }
            },
        },
    }
    body_bytes = json.dumps(body, sort_keys=True, separators=(",", ":")).encode("utf-8")
    body_path.write_bytes(body_bytes)
    body_hash = sha256(body_bytes).hexdigest()
    print(f"[manifest] catalog body sha256={body_hash} size={len(body_bytes)}")

    # 3. Manifest fields — must match the sidecar's parser exactly.
    issued_at = datetime.now(tz=timezone.utc)
    valid_until = issued_at + timedelta(hours=valid_hours)

    # Canonical timestamp form: `2026-05-07T08:42:30Z` (second precision,
    # `Z` UTC suffix). We pin this format here so the canonical bytes
    # are independent of chrono version drift. The sidecar's
    # verify_manifest_signature uses
    # `to_rfc3339_opts(SecondsFormat::Secs, true)` for the same reason.
    iso_z = lambda dt: dt.isoformat(timespec="seconds").replace("+00:00", "Z")
    manifest_payload = {
        "manifest_version":            "v1",
        "current_catalog_version_id":  version_id,
        "current_catalog_url":         f"https://{serve_host}:{serve_port}/v1/catalog/{version_id}",
        "current_catalog_hash":        body_hash,
        "issued_at":                   iso_z(issued_at),
        "valid_until":                 iso_z(valid_until),
        "signing_key_id":              "demo-manifest-key",
    }

    canonical_str = json.dumps(
        canonical_sort(manifest_payload),
        separators=(",", ":"),
        ensure_ascii=False,
    )
    sig = priv.sign(canonical_str.encode("utf-8"))
    manifest_payload["signature"] = base64.standard_b64encode(sig).decode("ascii")

    manifest_path.write_text(json.dumps(manifest_payload, indent=2) + "\n")
    print(f"[manifest] wrote {manifest_path} valid_until={manifest_payload['valid_until']}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
