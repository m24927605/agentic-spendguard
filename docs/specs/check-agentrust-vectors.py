#!/usr/bin/env python3
import hashlib
import json
import struct
import uuid
import base64
from datetime import datetime, timezone
from pathlib import Path

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

ROOT = Path(__file__).resolve().parent
REPO_ROOT = ROOT.parents[1]
UUID_VECTOR = ROOT / "agentrust-manifest-uuidv7-vector-2026-07-01.json"
BUNDLE_VECTOR = ROOT / "agentrust-bundle-root-vector-2026-07-01.json"
PROBE = ROOT / "agentrust-package-probe-2026-07-01.json"
CLOUDEVENT_VECTOR = ROOT / "agentrust-cloudevent-signature-vector-2026-07-01.json"
SCHEMA_SOURCE = ROOT / "agentrust-schema-source-v1alpha1.json"
FIXTURE_MANIFEST = REPO_ROOT / "integrations/agentrust/examples/fixture-sha256-manifest.v1alpha1.json"
GENERATED_OUTPUT_MANIFEST = (
    REPO_ROOT / "integrations/agentrust/examples/generated-output-sha256-manifest.v1alpha1.json"
)


def u32be(n: int) -> bytes:
    return struct.pack(">I", n)


def main() -> None:
    uuid_data = json.loads(UUID_VECTOR.read_text())
    vectors = uuid_data.get("vectors", [uuid_data])
    for v in vectors:
        check_uuid_vector(v)

    first_manifest_id = vectors[0]["manifest_id"]

    schema = json.loads(SCHEMA_SOURCE.read_text())

    b = json.loads(BUNDLE_VECTOR.read_text())
    check_bundle_vector(b)

    p = json.loads(PROBE.read_text())
    assert p["accepted_json_shapes"]["agent_manifest_unsigned"]["manifest_id"] == first_manifest_id
    trace = p["accepted_json_shapes"]["trace_signed_level0"]
    trace_pre = {k: v for k, v in trace.items() if k != "signature"}
    pre_b = json.dumps(trace_pre, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()
    final_b = json.dumps(trace, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()
    assert hashlib.sha256(pre_b).hexdigest() == "4c423140c0bdeaac2a4318013abb3944d43bcf74a25d33701ec4f52e4f0be5c0"
    assert hashlib.sha256(final_b).hexdigest() == "76a3e6d94ae6599047e5abe09c10faa84a0e2192948c7772559a3f477faf34a5"

    check_cloudevent_vector(json.loads(CLOUDEVENT_VECTOR.read_text()), schema)
    check_fixture_manifest(json.loads(FIXTURE_MANIFEST.read_text()), schema)
    check_generated_output_manifest(json.loads(GENERATED_OUTPUT_MANIFEST.read_text()), schema)
    print("agentrust_vector_checks=pass")


def check_bundle_vector(b: dict) -> None:
    preimage_s = b["canonical_preimage_without_lf"].encode()
    preimage = json.loads(preimage_s)
    assert json.dumps(preimage, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode() == preimage_s
    assert hashlib.sha256(preimage_s).hexdigest() == b["preimage_sha256"]
    assert b["expected_evidence_bundle_hash"] == "sha256:" + b["preimage_sha256"]
    assert set(preimage) == {
        "version",
        "bundle_id",
        "created_at",
        "tenant_id",
        "run_id",
        "decision_id",
        "decision_path",
        "files",
    }
    assert preimage["version"] == "spendguard-agentrust-evidence/v1"
    assert preimage["created_at"].endswith(".000Z")
    assert preimage["decision_path"] == "allow"
    assert [entry["path"] for entry in preimage["files"]] == [
        "decision.json",
        "outcome.json",
        "policy-bundle.json",
        "tool-catalog.json",
        "build-provenance.json",
    ]


def check_uuid_vector(v: dict) -> None:
    issued = datetime.fromisoformat(v["issued_at"].replace("Z", "+00:00"))
    assert issued.tzinfo == timezone.utc
    assert int(issued.timestamp()) == v["unix_seconds"]
    assert int(issued.timestamp() * 1000) == v["unix_ms"]

    ebh = v["evidence_bundle_hash"].encode()
    agent_id = v["agent_id"].encode()
    label = b"agent-manifest"
    preimage = u32be(len(ebh)) + ebh + u32be(len(agent_id)) + agent_id + u32be(len(label)) + label
    digest = hashlib.sha256(preimage).digest()
    assert len(preimage) == v["preimage_length"]
    assert digest.hex() == v["preimage_sha256"]

    bits = int.from_bytes(digest, "big")
    rand_a = bits >> (256 - 12)
    rand_b = (bits >> (256 - 12 - 62)) & ((1 << 62) - 1)
    low64 = (0b10 << 62) | rand_b
    uuid_int = (v["unix_ms"] << 80) | (0x7 << 76) | (rand_a << 64) | low64
    manifest_id = str(uuid.UUID(int=uuid_int))
    assert f"{rand_a:03x}" == v["rand_a_hex"]
    assert f"{rand_b:016x}" == v["rand_b_hex"]
    assert f"{low64:016x}" == v["low64_hex_after_variant"]
    assert manifest_id == v["manifest_id"]


def b64url_decode(s: str) -> bytes:
    return base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))


def assert_verifies(pub: Ed25519PublicKey, sig: bytes, msg: bytes) -> None:
    pub.verify(sig, msg)


def assert_rejects(pub: Ed25519PublicKey, sig: bytes, msg: bytes) -> None:
    try:
        pub.verify(sig, msg)
    except InvalidSignature:
        return
    raise AssertionError("signature unexpectedly verified")


def check_cloudevent_vector(v: dict, schema: dict) -> None:
    assert v["schema"] == "spendguard-agentrust-cloudevent-signature-vector/v1"

    data = v["payload"]["data_json_canonical_without_lf"].encode()
    payload = json.loads(data)
    check_decision_payload_shape(payload, schema)
    reparsed = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()
    assert reparsed == data
    assert hashlib.sha256(data).hexdigest() == v["payload"]["data_sha256"]
    assert base64.b64encode(data).decode() == v["payload"]["data_b64_standard_padded"]

    proto = bytes.fromhex(v["proto_branch"]["canonical_bytes_hex"])
    ledger = v["ledger_json_branch"]["canonical_bytes_utf8"].encode()
    assert hashlib.sha256(proto).hexdigest() == v["proto_branch"]["canonical_bytes_sha256"]
    assert hashlib.sha256(ledger).hexdigest() == v["ledger_json_branch"]["canonical_bytes_sha256"]
    assert json.loads(ledger)["data_b64"] == v["payload"]["data_b64_standard_padded"]

    pub = Ed25519PublicKey.from_public_bytes(b64url_decode(v["key"]["public_key_base64url"]))
    proto_sig = b64url_decode(v["proto_branch"]["signature_base64url"])
    ledger_sig = b64url_decode(v["ledger_json_branch"]["signature_base64url"])

    assert_verifies(pub, proto_sig, proto)
    assert_verifies(pub, ledger_sig, ledger)
    assert_rejects(pub, proto_sig, ledger)
    assert_rejects(pub, ledger_sig, proto)

    controls = v["negative_controls"]
    assert controls["proto_signature_verifies_proto_bytes"] is True
    assert controls["ledger_signature_verifies_ledger_json_bytes"] is True
    assert controls["proto_signature_verifies_ledger_json_bytes"] is False
    assert controls["ledger_signature_verifies_proto_bytes"] is False


def check_decision_payload_shape(payload: dict, schema: dict) -> None:
    required = schema["closed_objects"]["decision_payload"]["required"]
    assert set(payload) == set(required)
    assert set(payload["spendguard"]) == set(schema["closed_objects"]["payload_spendguard_bindings"]["required"])
    assert set(payload["spendguard"]["build"]) == set(schema["closed_objects"]["payload_build_bindings"]["required"])
    assert set(payload["model"]) == set(schema["closed_objects"]["model_identity_payload"]["required"])
    assert isinstance(payload["reason_codes"], list)
    assert isinstance(payload["matched_rules"], list)
    assert payload["spendguard"]["build"]["container_image_digest"].startswith("sha256:")
    assert payload["spendguard"]["build"]["release_bundle_digest"] is None
    assert payload["spendguard"]["build"]["sbom_digest"] is None
    assert payload["spendguard"]["build"]["source_commit_digest"] is None


def check_fixture_manifest(manifest: dict, schema: dict) -> None:
    assert manifest["schema"] == "spendguard-agentrust-fixture-sha256-manifest/v1alpha1"
    assert manifest["expected_bundle_ids"] == {"allow": "demo-allow", "deny": "demo-deny"}
    assert manifest["expected_spendguard_key_id"] == "sg-ed25519-2026-06"

    entries = manifest["entries"]
    assert entries == sorted(entries, key=lambda e: e["path"])
    expected_paths = {
        "integrations/agentrust/examples/allow/build-provenance.json",
        "integrations/agentrust/examples/allow/bundle.json",
        "integrations/agentrust/examples/allow/decision.json",
        "integrations/agentrust/examples/allow/outcome.json",
        "integrations/agentrust/examples/allow/policy-bundle.json",
        "integrations/agentrust/examples/allow/tool-catalog.json",
        "integrations/agentrust/examples/deny/build-provenance.json",
        "integrations/agentrust/examples/deny/bundle.json",
        "integrations/agentrust/examples/deny/decision.json",
        "integrations/agentrust/examples/deny/policy-bundle.json",
        "integrations/agentrust/examples/deny/tool-catalog.json",
        "integrations/agentrust/examples/trust-store.json",
        "integrations/agentrust/test-private-keys/spendguard-agentrust-fixture.test.ed25519.pem",
        "integrations/agentrust/test-private-keys/spendguard-agentrust-wrong.test.ed25519.pem",
    }
    assert {e["path"] for e in entries} == expected_paths
    for entry in entries:
        assert len(entry["sha256"]) == 64
        int(entry["sha256"], 16)
    assert len(b64url_decode(manifest["wrong_key_public_key_base64url"])) == 32


def check_generated_output_manifest(manifest: dict, schema: dict) -> None:
    assert manifest["schema"] == "spendguard-agentrust-generated-output-sha256-manifest/v1alpha1"
    assert manifest["public_out_dir"] == "integrations/agentrust/out"
    entries = manifest["entries"]
    assert entries == sorted(entries, key=lambda e: e["path"])
    expected = {
        "integrations/agentrust/examples/expected-outputs/allow/agent-manifest.json": (
            "allow",
            "agent_manifest",
            "integrations/agentrust/out/demo-allow/agent-manifest.json",
        ),
        "integrations/agentrust/examples/expected-outputs/allow/spendguard-artifacts.json": (
            "allow",
            "spendguard_artifacts",
            "integrations/agentrust/out/demo-allow/spendguard-artifacts.json",
        ),
        "integrations/agentrust/examples/expected-outputs/allow/trace-level0.json": (
            "allow",
            "trace_level0",
            "integrations/agentrust/out/demo-allow/trace-level0.json",
        ),
        "integrations/agentrust/examples/expected-outputs/allow/verification-report.json": (
            "allow",
            "verification_report",
            "integrations/agentrust/out/demo-allow/verification-report.json",
        ),
        "integrations/agentrust/examples/expected-outputs/deny/agent-manifest.json": (
            "deny",
            "agent_manifest",
            "integrations/agentrust/out/demo-deny/agent-manifest.json",
        ),
        "integrations/agentrust/examples/expected-outputs/deny/spendguard-artifacts.json": (
            "deny",
            "spendguard_artifacts",
            "integrations/agentrust/out/demo-deny/spendguard-artifacts.json",
        ),
        "integrations/agentrust/examples/expected-outputs/deny/trace-level0.json": (
            "deny",
            "trace_level0",
            "integrations/agentrust/out/demo-deny/trace-level0.json",
        ),
        "integrations/agentrust/examples/expected-outputs/deny/verification-report.json": (
            "deny",
            "verification_report",
            "integrations/agentrust/out/demo-deny/verification-report.json",
        ),
        "integrations/agentrust/examples/expected-outputs/verify-agentrust-report.json": (
            "make",
            "make_verification_report",
            "integrations/agentrust/out/.verify-agentrust/<run-id>/verify-agentrust-report.json",
        ),
    }
    assert {e["path"] for e in entries} == set(expected)
    for entry in entries:
        fixture, role, generated_path = expected[entry["path"]]
        assert entry["fixture"] == fixture
        assert entry["role"] == role
        assert entry["generated_path"] == generated_path
        path = REPO_ROOT / entry["path"]
        assert path.is_file(), entry["path"]
        data = path.read_bytes()
        assert hashlib.sha256(data).hexdigest() == entry["sha256"]
        if role == "make_verification_report":
            assert data.endswith(b"\n")
            report = json.loads(data)
            required = schema["closed_objects"]["make_verification_report"]["required"]
            assert set(report) == set(required)
            assert report["status"] == "pass"
            expected_conformance = {
                "artifact_preflight_envelope",
                "event_type_binding",
                "fixture_identity",
                "internal_guard",
                "manifest_issuer_variant",
                "package_guard_surfaces",
                "private_parser_matrix",
                "public_parser_matrix",
            }
            assert set(report["conformance_checks"]) == expected_conformance
            conformance_required = schema["closed_objects"]["conformance_check_result"]["required"]
            deferred_conformance = {
                "artifact_preflight_envelope",
                "event_type_binding",
                "manifest_issuer_variant",
                "private_parser_matrix",
                "public_parser_matrix",
            }
            for name, result in report["conformance_checks"].items():
                assert set(result) == set(conformance_required)
                if name in deferred_conformance:
                    assert result == {
                        "status": "deferred_fixture_mvp",
                        "diagnostic_code": "deferred_fixture_mvp",
                    }
                else:
                    assert result == {"status": "pass", "diagnostic_code": None}
            assert set(report["success_fixtures"]) == {"allow", "deny"}
            result_required = schema["closed_objects"]["success_fixture_result"]["required"]
            for fixture_name, bundle_id in {"allow": "demo-allow", "deny": "demo-deny"}.items():
                result = report["success_fixtures"][fixture_name]
                assert set(result) == set(result_required)
                assert result == {
                    "status": "pass",
                    "bundle_id": bundle_id,
                    "outputs_match": True,
                    "diagnostic_code": None,
                }
        else:
            assert not data.endswith(b"\n")


if __name__ == "__main__":
    main()
