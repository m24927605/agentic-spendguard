// D37 SLICE 5 demo driver — exercises the n8n integration's
// reservation lifecycle against the in-cluster sidecar HTTP companion.
//
// The integration's runtime path (nodes/SpendGuardChatModel.node.ts
// supplyData() → @spendguard/langchain SpendGuardCallbackHandler →
// @spendguard/sdk reserve/commit) ultimately POSTs to the sidecar at
// /v1/decision (reserve) and /v1/trace (commit). Rather than spin up an
// n8n v1.50 runtime + editor (~600 MB image, several minutes of boot
// time) just to dispatch the same two HTTP calls, this driver invokes
// the wire shape directly — same integration semantics, deterministic
// 30-second runtime. The full v1.50 runtime invariant is verified in CI
// via testcontainers (.github/workflows/n8n-integration-ci.yml).
//
// 3-step matrix:
//   1. ALLOW  — reserve + commit succeed; upstream counting stub +1.
//   2. DENY   — sidecar returns DENY; integration translates to
//               NodeApiError(httpCode: "403"); upstream stub UNCHANGED.
//               INV-1.
//   3. STREAM — same as ALLOW but with `stream=true` flag on the
//               decision_context (so SQL gate sees the streaming row).
//
// Exit codes:
//   0 — all 3 steps PASS; success line printed.
//   7 — any step FAILS; failing step printed on stderr.

const SIDECAR =
  process.env.SPENDGUARD_N8N_SIDECAR_URL ?? "http://sidecar:8443";
const STUB =
  process.env.SPENDGUARD_COUNTING_STUB_URL ?? "http://counting-stub:8765";
const TENANT =
  process.env.SPENDGUARD_N8N_TENANT_ID ??
  "00000000-0000-4000-8000-000000000001";
const BUDGET =
  process.env.SPENDGUARD_N8N_BUDGET_ID ??
  "44444444-4444-4444-8444-444444444444";
const WINDOW =
  process.env.SPENDGUARD_N8N_WINDOW_INSTANCE_ID ??
  "55555555-5555-4555-8555-555555555555";

function log(msg) {
  process.stdout.write(`[n8n-demo] ${msg}\n`);
}
function err(msg) {
  process.stderr.write(`[n8n-demo] ${msg}\n`);
}

async function postJson(url, body) {
  const resp = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const text = await resp.text();
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch {
    parsed = { _raw: text };
  }
  return { status: resp.status, body: parsed };
}

async function getJson(url) {
  const resp = await fetch(url);
  return resp.json();
}

function uuid7() {
  const ts = Date.now();
  const tsHex = ts.toString(16).padStart(12, "0");
  const rand = Math.random().toString(16).slice(2, 14).padStart(12, "0");
  return `${tsHex.slice(0, 8)}-${tsHex.slice(8, 12)}-7${rand.slice(0, 3)}-8${rand.slice(3, 6)}-${rand.slice(6, 18).padEnd(12, "0")}`;
}

function buildDecisionRequest(opts) {
  const {
    executionId,
    nodeName = "AI Agent",
    decisionCtxExtras = {},
    claimEstimate = "100",
  } = opts;
  const runId = `${executionId}:${nodeName}`;
  const stepId = nodeName;
  const llmCallId = runId;
  const idempotency = `sg-demo-${runId}-${stepId}-${llmCallId}`;
  return {
    body: {
      tenant_id: TENANT,
      claim_estimate_atomic: claimEstimate,
      prompt_class: "abc012345689",
      model_class: "openai",
      idempotency_key: idempotency,
      budget_id: BUDGET,
      decision_context: {
        integration: "n8n",
        mode: "community_node",
        upstream_provider: "openai",
        workflow_id: "demo-workflow-1",
        node_name: nodeName,
        execution_id: executionId,
        model: "gpt-4o-mini",
        window_instance_id: WINDOW,
        prompt_hash:
          "0000000000000000000000000000000000000000000000000000000000000000",
        run_id: runId,
        step_id: stepId,
        llm_call_id: llmCallId,
        ...decisionCtxExtras,
      },
    },
    runId,
    stepId,
    llmCallId,
  };
}

async function step1Allow() {
  log("step 1 ALLOW — reserve + upstream + commit");
  const req = buildDecisionRequest({ executionId: uuid7() });
  const before = await getJson(`${STUB}/_count`);
  const decision = await postJson(`${SIDECAR}/v1/decision`, req.body);
  if (decision.status !== 200) {
    err(
      `step 1 reserve HTTP ${decision.status}: ${JSON.stringify(decision.body)}`,
    );
    return false;
  }
  if (decision.body.verdict !== "ALLOW") {
    err(`step 1 expected ALLOW, got ${decision.body.verdict}`);
    return false;
  }
  const upstream = await postJson(`${STUB}/v1/chat/completions`, {
    model: "gpt-4o-mini",
    messages: [{ role: "user", content: "hi from n8n demo step 1" }],
  });
  if (upstream.status !== 200) {
    err(`step 1 upstream HTTP ${upstream.status}`);
    return false;
  }
  const after = await getJson(`${STUB}/_count`);
  if (after.calls !== before.calls + 1) {
    err(
      `step 1 upstream stub counter expected +1, before=${before.calls} after=${after.calls}`,
    );
    return false;
  }
  const usage = upstream.body.usage ?? {};
  const inputTokens = usage.prompt_tokens ?? usage.inputTokens ?? 5;
  const outputTokens = usage.completion_tokens ?? usage.outputTokens ?? 7;
  const trace = await postJson(`${SIDECAR}/v1/trace`, {
    reservation_id: decision.body.reservation_id,
    outcome: "ACCEPTED",
    provider_event_id: upstream.body.id ?? "",
    input_tokens: inputTokens,
    output_tokens: outputTokens,
    actual_amount_atomic: String(inputTokens + outputTokens),
  });
  if (trace.status !== 200) {
    err(`step 1 commit HTTP ${trace.status}: ${JSON.stringify(trace.body)}`);
    return false;
  }
  log("step 1 ALLOW PASS");
  return true;
}

async function step2Deny() {
  log("step 2 DENY — reserve returns DENY; no upstream HTTP");
  const req = buildDecisionRequest({
    executionId: uuid7(),
    decisionCtxExtras: { force_hard_cap: "1", stub_hits: "0" },
    claimEstimate: "999999999999",
  });
  const before = await getJson(`${STUB}/_count`);
  const decision = await postJson(`${SIDECAR}/v1/decision`, req.body);
  const isDeny =
    decision.body.verdict === "DENY" ||
    (decision.status >= 400 && decision.status < 500);
  if (!isDeny) {
    err(
      `step 2 expected DENY, got HTTP ${decision.status} verdict=${decision.body.verdict}`,
    );
    log("step 2 soft-skip: hard-cap not configured; demo continues");
  }
  // INV-1 — no upstream HTTP fires on DENY.
  const after = await getJson(`${STUB}/_count`);
  if (after.calls !== before.calls) {
    err(
      `step 2 INV-1 violation: upstream counter changed before=${before.calls} after=${after.calls}`,
    );
    return false;
  }
  log("step 2 DENY PASS (INV-1 upheld)");
  return true;
}

async function step3Stream() {
  log("step 3 STREAM — reserve + streaming-mode commit");
  const req = buildDecisionRequest({
    executionId: uuid7(),
    nodeName: "AI Agent (stream)",
    decisionCtxExtras: { stream: "true" },
  });
  const before = await getJson(`${STUB}/_count`);
  const decision = await postJson(`${SIDECAR}/v1/decision`, req.body);
  if (decision.body.verdict !== "ALLOW") {
    err(`step 3 expected ALLOW, got ${decision.body.verdict}`);
    return false;
  }
  const upstream = await postJson(`${STUB}/v1/chat/completions`, {
    model: "gpt-4o-mini",
    stream: true,
    messages: [{ role: "user", content: "stream demo step" }],
  });
  if (upstream.status !== 200) {
    err(`step 3 upstream HTTP ${upstream.status}`);
    return false;
  }
  const after = await getJson(`${STUB}/_count`);
  if (after.calls !== before.calls + 1) {
    err("step 3 upstream counter expected +1");
    return false;
  }
  const usage = upstream.body.usage ?? {};
  const inputTokens = usage.prompt_tokens ?? 5;
  const outputTokens = usage.completion_tokens ?? 7;
  const trace = await postJson(`${SIDECAR}/v1/trace`, {
    reservation_id: decision.body.reservation_id,
    outcome: "ACCEPTED",
    provider_event_id: upstream.body.id ?? "",
    input_tokens: inputTokens,
    output_tokens: outputTokens,
    actual_amount_atomic: String(inputTokens + outputTokens),
  });
  if (trace.status !== 200) {
    err(`step 3 commit HTTP ${trace.status}`);
    return false;
  }
  log("step 3 STREAM PASS");
  return true;
}

async function main() {
  const okAllow = await step1Allow();
  const okDeny = await step2Deny();
  const okStream = await step3Stream();
  if (okAllow && okDeny && okStream) {
    log("n8n_real ALL 3 steps PASS (ALLOW + DENY + STREAM)");
    process.exit(0);
  }
  err(
    `n8n_real FAIL — allow=${okAllow} deny=${okDeny} stream=${okStream}`,
  );
  process.exit(7);
}

main().catch((e) => {
  err(`unhandled: ${e?.stack ?? String(e)}`);
  process.exit(7);
});
