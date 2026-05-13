"""AgentBudget runner — drop-in budget enforcement for OpenAI calls.

Pattern from https://github.com/sahiljagtap08/agentbudget README:
    import agentbudget
    agentbudget.init("$10.00")
    # Use openai client as normal; BudgetExhausted raised when cap hit.

We point the OpenAI client at the mock LLM (OPENAI_BASE_URL env var)
and loop until the library aborts us. After exit, we dump:
  - the runner's self-reported spent / remaining
  - the wall-clock time
  - any exception class observed
to /results/agentbudget.json.

The mock LLM is the source of truth for actual $ spent — this runner's
self-report is what the LIBRARY thinks happened.
"""

from __future__ import annotations

import json
import os
import time
import traceback
from pathlib import Path

import agentbudget
import httpx
import openai

BUDGET_USD = float(os.environ.get("BUDGET_USD", "10.00"))
MAX_CALLS = int(os.environ.get("MAX_CALLS", "100"))
BASE_URL = os.environ.get("OPENAI_BASE_URL", "http://mock-llm:8080/v1")
RESULT_PATH = Path(os.environ.get("RESULT_PATH", "/results/agentbudget.json"))
RUNNER_ID = "agentbudget"


def main() -> None:
    RESULT_PATH.parent.mkdir(parents=True, exist_ok=True)

    # max_repeated_calls=100000 effectively disables loop detection so
    # we measure budget enforcement, not the heuristic that fires on
    # repeated identical calls. (loop detection is its own dimension we
    # report qualitatively; this scenario is about budget overshoot.)
    agentbudget.init(
        f"${BUDGET_USD:.2f}",
        max_repeated_calls=100000,
        loop_window_seconds=3600.0,
    )

    # Disable retries so a runner attempt maps 1:1 to a wire call —
    # otherwise the SDK's auto-retry would inflate the mock LLM's
    # ground-truth call count without showing up in calls_attempted.
    client = openai.OpenAI(
        base_url=BASE_URL,
        api_key="sk-mock",
        max_retries=0,
        http_client=httpx.Client(headers={"X-Runner": RUNNER_ID}),
    )

    calls_attempted = 0
    calls_succeeded = 0
    abort_reason: str | None = None
    abort_at_call: int | None = None
    abort_exception_class: str | None = None
    started = time.monotonic()

    for i in range(MAX_CALLS):
        calls_attempted += 1
        try:
            client.chat.completions.create(
                model="gpt-4o",
                messages=[{"role": "user", "content": f"call {i}"}],
            )
            calls_succeeded += 1
        except Exception as exc:
            abort_at_call = i + 1
            abort_exception_class = type(exc).__name__
            abort_reason = str(exc)
            break

    elapsed = time.monotonic() - started

    try:
        spent_self = agentbudget.spent()
    except Exception as exc:
        spent_self = f"<error: {exc}>"
    try:
        remaining_self = agentbudget.remaining()
    except Exception as exc:
        remaining_self = f"<error: {exc}>"

    try:
        agentbudget.teardown()
    except Exception:
        pass

    record = {
        "runner": RUNNER_ID,
        "budget_usd": BUDGET_USD,
        "max_calls": MAX_CALLS,
        "calls_attempted": calls_attempted,
        "calls_succeeded": calls_succeeded,
        "abort_at_call": abort_at_call,
        "abort_exception_class": abort_exception_class,
        "abort_reason": abort_reason,
        "self_reported_spent": str(spent_self),
        "self_reported_remaining": str(remaining_self),
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
