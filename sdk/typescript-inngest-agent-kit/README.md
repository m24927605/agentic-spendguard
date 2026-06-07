# @spendguard/inngest-agent-kit

SpendGuard adapter for [Inngest AgentKit](https://github.com/inngest/agent-kit) — pre-call budget enforcement inside every `step.ai` invocation, with deterministic retry-dedup driven by Inngest's own step identity.

> **Status:** D29 SLICE 1+2+3 bundle (`0.1.0-pre`). Demo (SLICE 5) and docs / publish workflow (SLICE 6) land in follow-on slices.

## Quickstart

```ts
import { Inngest } from "inngest";
import { SpendGuardClient } from "@spendguard/sdk";
import { wrapWithSpendGuard } from "@spendguard/inngest-agent-kit";

const client = new SpendGuardClient({ /* ...UDS config... */ });
await client.connect();
await client.handshake();

const inngest = new Inngest({ id: "my-app" });

inngest.createFunction({ id: "agent-fn" }, { event: "agent/run" },
  async ({ step }) => {
    const sgStep = wrapWithSpendGuard(step.ai, client, {
      tenantId: "tenant-prod",
      budgetId: BUDGET_ID,
      claimEstimator: ({ stepId, model, body }) => [{
        scopeId: BUDGET_ID,
        amountAtomic: "1000000",
        unit: { unit: "USD_MICROS", denomination: 1 },
      }],
    });
    return await sgStep.infer("call-openai", {
      model: openai({ model: "gpt-4o-mini" }),
      body: { messages: [{ role: "user", content: "hi" }] },
    });
  });
```

## Retry dedup

Inngest retries the same step body with the **same `step.id`** (and the same `step.idempotencyKey` when one is supplied). The adapter feeds that identity into SpendGuard's `idempotencyKey` derivation, so:

- Attempt 0 fires `reserve` against the sidecar.
- Attempt 1+ (on retry) re-derives the **same** key. With `opts.idempotencyCache` set, the cached `DecisionOutcome` short-circuits the sidecar round-trip; without it, the sidecar's own dedup layer absorbs the duplicate.
- The audit journal records **one** `LLM_CALL_PRE` row across N attempts.

`attempt` is intentionally NOT part of the seed (review-standards §6.5).

## Errors

| Error | Cause | Behaviour |
|---|---|---|
| `DecisionDenied` | budget exhausted at PRE | thrown out of step body; step fails before provider call |
| `DecisionStopped` | hard-stop policy at PRE | thrown out of step body |
| `DecisionSkipped` | skip policy at PRE | thrown out of step body |
| `ApprovalRequired` | gate requires approval at PRE | thrown unless `onApprovalRequired` resumes |
| `SidecarUnavailable` | sidecar unreachable | thrown out of step body (strict mode) |

Provider-side errors thrown from `step.ai.infer`'s body produce a `commitEstimated(outcome="PROVIDER_ERROR")` then re-throw, so the audit journal carries one PRE + one PROVIDER_ERROR-tagged POST for the failed attempt.

## License

Apache-2.0.
