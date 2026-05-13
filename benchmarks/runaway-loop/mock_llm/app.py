"""Mock OpenAI-compatible chat completions endpoint.

Serves /v1/chat/completions with deterministic usage numbers. Every
call is appended to /var/log/mock_llm.jsonl as the ground-truth call
log. The X-Runner header on each request lets the analyzer split
calls by runner. The analyzer applies a centralized pricing table to
each (model, input_tokens, output_tokens) tuple to compute actual $
spent — we deliberately do not let runners influence the cost number,
so each library's self-report is comparable against the same source.
"""

from __future__ import annotations

import json
import os
import time
import uuid
from pathlib import Path

from fastapi import FastAPI, Header, Request
from fastapi.responses import JSONResponse

LOG_PATH = Path(os.environ.get("MOCK_LLM_LOG", "/var/log/mock_llm.jsonl"))
INPUT_TOKENS = int(os.environ.get("INPUT_TOKENS", "4000"))
OUTPUT_TOKENS = int(os.environ.get("OUTPUT_TOKENS", "4000"))
LATENCY_MS = int(os.environ.get("LATENCY_MS", "10"))

LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
# Reset on every startup so back-to-back `make benchmark` runs don't
# stack call histories from previous runs into the analyzer.
LOG_PATH.write_text("")

app = FastAPI()


@app.get("/healthz")
def healthz() -> dict[str, str]:
    return {"status": "ok"}


@app.post("/v1/chat/completions")
async def chat_completions(
    request: Request,
    x_runner: str = Header(default="unknown"),
) -> JSONResponse:
    body = await request.json()
    model = body.get("model", "gpt-4o")
    if LATENCY_MS:
        time.sleep(LATENCY_MS / 1000.0)
    record = {
        "ts": time.time(),
        "runner": x_runner,
        "model": model,
        "input_tokens": INPUT_TOKENS,
        "output_tokens": OUTPUT_TOKENS,
        "ua": request.headers.get("user-agent", "")[:60],
        "ct": request.headers.get("content-type", ""),
    }
    with LOG_PATH.open("a") as f:
        f.write(json.dumps(record) + "\n")

    response_id = f"chatcmpl-{uuid.uuid4().hex[:24]}"
    return JSONResponse(
        content={
            "id": response_id,
            "object": "chat.completion",
            "created": int(record["ts"]),
            "model": model,
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "ok",
                    },
                    "finish_reason": "stop",
                },
            ],
            "usage": {
                "prompt_tokens": INPUT_TOKENS,
                "completion_tokens": OUTPUT_TOKENS,
                "total_tokens": INPUT_TOKENS + OUTPUT_TOKENS,
            },
        },
    )
