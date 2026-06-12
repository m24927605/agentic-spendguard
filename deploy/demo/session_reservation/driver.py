#!/usr/bin/env python3
"""D41 session reservation substrate demo driver.

Runs the locked 7-step lifecycle against the demo Postgres ledger using the
same stored procedures the substrate tests exercise. Audit rows are signed with
the demo ledger Ed25519 key mounted in the Compose pki-data volume.
"""

from __future__ import annotations

import base64
import hashlib
import json
import subprocess
import sys
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any


DEMO_DIR = Path(__file__).resolve().parents[1]

TENANT_ID = "00000000-0000-4000-8000-000000000001"
BUDGET_ID = "44444444-4444-4444-8444-444444444444"
WINDOW_INSTANCE_ID = "55555555-5555-4555-8555-555555555555"
USD_UNIT_ID = "88888888-8888-4888-8888-888888888888"
PRICING_VERSION = "demo-pricing-v1"
FX_RATE_VERSION = "demo-fx-v1"
UNIT_CONVERSION_VERSION = "demo-units-v1"
ROUTE = "livekit/pipecat-session-reservation-demo"
SESSION_ID = "d41-session-reservation-demo"
SESSION_ID_DENIED = "d41-session-reservation-deny-demo"
SESSION_ID_EXPIRED = "d41-session-reservation-expire-demo"
PRODUCER_ID = "ledger:session-reservation-ledger"


def run(cmd: list[str], *, input_text: str | None = None, check: bool = True) -> subprocess.CompletedProcess[str]:
    proc = subprocess.run(
        cmd,
        cwd=DEMO_DIR,
        input=input_text,
        text=True,
        capture_output=True,
        check=False,
    )
    if check and proc.returncode != 0:
        sys.stderr.write(proc.stdout)
        sys.stderr.write(proc.stderr)
        raise SystemExit(proc.returncode)
    return proc


def compose(*args: str, input_text: str | None = None, check: bool = True) -> subprocess.CompletedProcess[str]:
    return run(["docker", "compose", "-f", "compose.yaml", *args], input_text=input_text, check=check)


def psql(sql: str, *, check: bool = True) -> str:
    proc = compose(
        "exec",
        "-T",
        "postgres",
        "psql",
        "-U",
        "spendguard",
        "-d",
        "spendguard_ledger",
        "-v",
        "ON_ERROR_STOP=1",
        "-v",
        "VERBOSITY=verbose",
        "-tA",
        "-c",
        sql,
        check=check,
    )
    return proc.stdout.strip()


def call_json_function(function_name: str, payload: dict[str, Any], *, check: bool = True) -> tuple[int, dict[str, Any] | None, str]:
    raw = json.dumps(payload, sort_keys=True, separators=(",", ":"))
    sql = f"SELECT {function_name}($sg${raw}$sg$::jsonb)::text;"
    proc = compose(
        "exec",
        "-T",
        "postgres",
        "psql",
        "-U",
        "spendguard",
        "-d",
        "spendguard_ledger",
        "-v",
        "ON_ERROR_STOP=1",
        "-v",
        "VERBOSITY=verbose",
        "-tA",
        "-c",
        sql,
        check=False,
    )
    if check and proc.returncode != 0:
        sys.stderr.write(proc.stdout)
        sys.stderr.write(proc.stderr)
        raise SystemExit(proc.returncode)
    if proc.returncode != 0:
        return proc.returncode, None, proc.stderr
    return 0, json.loads(proc.stdout.strip()), proc.stderr


def pricing_hash_hex() -> str:
    sql = (
        "SELECT encode(price_snapshot_hash, 'hex') "
        f"FROM pricing_snapshots WHERE pricing_version = '{PRICING_VERSION}'"
    )
    value = psql(sql)
    if not value:
        raise SystemExit("[demo] session_reservation FATAL: missing demo pricing snapshot")
    return value


def next_audit_sequences() -> tuple[int, int]:
    raw = psql("SELECT nextval_per_shard(1::smallint)::text || '|' || nextval_per_shard(1::smallint)::text;")
    left, right = raw.split("|", 1)
    return int(left), int(right)


def ledger_signing_key_id() -> str:
    script = (
        "openssl pkey -in /pki/signing/ledger.pem -pubout -outform DER "
        "| tail -c 32 | openssl dgst -sha256 -binary | xxd -p -c 256"
    )
    proc = compose("run", "--rm", "--no-deps", "--entrypoint", "sh", "pki-init", "-c", script)
    digest_hex = proc.stdout.strip()
    if len(digest_hex) < 16:
        raise SystemExit("[demo] session_reservation FATAL: could not derive ledger signing key id")
    return f"ed25519:{digest_hex[:16]}"


def sign_canonical(canonical_bytes: bytes) -> str:
    digest_hex = hashlib.sha256(canonical_bytes).hexdigest()
    script = (
        "set -eu; "
        "xxd -r -p >/tmp/spendguard-session-digest.bin; "
        "openssl pkeyutl -sign -rawin "
        "-inkey /pki/signing/ledger.pem "
        "-in /tmp/spendguard-session-digest.bin | xxd -p -c 256"
    )
    proc = compose(
        "run",
        "--rm",
        "--no-deps",
        "--entrypoint",
        "sh",
        "pki-init",
        "-c",
        script,
        input_text=digest_hex,
    )
    sig_hex = proc.stdout.strip()
    if len(sig_hex) != 128:
        raise SystemExit(f"[demo] session_reservation FATAL: invalid signature length {len(sig_hex)}")
    return sig_hex


def canonical_json_bytes(payload: dict[str, Any]) -> bytes:
    return json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")


def iso_seconds(value: datetime) -> str:
    return value.replace(microsecond=0).isoformat()


def tuple_fields(price_hash: str, event_time: str | None = None) -> dict[str, Any]:
    fields: dict[str, Any] = {
        "budget_id": BUDGET_ID,
        "fx_rate_version": FX_RATE_VERSION,
        "price_snapshot_hash_hex": price_hash,
        "pricing_version": PRICING_VERSION,
        "tenant_id": TENANT_ID,
        "unit": {"unit_id": USD_UNIT_ID},
        "unit_conversion_version": UNIT_CONVERSION_VERSION,
        "unit_id": USD_UNIT_ID,
        "window_instance_id": WINDOW_INSTANCE_ID,
    }
    if event_time is not None:
        fields["event_time"] = event_time
    return fields


def audit_context(
    *,
    key_id: str,
    session_event_type: str,
    session_reservation_id: str,
    event_outcome: dict[str, Any],
) -> dict[str, Any]:
    decision_sequence, outcome_sequence = next_audit_sequences()
    now = datetime.now(timezone.utc).replace(microsecond=0)
    time_seconds = int(now.timestamp())
    time_nanos = 0
    recorded_at = now.isoformat()
    decision_id = str(uuid.uuid4())

    def build(phase: str, cloud_event_type: str, producer_sequence: int) -> dict[str, Any]:
        data = {
            "event_outcome": event_outcome,
            "phase": phase,
            "session_event_type": session_event_type,
            "session_reservation_id": session_reservation_id,
        }
        data_b64 = base64.b64encode(canonical_json_bytes(data)).decode("ascii")
        audit_event_id = str(uuid.uuid4())
        payload = {
            "datacontenttype": "application/json",
            "data_b64": data_b64,
            "decisionid": decision_id,
            "id": audit_event_id,
            "producer_id": PRODUCER_ID,
            "producer_sequence": producer_sequence,
            "runid": "",
            "schema_bundle_id": "",
            "signing_key_id": key_id,
            "source": "urn:spendguard:ledger:session-reservations",
            "specversion": "1.0",
            "tenantid": TENANT_ID,
            "time_nanos": time_nanos,
            "time_seconds": time_seconds,
            "type": cloud_event_type,
        }
        return {
            "audit_outbox_id": str(uuid.uuid4()),
            "audit_event_id": audit_event_id,
            "data_b64": data_b64,
            "producer_sequence": producer_sequence,
            "signature_hex": sign_canonical(canonical_json_bytes(payload)),
        }

    return {
        "decision": build("decision", "spendguard.audit.decision", decision_sequence),
        "decision_id": decision_id,
        "outcome": build("outcome", "spendguard.audit.outcome", outcome_sequence),
        "producer_id": PRODUCER_ID,
        "recorded_at": recorded_at,
        "signing_key_id": key_id,
        "time_nanos": time_nanos,
        "time_seconds": time_seconds,
    }


def require_field(outcome: dict[str, Any], key: str) -> str:
    value = outcome.get(key)
    if not isinstance(value, str) or not value:
        raise SystemExit(f"[demo] session_reservation FATAL: missing {key}: {outcome}")
    return value


def main() -> None:
    key_id = ledger_signing_key_id()
    price_hash = pricing_hash_hex()
    base_time = datetime.now(timezone.utc).replace(microsecond=0)
    session_reservation_id = str(uuid.uuid4())
    ttl_expires_at = iso_seconds(base_time + timedelta(minutes=10))

    reserve_outcome_for_audit = {
        "committed_amount_atomic": "0",
        "remaining_amount_atomic": "100000",
        "released_amount_atomic": "0",
        "reserved_amount_atomic": "100000",
        "session_reservation_id": session_reservation_id,
        "status": "accepted",
        "ttl_expires_at": ttl_expires_at,
    } | tuple_fields(price_hash)
    reserve_req = {
        "audit_context": audit_context(
            key_id=key_id,
            session_event_type="spendguard.audit.session.reserve",
            session_reservation_id=session_reservation_id,
            event_outcome=reserve_outcome_for_audit,
        ),
        "budget_id": BUDGET_ID,
        "estimated_amount_atomic": "100000",
        "fx_rate_version": FX_RATE_VERSION,
        "idempotency_key": "d41-session-reserve",
        "price_snapshot_hash_hex": price_hash,
        "pricing_version": PRICING_VERSION,
        "route": ROUTE,
        "server_mint": {
            "session_reservation_id": session_reservation_id,
            "ttl_expires_at": ttl_expires_at,
        },
        "session_id": SESSION_ID,
        "tenant_id": TENANT_ID,
        "ttl_seconds": 600,
        "unit_conversion_version": UNIT_CONVERSION_VERSION,
        "unit_id": USD_UNIT_ID,
        "window_instance_id": WINDOW_INSTANCE_ID,
    }
    _, reserve_outcome, _ = call_json_function("post_session_reserve", reserve_req)
    assert reserve_outcome is not None
    if require_field(reserve_outcome, "session_reservation_id") != session_reservation_id:
        raise SystemExit("[demo] session_reservation FATAL: reserve session id drift")

    def commit_request(
        streaming_commit_id: str,
        amount: str,
        committed_after: str,
        remaining_after: str,
        *,
        event_time: str,
        event_outcome: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        outcome = event_outcome or {
            "amount_atomic_delta": amount,
            "committed_amount_atomic": committed_after,
            "remaining_amount_atomic": remaining_after,
            "session_reservation_id": session_reservation_id,
            "status": "accepted",
            "streaming_commit_id": streaming_commit_id,
        } | tuple_fields(price_hash, event_time)
        return {
            "amount_atomic_delta": amount,
            "audit_context": audit_context(
                key_id=key_id,
                session_event_type=(
                    "spendguard.audit.session.denied"
                    if outcome.get("status") == "denied"
                    else "spendguard.audit.session.commit_delta"
                ),
                session_reservation_id=session_reservation_id,
                event_outcome=outcome,
            ),
            "budget_id": BUDGET_ID,
            "event_time": event_time,
            "fx_rate_version": FX_RATE_VERSION,
            "idempotency_key": f"{streaming_commit_id}-idem",
            "outcome": "estimated",
            "price_snapshot_hash_hex": price_hash,
            "pricing_version": PRICING_VERSION,
            "session_reservation_id": session_reservation_id,
            "streaming_commit_id": streaming_commit_id,
            "unit_conversion_version": UNIT_CONVERSION_VERSION,
            "unit_id": USD_UNIT_ID,
            "tenant_id": TENANT_ID,
            "window_instance_id": WINDOW_INSTANCE_ID,
        }

    commit1_time = iso_seconds(base_time + timedelta(seconds=1))
    commit1 = commit_request(
        "d41-delta-000001",
        "1000",
        "1000",
        "99000",
        event_time=commit1_time,
    )
    _, commit1_outcome, _ = call_json_function("post_session_commit_delta", commit1)
    assert commit1_outcome is not None
    if require_field(commit1_outcome, "committed_amount_atomic") != "1000":
        raise SystemExit("[demo] session_reservation FATAL: commit1 amount drift")

    commit2_time = iso_seconds(base_time + timedelta(seconds=2))
    commit2 = commit_request(
        "d41-delta-000002",
        "2000",
        "3000",
        "97000",
        event_time=commit2_time,
    )
    _, commit2_outcome, _ = call_json_function("post_session_commit_delta", commit2)
    assert commit2_outcome is not None
    if require_field(commit2_outcome, "remaining_amount_atomic") != "97000":
        raise SystemExit("[demo] session_reservation FATAL: commit2 remaining drift")

    _, replay2_outcome, _ = call_json_function("post_session_commit_delta", commit2)
    if replay2_outcome != commit2_outcome:
        raise SystemExit("[demo] session_reservation FATAL: idempotent replay changed outcome")

    conflict = dict(commit2)
    conflict["amount_atomic_delta"] = "3000"
    conflict.pop("audit_context", None)
    rc, _, conflict_err = call_json_function("post_session_commit_delta", conflict, check=False)
    if rc == 0 or "40P03" not in conflict_err:
        raise SystemExit("[demo] session_reservation FATAL: conflicting replay did not fail with 40P03")

    overrun_time = iso_seconds(base_time + timedelta(seconds=3))
    denied_outcome_for_audit = {
        "attempted_amount_atomic_delta": "200000",
        "committed_amount_atomic": "3000",
        "reason": "OVERRUN_RESERVATION",
        "remaining_amount_atomic": "97000",
        "session_reservation_id": session_reservation_id,
        "status": "denied",
    } | tuple_fields(price_hash, overrun_time)
    overrun = commit_request(
        "d41-delta-overrun",
        "200000",
        "3000",
        "97000",
        event_time=overrun_time,
        event_outcome=denied_outcome_for_audit,
    )
    _, overrun_outcome, _ = call_json_function("post_session_commit_delta", overrun)
    assert overrun_outcome is not None
    if require_field(overrun_outcome, "reason") != "OVERRUN_RESERVATION":
        raise SystemExit("[demo] session_reservation FATAL: overrun denial drift")

    release_time = iso_seconds(base_time + timedelta(seconds=4))
    release_outcome_for_audit = {
        "committed_amount_atomic": "3000",
        "reason_code": "session_completed",
        "released_amount_atomic": "97000",
        "released_this_call_atomic": "97000",
        "remaining_amount_atomic": "0",
        "session_reservation_id": session_reservation_id,
        "session_status": "released",
        "status": "accepted",
    } | tuple_fields(price_hash, release_time)
    release_req = {
        "audit_context": audit_context(
            key_id=key_id,
            session_event_type="spendguard.audit.session.release",
            session_reservation_id=session_reservation_id,
            event_outcome=release_outcome_for_audit,
        ),
        "event_time": release_time,
        "idempotency_key": "d41-session-release",
        "reason_code": "session_completed",
        "session_reservation_id": session_reservation_id,
    }
    _, release_outcome, _ = call_json_function("post_session_release", release_req)
    assert release_outcome is not None
    if require_field(release_outcome, "remaining_amount_atomic") != "0":
        raise SystemExit("[demo] session_reservation FATAL: release remaining drift")

    denied_session_reservation_id = str(uuid.uuid4())
    denied_ttl = iso_seconds(base_time + timedelta(minutes=10))
    denied_route = f"{ROUTE}/denied"
    denied_reserve_audit_outcome = {
        "committed_amount_atomic": "0",
        "reason": "INSUFFICIENT_AVAILABLE_BUDGET",
        "released_amount_atomic": "0",
        "remaining_amount_atomic": "0",
        "requested_amount_atomic": "999999",
        "route": denied_route,
        "session_id": SESSION_ID_DENIED,
        "session_reservation_id": denied_session_reservation_id,
        "status": "denied",
        "ttl_expires_at": denied_ttl,
    } | tuple_fields(price_hash)
    denied_reserve_req = {
        "audit_context": audit_context(
            key_id=key_id,
            session_event_type="spendguard.audit.session.denied",
            session_reservation_id=denied_session_reservation_id,
            event_outcome=denied_reserve_audit_outcome,
        ),
        "budget_id": BUDGET_ID,
        "estimated_amount_atomic": "999999",
        "fx_rate_version": FX_RATE_VERSION,
        "idempotency_key": "d41-session-denied-reserve",
        "price_snapshot_hash_hex": price_hash,
        "pricing_version": PRICING_VERSION,
        "route": denied_route,
        "server_mint": {
            "session_reservation_id": denied_session_reservation_id,
            "ttl_expires_at": denied_ttl,
        },
        "session_id": SESSION_ID_DENIED,
        "tenant_id": TENANT_ID,
        "ttl_seconds": 600,
        "unit_conversion_version": UNIT_CONVERSION_VERSION,
        "unit_id": USD_UNIT_ID,
        "window_instance_id": WINDOW_INSTANCE_ID,
    }
    _, denied_reserve_outcome, _ = call_json_function("post_session_reserve", denied_reserve_req)
    assert denied_reserve_outcome is not None
    if require_field(denied_reserve_outcome, "reason") != "INSUFFICIENT_AVAILABLE_BUDGET":
        raise SystemExit("[demo] session_reservation FATAL: reserve denial reason drift")
    _, denied_reserve_replay, _ = call_json_function("post_session_reserve", denied_reserve_req)
    if denied_reserve_replay != denied_reserve_outcome:
        raise SystemExit("[demo] session_reservation FATAL: reserve denial replay changed outcome")
    denied_reserve_conflict = dict(denied_reserve_req)
    denied_reserve_conflict["estimated_amount_atomic"] = "999998"
    denied_reserve_conflict.pop("audit_context", None)
    rc, _, denied_reserve_conflict_err = call_json_function(
        "post_session_reserve",
        denied_reserve_conflict,
        check=False,
    )
    if rc == 0 or "40P03" not in denied_reserve_conflict_err:
        raise SystemExit("[demo] session_reservation FATAL: reserve denial conflict did not fail with 40P03")

    expired_session_reservation_id = str(uuid.uuid4())
    expired_ttl = iso_seconds(base_time - timedelta(seconds=30))
    expired_reserve_outcome_for_audit = {
        "committed_amount_atomic": "0",
        "remaining_amount_atomic": "5000",
        "released_amount_atomic": "0",
        "reserved_amount_atomic": "5000",
        "session_reservation_id": expired_session_reservation_id,
        "status": "accepted",
        "ttl_expires_at": expired_ttl,
    } | tuple_fields(price_hash)
    expired_reserve_req = {
        "audit_context": audit_context(
            key_id=key_id,
            session_event_type="spendguard.audit.session.reserve",
            session_reservation_id=expired_session_reservation_id,
            event_outcome=expired_reserve_outcome_for_audit,
        ),
        "budget_id": BUDGET_ID,
        "estimated_amount_atomic": "5000",
        "fx_rate_version": FX_RATE_VERSION,
        "idempotency_key": "d41-session-expired-reserve",
        "price_snapshot_hash_hex": price_hash,
        "pricing_version": PRICING_VERSION,
        "route": f"{ROUTE}/expired",
        "server_mint": {
            "session_reservation_id": expired_session_reservation_id,
            "ttl_expires_at": expired_ttl,
        },
        "session_id": SESSION_ID_EXPIRED,
        "tenant_id": TENANT_ID,
        "ttl_seconds": 1,
        "unit_conversion_version": UNIT_CONVERSION_VERSION,
        "unit_id": USD_UNIT_ID,
        "window_instance_id": WINDOW_INSTANCE_ID,
    }
    _, expired_reserve_outcome, _ = call_json_function("post_session_reserve", expired_reserve_req)
    assert expired_reserve_outcome is not None
    if require_field(expired_reserve_outcome, "session_reservation_id") != expired_session_reservation_id:
        raise SystemExit("[demo] session_reservation FATAL: expired reserve session id drift")

    expire_time = iso_seconds(base_time + timedelta(seconds=5))
    expire_outcome_for_audit = {
        "committed_amount_atomic": "0",
        "released_amount_atomic": "5000",
        "released_this_call_atomic": "5000",
        "remaining_amount_atomic": "0",
        "session_reservation_id": expired_session_reservation_id,
        "session_status": "expired",
        "status": "accepted",
    } | tuple_fields(price_hash, expire_time)
    expire_req = {
        "audit_context": audit_context(
            key_id=key_id,
            session_event_type="spendguard.audit.session.expired",
            session_reservation_id=expired_session_reservation_id,
            event_outcome=expire_outcome_for_audit,
        ),
        "event_time": expire_time,
        "idempotency_key": "d41-session-expire",
        "session_reservation_id": expired_session_reservation_id,
    }
    _, expire_outcome, _ = call_json_function("post_session_expire", expire_req)
    assert expire_outcome is not None
    if require_field(expire_outcome, "session_status") != "expired":
        raise SystemExit("[demo] session_reservation FATAL: expire status drift")

    print("[demo] session_reservation ALL 7 steps PASS")


if __name__ == "__main__":
    main()
