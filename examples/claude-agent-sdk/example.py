"""SpendGuard egress recipe smoke for Anthropic's claude-agent-sdk.

Drives one short, no-tool prompt through `claude-agent-sdk`. The SDK
subprocesses the `claude` CLI binary, which inherits HTTPS_PROXY and
NODE_EXTRA_CA_CERTS from the shell environment (D02 `spendguard install`
sets both). The resulting `POST /v1/messages` request flows through the
SpendGuard egress proxy, producing one RESERVE_RESPONSE row and one
matching COMMIT_OUTCOME row in audit_outbox.

This is intentionally NOT a SpendGuard SDK adapter — see the
integration page for why no SDK-level adapter exists for this framework:
https://agenticspendguard.dev/docs/integrations/claude-agent-sdk/

Usage:

    export ANTHROPIC_API_KEY=sk-ant-...
    # spendguard install already exported HTTPS_PROXY + NODE_EXTRA_CA_CERTS;
    # if it did not, re-run it. Verify in this shell:
    echo "$HTTPS_PROXY"  # → http://localhost:9000
    python example.py

After it prints, verify the audit chain with the SQL in README.md.
"""

from __future__ import annotations

import os
import sys

import anyio
from claude_agent_sdk import ClaudeAgentOptions, query

PROMPT = "List two ways to reverse a string in Python in under 30 words."
MODEL = "claude-sonnet-4-5"


async def _run() -> None:
    if not os.environ.get("ANTHROPIC_API_KEY"):
        print(
            "FATAL: ANTHROPIC_API_KEY is required (BYOK only — set it before running).",
            file=sys.stderr,
        )
        sys.exit(8)

    if not os.environ.get("HTTPS_PROXY"):
        print(
            "FATAL: HTTPS_PROXY is unset. Run `spendguard install`, then open a "
            "fresh shell so the rc snippet is picked up.",
            file=sys.stderr,
        )
        sys.exit(9)

    async for msg in query(
        prompt=PROMPT,
        options=ClaudeAgentOptions(model=MODEL, max_turns=1),
    ):
        print(msg)


def main() -> int:
    anyio.run(_run)
    print()
    print(
        "[example] claude-agent-sdk call complete. "
        "Verify the audit-chain rows with the SQL in README.md."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
