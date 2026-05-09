# LangChain & LangGraph integration

Same module covers both since LangGraph operates on LangChain's
`BaseChatModel` interface.

```python
from langchain_openai import ChatOpenAI
from langgraph.prebuilt import create_react_agent

from spendguard import SpendGuardClient
from spendguard.integrations.langchain import (
    SpendGuardChatModel, RunContext, run_context,
)

guarded = SpendGuardChatModel(
    inner=ChatOpenAI(model="gpt-4o-mini"),
    client=client,
    budget_id=...,
    # ...
    claim_estimator=lambda messages: [common_pb2.BudgetClaim(...)],
)

agent = create_react_agent(guarded, tools=[my_tool])

async with run_context(RunContext(run_id=...)):
    await agent.ainvoke({"messages": [...]})
```

Install: `pip install 'spendguard-sdk[langchain,langgraph]'`

`bind_tools()` and `with_structured_output()` forward to the inner
model so LangGraph + structured-output prompts work without code
changes.
