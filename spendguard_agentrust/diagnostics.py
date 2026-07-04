from __future__ import annotations

from dataclasses import dataclass
import json


DIAGNOSTIC_SCHEMA = "spendguard-agentrust-diagnostic/v1"


@dataclass(frozen=True)
class Diagnostic:
    code: str
    exit_code: int = 2
    phase: str = "phase-1"
    json_pointer: str | None = None

    @property
    def message(self) -> str:
        return self.code.replace("_", " ")


def diagnostic_json(diagnostic: Diagnostic) -> str:
    payload = {
        "schema": DIAGNOSTIC_SCHEMA,
        "exit_code": diagnostic.exit_code,
        "phase": diagnostic.phase,
        "code": diagnostic.code,
        "message": diagnostic.message,
        "json_pointer": diagnostic.json_pointer,
    }
    return json.dumps(payload, separators=(",", ":"), ensure_ascii=False)
