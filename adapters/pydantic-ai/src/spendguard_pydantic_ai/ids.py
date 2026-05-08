"""ID and idempotency-key helpers.

Two flavors of identity are minted here:

  - Time-ordered IDs (`new_uuid7`) for one-shot operations whose
    identity does not need to survive retries — e.g., Handshake's
    workload_instance_id, RunContext.run_id when the caller hasn't
    set one.

  - **Content-derived** IDs for everything inside a Pydantic-AI
    `Model.request()` call. Pydantic-AI's Agent run loop will call
    `request()` again on transient provider failure (or when the
    framework's retry policy fires). The wrapper has no signal
    distinguishing "fresh call" from "retry of the same step" other
    than the inputs themselves. So we derive `step_id` / `llm_call_id`
    / decision-trace-id / idempotency_key from a stable hash of the
    messages + model_settings + run_id. Same logical call → same hash
    → same idempotency_key → sidecar cache hit + ledger UNIQUE
    collapse onto the first decision.

References: Trace Schema §3.4 (idempotency_key fallback);
Sidecar Architecture §6 (idempotency cache).
"""

from __future__ import annotations

import hashlib
import os
import secrets
import time
import uuid
from collections.abc import Sequence
from typing import Any


def new_uuid7() -> uuid.UUID:
    """Mint a UUIDv7 (RFC 9562 §5.7).

    Layout (128 bits, big-endian):
      - 48 bits unix epoch ms
      - 4 bits version (0b0111)
      - 12 bits random
      - 2 bits variant (0b10)
      - 62 bits random
    """
    ts_ms = int(time.time() * 1000) & ((1 << 48) - 1)
    rand_a = secrets.randbits(12)
    rand_b = secrets.randbits(62)

    # Compose:
    #   bits 127..80  unix_ts_ms (48)
    #   bits 79..76   version 0x7 (4)
    #   bits 75..64   rand_a (12)
    #   bits 63..62   variant 0b10 (2)
    #   bits 61..0    rand_b (62)
    value = (
        (ts_ms << 80)
        | (0x7 << 76)
        | (rand_a << 64)
        | (0b10 << 62)
        | rand_b
    )
    return uuid.UUID(int=value)


def derive_idempotency_key(
    *,
    tenant_id: str,
    session_id: str,
    run_id: str,
    step_id: str,
    llm_call_id: str,
    trigger: str,
) -> str:
    """Deterministic idempotency key for a trigger boundary.

    Same (tenant, session, run, step, llm_call, trigger) → same key.
    A retry of the SAME logical step within Pydantic-AI's run loop
    (e.g., transient HTTP failure) MUST reuse this so the sidecar's
    cache short-circuits + the ledger's UNIQUE returns Replay.

    Returns a hex-encoded 32-char (128-bit) string — short enough to
    fit comfortably in log lines, wide enough for collision resistance.
    """
    canonical = "\x1f".join(
        [
            "v1",
            tenant_id,
            session_id,
            run_id,
            step_id,
            llm_call_id,
            trigger,
        ]
    )
    digest = hashlib.blake2b(canonical.encode("utf-8"), digest_size=16).hexdigest()
    return f"sg-{digest}"


CallSignatureFn = "Callable[[Sequence[Any], Any], str]"
"""Type alias for a custom call-signature function (see SpendGuardModel)."""


def default_call_signature(
    messages: Sequence[Any],
    model_settings: Any | None,
) -> str:
    """Stable hash over the *content* of a Pydantic-AI Model.request() call.

    The output is a 32-char hex digest. Two `Model.request()`
    invocations with identical messages and model_settings produce the
    same digest — which is what makes idempotency survive Pydantic-AI's
    framework-level retry, where the wrapper sees `request()` re-entered
    with bit-identical inputs.

    Serialization strategy:
      - Pydantic-AI message types are pydantic v2 models; we use
        `.model_dump_json(exclude_none=True)` for canonical form when
        available.
      - `model_settings` is typically a TypedDict; we sort-key
        json.dumps it.
      - Anything else falls back to `repr()` — brittle but deterministic
        within a single Python session.

    Callers that need stronger guarantees (cross-version stability,
    cross-runtime portability) should pass a custom `call_signature_fn`
    to `SpendGuardModel`.
    """
    import json

    h = hashlib.blake2b(digest_size=16)
    h.update(b"v1:call:")
    for i, msg in enumerate(messages):
        h.update(f"|msg{i}|".encode("utf-8"))
        if hasattr(msg, "model_dump_json"):
            try:
                h.update(msg.model_dump_json(exclude_none=True).encode("utf-8"))
                continue
            except Exception:  # noqa: BLE001 — fall through to repr
                pass
        h.update(repr(msg).encode("utf-8"))
    h.update(b"|settings|")
    if model_settings is None:
        h.update(b"none")
    elif hasattr(model_settings, "model_dump_json"):
        try:
            h.update(
                model_settings.model_dump_json(exclude_none=True).encode("utf-8")
            )
        except Exception:  # noqa: BLE001
            h.update(repr(model_settings).encode("utf-8"))
    elif isinstance(model_settings, dict):
        h.update(
            json.dumps(model_settings, sort_keys=True, default=str).encode("utf-8")
        )
    else:
        h.update(repr(model_settings).encode("utf-8"))
    return h.hexdigest()


def derive_uuid_from_signature(signature: str, *, scope: str) -> uuid.UUID:
    """Derive a stable UUID (v4-shaped) from a content signature + scope.

    `scope` ("decision_id", "llm_call_id", etc.) namespaces the UUID
    so different identifier slots never collide for the same call.
    """
    digest = hashlib.blake2b(
        f"{scope}|{signature}".encode("utf-8"), digest_size=16
    ).digest()
    buf = bytearray(digest)
    buf[6] = (buf[6] & 0x0F) | 0x40  # version 4
    buf[8] = (buf[8] & 0x3F) | 0x80  # variant 10
    return uuid.UUID(bytes=bytes(buf))


def workload_instance_id() -> str:
    """Sidecar workload identity hint.

    The adapter ASSERTS this in handshake; the sidecar verifies against
    SO_PEERCRED + signed manifest (per Sidecar §5). For POC we read it
    from the SPENDGUARD_WORKLOAD_INSTANCE_ID env var; production
    deployments inject this via the platform.
    """
    return os.environ.get("SPENDGUARD_WORKLOAD_INSTANCE_ID", "")
