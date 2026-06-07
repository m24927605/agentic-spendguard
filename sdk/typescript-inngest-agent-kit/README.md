# @spendguard/inngest-agent-kit

SpendGuard adapter for [Inngest AgentKit](https://github.com/inngest/agent-kit) — pre-call budget enforcement inside every `step.ai` invocation, with deterministic retry-dedup driven by Inngest's own step identity.

> **Status:** D29 v0.1.0 — first GA release. Closes deliverable D29 (Inngest AgentKit adapter). See [`docs/integrations/inngest-agent-kit`](https://github.com/m24927605/agentic-spendguard/blob/main/docs/site-v2/src/content/docs/docs/integrations/inngest-agent-kit.mdx) for the full integration page and [`CHANGELOG.md`](./CHANGELOG.md) for the release notes.

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

## Demo

```bash
# Laptop iteration — no sidecar, no real provider.
cd examples/inngest-agent-kit && node index.mjs --mock

# Full sidecar-backed compose demo.
make demo-up DEMO_MODE=inngest_agent_kit
```

The composite demo drives **ALLOW + DENY + RETRY_DEDUP** through the wrap. Success line (LOCKED):

```
[demo] inngest_agent_kit ALL 3 steps PASS (ALLOW + DENY + RETRY_DEDUP)
```

The headline `RETRY_DEDUP` step asserts 3 attempts of the same step body produce **1 SpendGuard reservation** — verified at the SQL level by `COV_D29_DEDUP_GATE` in [`deploy/demo/verify_step_inngest_agent_kit.sql`](../../deploy/demo/verify_step_inngest_agent_kit.sql).

## Known limitations

- **Stream-per-chunk gating** is anti-scope for v0.1.x — `step.ai.infer` is non-streaming by Inngest design.
- **`UnitRef` substrate gap** — the TS substrate's `mapUnitRef` hardcodes empty `unit_id`, so the `--real` demo flow surfaces `claim[0].unit.unit_id empty` from the sidecar's validator. This is the cross-slice D05 UnitRef gap (D04 / D06 / D08 share the same surface); the adapter's wire shape and retry-dedup contract are independently verified by the 76 vitest cases and the `--mock` demo mode. A future hardening slice picks up the substrate-side broadening.
- See [`CHANGELOG.md`](./CHANGELOG.md) "Known limitations" for the full list.

## License

Apache-2.0. See [`LICENSE_NOTICES.md`](./LICENSE_NOTICES.md) for third-party license attributions.
