# spendguard-langflow-component

SpendGuard custom component for [Langflow](https://github.com/langflow-ai/langflow)
1.8+. Drag-drop budget gate for any LangChain chat model.

## What it does

Drops a single canvas card -- the **SpendGuard Budget Gate** -- onto
your Langflow flow. Connect a `ChatOpenAI` / `ChatAnthropic` /
`ChatVertexAI` / etc. node into the **Inner Model** input. Downstream
nodes see a budget-gated `LanguageModel` handle that:

- Pre-reserves projected spend against the SpendGuard sidecar BEFORE
  the inner model dispatches the upstream HTTP. DENY skips upstream
  entirely (INV-1).
- Commits real `total_tokens` from the response usage frame
  end-of-call (INV-5).
- Surfaces `DecisionDenied` / `DecisionSkipped` as Langflow error
  nodes -- fail-closed by default.

All reservation/commit logic lives in the existing
[`spendguard.integrations.langchain.SpendGuardChatModel`](https://github.com/michael-chen/agentic-spendguard/blob/main/sdk/python/src/spendguard/integrations/langchain.py).
This package is adapter glue: canvas inputs, run-context auto-bind, and
audit-chain tagging so Langflow calls are distinguishable from raw
LangChain SDK callers.

## Install

```bash
pip install spendguard-langflow-component
spendguard-langflow-install --target $LANGFLOW_COMPONENTS_PATH
```

Restart Langflow. The **SpendGuard Budget Gate** card appears under the
**Models** category in the canvas palette.

### Air-gapped / vendor-drop install

```bash
# 1. Copy the component file + metadata YAML into the Langflow tree.
cp src/spendguard_langflow/component.py \
   $LANGFLOW_COMPONENTS_PATH/spendguard_chat_model_wrapper.py
cp src/spendguard_langflow/metadata/spendguard_chat_model_wrapper.yaml \
   $LANGFLOW_COMPONENTS_PATH/spendguard_chat_model_wrapper.yaml

# 2. Ensure the runtime has spendguard-sdk[langchain]>=0.5.1.
pip install 'spendguard-sdk[langchain]>=0.5.1'
```

## Requirements

- **Langflow** `>=1.8.0,<2.0.0`. v1.7 lacked stable `HandleInput` +
  `LanguageModel` handle support.
- **Python** `>=3.10`.
- **spendguard-sdk[langchain]** `>=0.5.1`.

## Canvas inputs

| Input | Type | Required | Notes |
|---|---|---|---|
| `inner` | `LanguageModel` handle | yes | Drop a `ChatOpenAI` / `ChatAnthropic` / etc. node and connect its output here. |
| `sidecar_uds_path` | text | yes | Filesystem path of the SpendGuard sidecar UDS. Falls back to env `SPENDGUARD_SIDECAR_UDS`. |
| `tenant_id` | secret | yes | Operator-issued tenant UUID. |
| `budget_id` | text | yes | UUID of the SpendGuard budget this gate debits. |
| `window_instance_id` | text | yes | UUID of the active rolling window. |
| `unit_token_kind` | text | no (advanced, default `output_token`) | Token kind for the BudgetClaim. |
| `model_family` | text | no (advanced, default `gpt-4`) | Model family label baked into the UnitRef. |
| `claim_estimator_chars_per_token` | int | no (advanced, default `4`) | Heuristic divisor for the default chars/N estimator. |

## Limitations (v1)

- **No embeddings gate.** Budget gate fires only at LLM call boundary.
- **No token-by-token mid-stream cap.** End-of-stream commit only.
- **Per-node wrap only.** Global model-provider config interception
  (Langflow v1.8+ feature) deferred to v1.1.
- **No Langflow Cloud marketplace push.** PyPI is the install surface;
  Cloud push is a follow-up.
- **Per-flow budget IDs read from canvas inputs only.** Flow metadata
  reading deferred.

See [the full docs page](https://spendguard.dev/docs/integrations/langflow)
for the decision matrix vs the SpendGuard egress proxy and a canvas
screenshot.

## Demo

```bash
git clone https://github.com/michael-chen/agentic-spendguard
cd agentic-spendguard
make demo-up DEMO_MODE=langflow_real
```

Brings up the base SpendGuard stack + an in-network counting stub + a
Python 3.12 runner that exercises the component's reserve / commit /
release lifecycle in a 3-step matrix (ALLOW + DENY + STREAM).

## License

Apache-2.0. Same license as `spendguard-sdk`.
