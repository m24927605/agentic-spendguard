"""SpendGuard runner — pre-call reservation, then call, then commit.

Pattern:
    POST /reserve $0.18  →  reservation_id  (or 402 → break)
    POST /v1/chat/completions to mock LLM
    POST /commit reservation_id

This is the structural pattern the production SpendGuard sidecar uses,
exercised here against a minimal reservation-gateway shim
(`spendguard_shim/`). The shim implements *only* the reservation-vs-
post-call dimension; everything else SpendGuard provides (KMS-signed
audit chain, contract DSL, multi-tenant, approval workflow, L0–L3
capability levels) is documented qualitatively in the benchmark
write-up and exercised separately by `make demo-up` at the repo root.
"""

from __future__ import annotations

import json
import os
import time
import traceback
from pathlib import Path

import httpx
import openai

BUDGET_USD = float(os.environ.get("BUDGET_USD", "1.00"))
MAX_CALLS = int(os.environ.get("MAX_CALLS", "100"))
RESERVATION_USD = float(os.environ.get("RESERVATION_USD", "0.18"))
BASE_URL = os.environ.get("OPENAI_BASE_URL", "http://mock-llm:8080/v1")
SHIM_URL = os.environ.get("SPENDGUARD_SHIM_URL", "http://spendguard-shim:8090")
RESULT_PATH = Path(os.environ.get("RESULT_PATH", "/results/spendguard.json"))
RUNNER_ID = "spendguard"


def main() -> None:
    RESULT_PATH.parent.mkdir(parents=True, exist_ok=True)

    client = openai.OpenAI(
        base_url=BASE_URL,
        api_key="sk-mock",
        max_retries=0,
        http_client=httpx.Client(headers={"X-Runner": RUNNER_ID}),
    )
    shim = httpx.Client(base_url=SHIM_URL, timeout=5.0)

    calls_attempted = 0
    calls_succeeded = 0
    abort_reason: str | None = None
    abort_at_call: int | None = None
    abort_exception_class: str | None = None
    started = time.monotonic()

    for i in range(MAX_CALLS):
        calls_attempted += 1
        # 1. Reserve.
        r = shim.post("/reserve", json={"amount_usd": RESERVATION_USD})
        if r.status_code == 402:
            abort_at_call = i + 1
            abort_exception_class = "ReservationDenied"
            abort_reason = json.dumps(r.json())
            break
        r.raise_for_status()
        reservation_id = r.json()["reservation_id"]

        # 2. Make the upstream call.
        try:
            client.chat.completions.create(
                model="gpt-4o",
                messages=[{"role": "user", "content": f"call {i}"}],
            )
            calls_succeeded += 1
        except Exception as exc:
            release = shim.post(
                "/release", json={"reservation_id": reservation_id}
            )
            release.raise_for_status()
            abort_at_call = i + 1
            abort_exception_class = type(exc).__name__
            abort_reason = str(exc)
            break

        # 3. Commit. Fail loudly if the shim refuses — silent commit
        # failure would let the runner over-spend.
        commit = shim.post(
            "/commit",
            json={"reservation_id": reservation_id, "actual_usd": RESERVATION_USD},
        )
        commit.raise_for_status()

    elapsed = time.monotonic() - started

    state = shim.get("/state").json()

    record = {
        "runner": RUNNER_ID,
        "budget_usd": BUDGET_USD,
        "max_calls": MAX_CALLS,
        "calls_attempted": calls_attempted,
        "calls_succeeded": calls_succeeded,
        "abort_at_call": abort_at_call,
        "abort_exception_class": abort_exception_class,
        "abort_reason": abort_reason,
        "self_reported_spent": state["spent"],
        "self_reported_remaining": state["remaining"],
        "elapsed_seconds": round(elapsed, 3),
    }

    with RESULT_PATH.open("w") as f:
        json.dump(record, f, indent=2)
    print(json.dumps(record, indent=2))


if __name__ == "__main__":
    try:
        main()
    except Exception:
        traceback.print_exc()
        raise
