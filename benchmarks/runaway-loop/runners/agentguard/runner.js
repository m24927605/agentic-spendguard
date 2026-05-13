// AgentGuard runner — npm `agent-guard` drop-in for OpenAI calls.
//
// Pattern from https://github.com/dipampaul17/AgentGuard:
//   const agentGuard = require('agent-guard');
//   await agentGuard.init({ limit: 10 });
//   // openai client used as normal; AGENTGUARD_LIMIT_EXCEEDED thrown on cap
//
// We point the OpenAI client at the mock LLM and loop until the
// library aborts us. After exit, dump same fields as the Python
// runners to /results/agentguard.json.

const fs = require('fs');
const path = require('path');
const agentGuard = require('agent-guard');
const OpenAI = require('openai').default;

const BUDGET_USD = parseFloat(process.env.BUDGET_USD || '10.00');
const MAX_CALLS = parseInt(process.env.MAX_CALLS || '100', 10);
const BASE_URL = process.env.OPENAI_BASE_URL || 'http://mock-llm:8080/v1';
const RESULT_PATH = process.env.RESULT_PATH || '/results/agentguard.json';
const RUNNER_ID = 'agentguard';

async function main() {
  fs.mkdirSync(path.dirname(RESULT_PATH), { recursive: true });

  await agentGuard.init({ limit: BUDGET_USD, mode: 'throw' });

  const openai = new OpenAI({
    baseURL: BASE_URL,
    apiKey: 'sk-mock',
    defaultHeaders: { 'X-Runner': RUNNER_ID },
  });

  let callsAttempted = 0;
  let callsSucceeded = 0;
  let abortAtCall = null;
  let abortExceptionClass = null;
  let abortReason = null;
  const started = Date.now();

  for (let i = 0; i < MAX_CALLS; i++) {
    callsAttempted += 1;
    try {
      await openai.chat.completions.create({
        model: 'gpt-4o',
        messages: [{ role: 'user', content: `call ${i}` }],
      });
      callsSucceeded += 1;
    } catch (err) {
      abortAtCall = i + 1;
      abortExceptionClass = err.constructor.name;
      abortReason = err.message;
      break;
    }
  }

  const elapsed = (Date.now() - started) / 1000;

  let spentSelf = null;
  let remainingSelf = null;
  try {
    if (typeof agentGuard.getSpent === 'function') spentSelf = agentGuard.getSpent();
    if (typeof agentGuard.getRemaining === 'function') remainingSelf = agentGuard.getRemaining();
    if (typeof agentGuard.getStats === 'function') {
      const stats = agentGuard.getStats();
      spentSelf = stats.totalSpent ?? stats.spent ?? spentSelf;
      remainingSelf = stats.remaining ?? remainingSelf;
    }
  } catch (e) {
    spentSelf = `<error: ${e.message}>`;
  }

  const record = {
    runner: RUNNER_ID,
    budget_usd: BUDGET_USD,
    max_calls: MAX_CALLS,
    calls_attempted: callsAttempted,
    calls_succeeded: callsSucceeded,
    abort_at_call: abortAtCall,
    abort_exception_class: abortExceptionClass,
    abort_reason: abortReason,
    self_reported_spent: spentSelf,
    self_reported_remaining: remainingSelf,
    elapsed_seconds: Number(elapsed.toFixed(3)),
  };

  fs.writeFileSync(RESULT_PATH, JSON.stringify(record, null, 2));
  console.log(JSON.stringify(record, null, 2));

  if (typeof agentGuard.shutdown === 'function') {
    try { await agentGuard.shutdown(); } catch (e) { /* ignore */ }
  }
  if (typeof agentGuard.cleanup === 'function') {
    try { await agentGuard.cleanup(); } catch (e) { /* ignore */ }
  }
}

main()
  .then(() => process.exit(0))
  .catch((err) => {
    console.error(err);
    process.exit(1);
  });
