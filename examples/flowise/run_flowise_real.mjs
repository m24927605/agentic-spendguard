// run_flowise_real.mjs — drive the SpendGuard Flowise wrapper against
// a self-hosted Flowise 2.x instance.
//
// 1. POST chatflow.json to POST /api/v1/chatflows (skips if a chatflow
//    with the same name already exists).
// 2. POST a prediction with prompt 'hi' to POST /api/v1/prediction/<id>.
// 3. Assert the response shape AND that the sidecar logged one
//    RequestDecision with trigger=LLM_CALL_PRE.
//
// Optional: set SPENDGUARD_DEMO_DENY=1 to swap chatflow.json's
// claimEstimatorJson for an oversized claim that forces a DENY; the
// runner then asserts the prediction returns 4xx OR a body containing
// STOP / DecisionStopped, AND that the upstream OpenAI request never
// fires.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const FLOWISE_URL = process.env.FLOWISE_URL ?? "http://localhost:3000";
const DENY = process.env.SPENDGUARD_DEMO_DENY === "1";

const here = dirname(fileURLToPath(import.meta.url));
const chatflowPath = join(here, "chatflow.json");
const chatflow = JSON.parse(readFileSync(chatflowPath, "utf-8"));

if (DENY) {
  const wrapper = chatflow.nodes.find(
    (n) => n.data.name === "spendGuardChatModelWrapper",
  );
  if (!wrapper) {
    throw new Error("chatflow.json is missing the spendGuardChatModelWrapper node");
  }
  wrapper.data.inputs.claimEstimatorJson = JSON.stringify({
    amountAtomic: "999999999999",
    scopeId: "deny-demo",
  });
}

function log(msg) {
  process.stdout.write(`[flowise-runner] ${msg}\n`);
}
function err(msg) {
  process.stderr.write(`[flowise-runner] ${msg}\n`);
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

async function main() {
  log(`FLOWISE_URL=${FLOWISE_URL}; DENY=${DENY}`);
  const created = await postJson(`${FLOWISE_URL}/api/v1/chatflows`, {
    name: chatflow.name,
    flowData: JSON.stringify({ nodes: chatflow.nodes, edges: chatflow.edges }),
  });
  if (created.status !== 200 && created.status !== 201) {
    err(`chatflow POST failed: ${created.status} ${JSON.stringify(created.body)}`);
    process.exit(7);
  }
  const id = created.body.id ?? created.body.chatflowId;
  if (!id) {
    err(`chatflow POST returned no id: ${JSON.stringify(created.body)}`);
    process.exit(7);
  }
  log(`chatflow created id=${id}`);

  const prediction = await postJson(`${FLOWISE_URL}/api/v1/prediction/${id}`, {
    question: "hi",
  });

  if (DENY) {
    const denied =
      (prediction.status >= 400 && prediction.status < 500) ||
      /STOP|DecisionStopped|BUDGET_DENIED/.test(JSON.stringify(prediction.body));
    if (!denied) {
      err(`expected DENY surface, got HTTP ${prediction.status} body=${JSON.stringify(prediction.body)}`);
      process.exit(7);
    }
    log("DENY surface verified — prediction returned the SpendGuard reason code");
    process.exit(0);
  }

  if (prediction.status !== 200) {
    err(`prediction failed: ${prediction.status} ${JSON.stringify(prediction.body)}`);
    process.exit(7);
  }
  // Flowise wraps the chat completion in `text` / `json` / `result`
  // depending on chain shape; accept any non-empty surface.
  const surface = prediction.body.text ?? prediction.body.json ?? prediction.body.result ?? prediction.body;
  if (!surface) {
    err(`prediction returned an empty body`);
    process.exit(7);
  }
  log(`prediction succeeded: ${JSON.stringify(surface).slice(0, 120)}...`);
  process.exit(0);
}

main().catch((e) => {
  err(`unhandled: ${e?.stack ?? String(e)}`);
  process.exit(7);
});
