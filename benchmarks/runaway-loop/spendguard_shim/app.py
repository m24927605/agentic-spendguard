"""SpendGuard reservation-gateway shim.

This is a *minimal* HTTP service that exposes the structural piece of
the production SpendGuard sidecar that the benchmark cares about:
**pre-call dollar reservation against a ledger**.

API
---

POST /reserve  { "amount_usd": 0.18 }
  - If `current_spent + pending_reservations + amount_usd <= budget`:
      → 200 { "reservation_id": "<uuid>", "remaining": <float> }
  - Else:
      → 402 Payment Required { "reason": "would exceed budget", ... }

POST /commit   { "reservation_id": "<uuid>", "actual_usd": 0.18 }
  - Releases the reservation, records the actual spend.

POST /release  { "reservation_id": "<uuid>" }
  - Releases the reservation without recording spend (e.g. on error).

GET  /state
  - { "budget_usd": 1.0, "spent": 0.18, "reserved": 0.0, "remaining": 0.82 }

What this *isn't*
-----------------

This shim deliberately does **not** include several things the
production SpendGuard sidecar does:
  - KMS-signed append-only audit chain (immutable evidence)
  - Contract DSL evaluation (declarative budget rules)
  - Multi-tenant scoping (one budget per tenant)
  - L0–L3 capability levels (handshake + enforcement strength)
  - Approval workflow (pause → operator resolve → resume)
  - mTLS between sidecar and ledger
  - Atomic outbox / publish_effect for downstream replay
  - Pricing-freeze with signed snapshot hash

Those are separate dimensions documented qualitatively in the
benchmark write-up. The point of this shim is to isolate the
**reservation-vs-post-call** dimension so the comparison against
agentbudget / agent-guard is apples-to-apples on that one axis.

The full sidecar is exercised by `deploy/demo/compose.yaml`
(`make demo-up` from the repo root). A future iteration of this
benchmark will swap this shim for a runner that talks to the real
sidecar over UDS.
"""

from __future__ import annotations

import json
import os
import threading
import time
import uuid
from pathlib import Path

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel

BUDGET_USD = float(os.environ.get("BUDGET_USD", "1.00"))
LOG_PATH = Path(os.environ.get("SHIM_LEDGER_LOG", "/var/log/spendguard_shim.jsonl"))

LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
# Truncate on each startup so back-to-back benchmark runs start clean.
LOG_PATH.write_text("")

_state_lock = threading.Lock()
_spent = 0.0
_reserved: dict[str, float] = {}


def _append(event: dict) -> None:
    event["ts"] = time.time()
    with LOG_PATH.open("a") as f:
        f.write(json.dumps(event) + "\n")


class ReserveRequest(BaseModel):
    amount_usd: float


class CommitRequest(BaseModel):
    reservation_id: str
    actual_usd: float | None = None


class ReleaseRequest(BaseModel):
    reservation_id: str


app = FastAPI()


@app.get("/healthz")
def healthz() -> dict[str, str]:
    return {"status": "ok"}


@app.get("/state")
def state() -> dict[str, float]:
    with _state_lock:
        reserved = sum(_reserved.values())
        return {
            "budget_usd": BUDGET_USD,
            "spent": round(_spent, 6),
            "reserved": round(reserved, 6),
            "remaining": round(BUDGET_USD - _spent - reserved, 6),
        }


@app.post("/reserve")
def reserve(req: ReserveRequest) -> dict:
    with _state_lock:
        reserved_total = sum(_reserved.values())
        if _spent + reserved_total + req.amount_usd > BUDGET_USD:
            _append(
                {
                    "kind": "reserve_denied",
                    "amount_usd": req.amount_usd,
                    "spent": _spent,
                    "reserved": reserved_total,
                    "budget": BUDGET_USD,
                }
            )
            raise HTTPException(
                status_code=402,
                detail={
                    "reason": "would_exceed_budget",
                    "amount_usd": req.amount_usd,
                    "spent": _spent,
                    "reserved": reserved_total,
                    "budget_usd": BUDGET_USD,
                    "remaining_usd": BUDGET_USD - _spent - reserved_total,
                },
            )
        rid = str(uuid.uuid4())
        _reserved[rid] = req.amount_usd
        _append(
            {
                "kind": "reserve",
                "reservation_id": rid,
                "amount_usd": req.amount_usd,
            }
        )
        return {
            "reservation_id": rid,
            "amount_usd": req.amount_usd,
            "remaining_usd": BUDGET_USD - _spent - sum(_reserved.values()),
        }


@app.post("/commit")
def commit(req: CommitRequest) -> dict:
    global _spent
    with _state_lock:
        if req.reservation_id not in _reserved:
            raise HTTPException(404, "reservation not found")
        held = _reserved.pop(req.reservation_id)
        actual = req.actual_usd if req.actual_usd is not None else held
        _spent += actual
        _append(
            {
                "kind": "commit",
                "reservation_id": req.reservation_id,
                "actual_usd": actual,
                "spent_after": _spent,
            }
        )
        return {
            "reservation_id": req.reservation_id,
            "actual_usd": actual,
            "spent_total": _spent,
        }


@app.post("/release")
def release(req: ReleaseRequest) -> dict:
    with _state_lock:
        if req.reservation_id not in _reserved:
            raise HTTPException(404, "reservation not found")
        amount = _reserved.pop(req.reservation_id)
        _append(
            {
                "kind": "release",
                "reservation_id": req.reservation_id,
                "amount_usd": amount,
            }
        )
        return {"reservation_id": req.reservation_id, "released_usd": amount}
