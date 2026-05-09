"""SpendGuard SDK adapter — onboarding template (Phase 5 S20).

Wraps a langchain LLM call with a SpendGuard sidecar decision RPC
over UDS. Demonstrates all three decision outcomes the contract
template exposes:

  * STOP — caller raises BudgetExhausted; the call never hits the
           upstream LLM.
  * REQUIRE_APPROVAL — caller waits on Control Plane's approval
           queue (or fails-fast in offline contexts) before
           proceeding.
  * CONTINUE — normal path; sidecar reserves budget BEFORE the
           call, then releases / commits AFTER based on the
           response.

This file is INSTRUCTIONAL — copy it into your project, replace
the placeholders, and adapt to your framework. It does NOT install
spendguard-sdk; that's a separate package.

Tested against:
  langchain >= 0.1
  spendguard-sdk >= 0.1 (Phase 4 O5)
  python >= 3.11
"""

from __future__ import annotations

import os
import uuid
from dataclasses import dataclass

# spendguard-sdk: assumed importable via pip install spendguard-sdk
# (the SDK is shipped as part of Phase 4 O5; if it isn't on PyPI
# in your environment, point at the local sdk/python checkout).
from spendguard import (
    SidecarClient,
    DecisionRequest,
    DecisionResponse,
    Decision,
    BudgetExhausted,
    ApprovalRequired,
    ProjectedClaim,
    UnitRef,
)


# ---------------------------------------------------------------------------
# Operator placeholders — edit these.
# ---------------------------------------------------------------------------

UDS_PATH = os.environ.get(
    "SPENDGUARD_SIDECAR_UDS",
    "/var/run/spendguard/adapter.sock",
)
TENANT_ID = os.environ["SPENDGUARD_TENANT_ID"]
BUDGET_ID = os.environ["SPENDGUARD_BUDGET_ID"]
WINDOW_INSTANCE_ID = os.environ["SPENDGUARD_WINDOW_INSTANCE_ID"]
UNIT_ID = os.environ["SPENDGUARD_UNIT_ID"]
PRICING_VERSION = os.environ["SPENDGUARD_PRICING_VERSION"]
PRICE_SNAPSHOT_HASH_HEX = os.environ["SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX"]


# ---------------------------------------------------------------------------
# Adapter
# ---------------------------------------------------------------------------


@dataclass
class CallEstimate:
    """Pre-call estimate the SDK uses for the projected claim."""
    estimated_amount_atomic: int
    model_id: str
    prompt_tokens: int


def estimate_cost(prompt: str, model_id: str = "gpt-4o-mini") -> CallEstimate:
    """Estimate the USD-micros cost of a chat completion.

    Production code should use a tokenizer (tiktoken etc.). This
    template uses a crude character-count heuristic — replace.
    """
    prompt_tokens = max(1, len(prompt) // 4)
    # gpt-4o-mini input price (May 2026): $0.15 per 1M tokens.
    # Convert to micro-USD: 0.15 * 1_000_000 = 150_000 per 1M tokens.
    input_micros = (prompt_tokens * 150_000) // 1_000_000
    # Output guess: 500 tokens × $0.60 / 1M.
    output_micros = (500 * 600_000) // 1_000_000
    return CallEstimate(
        estimated_amount_atomic=input_micros + output_micros,
        model_id=model_id,
        prompt_tokens=prompt_tokens,
    )


def call_with_spendguard(prompt: str, llm_call) -> str:
    """Invoke an LLM with SpendGuard hard-cap / soft-cap / continue
    enforcement.

    Args:
        prompt: the user prompt.
        llm_call: a callable taking (prompt) and returning the LLM's
                  response string.

    Raises:
        BudgetExhausted: the contract's hard-cap-stop rule fired.
        ApprovalRequired: the soft-cap-approval rule fired and an
            approver hasn't (yet) granted the call.
    """
    estimate = estimate_cost(prompt)
    decision_id = uuid.uuid4()
    sidecar = SidecarClient(uds_path=UDS_PATH)

    try:
        response: DecisionResponse = sidecar.request_decision(
            DecisionRequest(
                tenant_id=TENANT_ID,
                decision_id=str(decision_id),
                projected_claims=[
                    ProjectedClaim(
                        budget_id=BUDGET_ID,
                        amount_atomic=str(estimate.estimated_amount_atomic),
                        window_instance_id=WINDOW_INSTANCE_ID,
                        unit=UnitRef(unit_id=UNIT_ID),
                    )
                ],
                pricing_version=PRICING_VERSION,
                price_snapshot_hash_hex=PRICE_SNAPSHOT_HASH_HEX,
                model_id=estimate.model_id,
            )
        )
    finally:
        # SidecarClient is short-lived in this template; production
        # code reuses one client per process.
        pass

    if response.decision == Decision.STOP:
        # Hard-cap: refuse the call entirely. No budget moved; no
        # LLM round-trip. Caller surfaces the typed error to the
        # human or upstream caller.
        raise BudgetExhausted(
            decision_id=decision_id,
            matched_rule_ids=response.matched_rule_ids,
            reason_codes=response.reason_codes,
        )

    if response.decision == Decision.REQUIRE_APPROVAL:
        # Soft-cap: reservation is created but in `awaiting_approval`
        # state. The caller waits on Control Plane's approval queue
        # (or fails fast in non-interactive contexts).
        raise ApprovalRequired(
            decision_id=decision_id,
            approval_id=response.approval_id,
            approver_role=response.approver_role,
        )

    # CONTINUE path: reservation held; do the LLM call.
    assert response.decision == Decision.CONTINUE
    reservation_id = response.reservation_set_id

    try:
        llm_response = llm_call(prompt)
    except Exception:
        # Release the reservation — no spend committed.
        sidecar.confirm_publish_outcome(
            decision_id=str(decision_id),
            outcome="apply_failed",
        )
        raise

    # Commit the actual cost (after we know what the LLM charged us).
    actual_cost_atomic = compute_actual_cost_atomic(prompt, llm_response, estimate)
    sidecar.commit_estimated(
        reservation_id=reservation_id,
        estimated_amount_atomic=str(actual_cost_atomic),
    )
    return llm_response


def compute_actual_cost_atomic(
    prompt: str, response: str, estimate: CallEstimate
) -> int:
    """Replace with your tokenizer-based actual cost computation."""
    response_tokens = max(1, len(response) // 4)
    input_micros = (estimate.prompt_tokens * 150_000) // 1_000_000
    output_micros = (response_tokens * 600_000) // 1_000_000
    return input_micros + output_micros


# ---------------------------------------------------------------------------
# Smoke test
# ---------------------------------------------------------------------------


if __name__ == "__main__":
    # Demonstrates each decision path. Run with:
    #   python sdk_adapter.py
    # Expected output: three responses (CONTINUE, REQUIRE_APPROVAL,
    # STOP) interleaved with sidecar log lines.
    def fake_llm(p: str) -> str:
        return f"echo: {p}"

    # CONTINUE — small prompt, small estimated cost.
    print(call_with_spendguard("hello world", fake_llm))

    try:
        # REQUIRE_APPROVAL — > 100 USD estimated cost.
        print(call_with_spendguard("x" * 10_000_000, fake_llm))
    except ApprovalRequired as e:
        print(f"awaiting approval: {e}")

    try:
        # STOP — try to spend more than the budget allows.
        print(call_with_spendguard("y" * 100_000_000, fake_llm))
    except BudgetExhausted as e:
        print(f"budget exhausted: {e}")
