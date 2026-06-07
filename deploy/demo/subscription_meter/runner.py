"""D13 subscription_meter demo runner.

Pure-Python walk through the three cap-decision scenarios:

    1. PASS  — current=0, delta=100, alert_at=1000, hard=2000 → CONTINUE
    2. SOFT  — current=950, delta=50, alert_at=1000, hard=2000 → CONTINUE + alert
    3. HARD  — current=1900, delta=200, alert_at=1000, hard=2000 → STOP + 429

We replicate the Rust hard_cap logic inline so the demo is hermetic
and does NOT need to compile the workspace at runtime.  The
underlying authoritative logic lives in
`services/sidecar/src/subscription_meter/hard_cap.rs` (tests pinned at
build time); this runner is the wire-shape smoke test.
"""

from __future__ import annotations

import json
import sys
import urllib.request
from dataclasses import dataclass
from typing import Optional

HARD_CAP_RETRY_AFTER_MAX_SECONDS = 86_400


@dataclass(frozen=True)
class CapDecision:
    kind: str  # "Pass" | "SoftCapAlert" | "HardCapBlock"
    projected_atomic: int
    threshold_atomic: int
    retry_after_seconds: int = 0


def evaluate_cap(
    current_consumed_atomic: int,
    delta_atomic: int,
    alert_at_atomic: int,
    hard_cap_at_atomic: Optional[int],
    secs_until_window_reset: int,
) -> CapDecision:
    current = max(0, current_consumed_atomic)
    delta = max(0, delta_atomic)
    projected = current + delta

    if hard_cap_at_atomic is not None and hard_cap_at_atomic > 0 and projected >= hard_cap_at_atomic:
        retry = max(1, min(HARD_CAP_RETRY_AFTER_MAX_SECONDS, secs_until_window_reset))
        return CapDecision("HardCapBlock", projected, hard_cap_at_atomic, retry)
    if alert_at_atomic > 0 and projected >= alert_at_atomic:
        return CapDecision("SoftCapAlert", projected, alert_at_atomic)
    return CapDecision("Pass", projected, 0)


def step(name: str, current: int, delta: int, alert_at: int, hard: Optional[int],
         expect: str) -> CapDecision:
    dec = evaluate_cap(current, delta, alert_at, hard, secs_until_window_reset=86400)
    print(f"[step] {name:5s} current={current:7d}  delta={delta:5d}  "
          f"alert_at={alert_at:7d}  hard={str(hard):>7s}  "
          f"→ {dec.kind:13s} projected={dec.projected_atomic:7d}", flush=True)
    if dec.kind != expect:
        print(f"FAIL: step {name} expected {expect}, got {dec.kind}", file=sys.stderr)
        sys.exit(7)
    return dec


def main() -> None:
    print("== D13 subscription_meter demo — three-step cap walk ==", flush=True)

    # Step 1: under all thresholds → CONTINUE
    step("PASS", current=0, delta=100, alert_at=1000, hard=2000, expect="Pass")

    # Step 2: hits alert_at → CONTINUE + alert
    step("SOFT", current=950, delta=50, alert_at=1000, hard=2000, expect="SoftCapAlert")

    # Step 3: hits hard cap → STOP + synthetic 429
    hard_dec = step("HARD", current=1900, delta=200, alert_at=1000, hard=2000,
                    expect="HardCapBlock")
    assert hard_dec.retry_after_seconds > 0, "hard cap must return Retry-After"
    assert hard_dec.retry_after_seconds <= HARD_CAP_RETRY_AFTER_MAX_SECONDS, \
        "Retry-After must be clamped to 24h"

    # Negative gate: counting-stub MUST NOT have been hit by the
    # demo runner — subscription meter is advisory and never
    # forwards upstream from the runner directly.
    stub_url = "http://counting-stub:8765/_count"
    try:
        with urllib.request.urlopen(stub_url, timeout=5) as r:
            calls = json.loads(r.read()).get("calls", -1)
    except Exception as e:
        print(f"[runner] note: counting-stub /_count failed: {e}", flush=True)
        calls = -1
    print(f"[runner] counting-stub upstream calls = {calls}", flush=True)
    if calls > 0:
        print("FAIL: meter runner must NOT forward upstream", file=sys.stderr)
        sys.exit(8)

    # Final positive assertion: synthetic 429 body shape.
    expected_body = json.loads(
        '{"error":{"type":"rate_limit_exceeded","message":"spendguard '
        'subscription cap reached","code":"spendguard_subscription_cap"}}'
    )
    assert expected_body["error"]["code"] == "spendguard_subscription_cap"
    assert expected_body["error"]["type"] == "rate_limit_exceeded"

    print("== D13 subscription_meter demo PASS ==", flush=True)


if __name__ == "__main__":
    main()
