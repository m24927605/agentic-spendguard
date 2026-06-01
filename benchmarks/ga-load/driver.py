#!/usr/bin/env python3
"""GA_08 real-stack load driver.

Runs inside the demo container so it can use the sidecar UDS and the
compose-only service DNS names. The driver intentionally avoids PyYAML:
the scenario file is JSON-compatible YAML, which keeps the demo image
dependency surface unchanged.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import math
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any
from urllib.request import urlopen

import grpc

from spendguard import SpendGuardClient, derive_idempotency_key, new_uuid7
from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard._proto.spendguard.sidecar_adapter.v1 import adapter_pb2
from spendguard.run_plan import with_run_plan


def load_scenario(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as fh:
        return json.load(fh)


def require_env(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"missing required environment variable {name}")
    return value


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    rank = max(1, math.ceil((pct / 100.0) * len(ordered)))
    return ordered[min(rank - 1, len(ordered) - 1)]


def summarize(values: list[float]) -> dict[str, float | int]:
    return {
        "count": len(values),
        "p50_ms": round(percentile(values, 50), 3),
        "p95_ms": round(percentile(values, 95), 3),
        "p99_ms": round(percentile(values, 99), 3),
        "max_ms": round(max(values), 3) if values else 0.0,
    }


async def timed(label: str, latencies: dict[str, list[float]], fn):
    start = time.perf_counter()
    result = await fn()
    latencies.setdefault(label, []).append((time.perf_counter() - start) * 1000.0)
    return result


def generate_probe_stubs(proto_root: Path):
    out_dir = Path(tempfile.mkdtemp(prefix="ga_load_proto_"))
    protos = [
        proto_root / "spendguard/tokenizer/v1/tokenizer.proto",
        proto_root / "spendguard/output_predictor/v1/predictor.proto",
        proto_root / "spendguard/run_cost_projector/v1/projector.proto",
    ]
    subprocess.run(
        [
            sys.executable,
            "-m",
            "grpc_tools.protoc",
            f"--proto_path={proto_root}",
            f"--python_out={out_dir}",
            f"--grpc_python_out={out_dir}",
            *(str(p) for p in protos),
        ],
        check=True,
    )
    sys.path.insert(0, str(out_dir))
    import spendguard as spendguard_pkg

    spendguard_pkg.__path__.append(str(out_dir / "spendguard"))
    from spendguard.output_predictor.v1 import predictor_pb2, predictor_pb2_grpc
    from spendguard.run_cost_projector.v1 import projector_pb2, projector_pb2_grpc
    from spendguard.tokenizer.v1 import tokenizer_pb2, tokenizer_pb2_grpc

    return tokenizer_pb2, tokenizer_pb2_grpc, predictor_pb2, predictor_pb2_grpc, projector_pb2, projector_pb2_grpc


def metric_value(metrics: str, name: str, labels: dict[str, str] | None = None) -> float:
    labels = labels or {}
    pattern = re.compile(rf"^{re.escape(name)}(?:\{{([^}}]*)\}})?\s+([-+0-9.eE]+)$")
    for line in metrics.splitlines():
        match = pattern.match(line.strip())
        if not match:
            continue
        raw_labels = match.group(1) or ""
        parsed: dict[str, str] = {}
        for part in raw_labels.split(","):
            if not part or "=" not in part:
                continue
            key, value = part.split("=", 1)
            parsed[key.strip()] = value.strip().strip('"')
        if all(parsed.get(k) == v for k, v in labels.items()):
            return float(match.group(2))
    return 0.0


def scrape_metrics() -> dict[str, Any]:
    endpoints = {
        "output_predictor": "http://output-predictor:9100/metrics",
        "run_cost_projector": "http://run-cost-projector:9102/metrics",
        "sidecar": "http://sidecar:9093/metrics",
        "tokenizer": "http://tokenizer:9099/metrics",
    }
    scraped: dict[str, str] = {}
    for name, url in endpoints.items():
        try:
            with urlopen(url, timeout=5) as resp:
                scraped[name] = resp.read().decode("utf-8", errors="replace")
        except Exception as exc:  # noqa: BLE001
            scraped[name] = f"# scrape_error {exc!r}\n"

    return {
        "output_predictor_predict_ok_total": metric_value(
            scraped["output_predictor"],
            "spendguard_output_predictor_predict_total",
            {"outcome": "ok"},
        ),
        "output_predictor_predict_count": metric_value(
            scraped["output_predictor"],
            "spendguard_output_predictor_predict_latency_seconds_count",
        ),
        "output_predictor_cache_lookups": metric_value(
            scraped["output_predictor"],
            "spendguard_output_predictor_cache_lookup_total",
        ),
        "run_cost_projector_project_ok_total": metric_value(
            scraped["run_cost_projector"],
            "spendguard_run_cost_projector_project_total",
            {"outcome": "ok"},
        ),
        "run_cost_projector_project_count": metric_value(
            scraped["run_cost_projector"],
            "spendguard_run_cost_projector_project_latency_seconds_count",
        ),
        "sidecar_request_decision_ok": metric_value(
            scraped["sidecar"],
            "spendguard_sidecar_handler_calls_total",
            {"handler": "request_decision", "outcome": "ok"},
        ),
        "sidecar_confirm_publish_outcome_ok": metric_value(
            scraped["sidecar"],
            "spendguard_sidecar_handler_calls_total",
            {"handler": "confirm_publish_outcome", "outcome": "ok"},
        ),
        "sidecar_emit_trace_events_ok": metric_value(
            scraped["sidecar"],
            "spendguard_sidecar_handler_calls_total",
            {"handler": "emit_trace_events", "outcome": "ok"},
        ),
        "tokenizer_tier3_hits": metric_value(
            scraped["tokenizer"],
            "spendguard_tokenizer_tier3_hit_total",
        ),
    }


async def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--scenario", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--proto-root", required=True)
    args = parser.parse_args()

    scenario_path = Path(args.scenario)
    output_path = Path(args.output)
    scenario = load_scenario(scenario_path)
    (
        tokenizer_pb2,
        tokenizer_pb2_grpc,
        predictor_pb2,
        predictor_pb2_grpc,
        projector_pb2,
        projector_pb2_grpc,
    ) = generate_probe_stubs(Path(args.proto_root))

    socket_path = require_env("SPENDGUARD_SIDECAR_UDS")
    tenant_id = require_env("SPENDGUARD_TENANT_ID")
    budget_id = require_env("SPENDGUARD_BUDGET_ID")
    window_id = require_env("SPENDGUARD_WINDOW_INSTANCE_ID")
    unit_id = require_env("SPENDGUARD_UNIT_ID")
    pricing_version = require_env("SPENDGUARD_PRICING_VERSION")
    fx_rate_version = require_env("SPENDGUARD_FX_RATE_VERSION")
    unit_conversion_version = require_env("SPENDGUARD_UNIT_CONVERSION_VERSION")
    price_snapshot_hash_hex = require_env("SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX")

    logical_tenants = int(scenario["logical_tenants"])
    requests_per_logical_tenant = int(scenario["requests_per_logical_tenant"])
    expected_ops = logical_tenants * requests_per_logical_tenant
    concurrency = int(scenario["concurrency"])
    claim_amount = str(int(scenario["claim_amount_atomic"]))
    actual_input_tokens = int(scenario["actual_input_tokens"])
    actual_output_tokens = int(scenario["actual_output_tokens"])
    planned_calls = int(scenario["planned_calls"])
    planned_tools = int(scenario["planned_tools"])
    budget_remaining_atomic = str(int(scenario["budget_remaining_atomic"]))
    models = list(scenario["models"])
    providers = list(scenario["providers"])
    prompt_classes = list(scenario["prompt_classes"])
    local_smoke_limits = dict(scenario["local_smoke_limits"])

    latencies: dict[str, list[float]] = {}
    errors: list[dict[str, Any]] = []
    completed: list[dict[str, str]] = []

    tokenizer_channel = grpc.aio.insecure_channel("tokenizer:50053")
    predictor_channel = grpc.aio.insecure_channel("output-predictor:50054")
    projector_channel = grpc.aio.insecure_channel("run-cost-projector:50055")
    tokenizer = tokenizer_pb2_grpc.TokenizerStub(tokenizer_channel)
    predictor = predictor_pb2_grpc.OutputPredictorStub(predictor_channel)
    projector = projector_pb2_grpc.RunCostProjectorStub(projector_channel)

    client = SpendGuardClient(
        socket_path=socket_path,
        tenant_id=tenant_id,
        runtime_kind="ga-load",
        sdk_version="0.5.0-ga08",
        decision_timeout_s=5.0,
        publish_timeout_s=5.0,
        trace_timeout_s=5.0,
    )
    await client.connect()
    await client.handshake(workload_instance_id="ga-load-driver")

    unit = common_pb2.UnitRef(
        unit_id=unit_id,
        token_kind="output_token",
        model_family="gpt-4",
    )
    pricing = common_pb2.PricingFreeze(
        pricing_version=pricing_version,
        price_snapshot_hash=bytes.fromhex(price_snapshot_hash_hex),
        fx_rate_version=fx_rate_version,
        unit_conversion_version=unit_conversion_version,
    )

    @with_run_plan(planned_calls=planned_calls, planned_tools=planned_tools)
    async def call_sidecar(
        *,
        run_id: str,
        step_id: str,
        llm_call_id: str,
        decision_id: str,
        provider: str,
        model: str,
        prompt_class: str,
        tokenized,
        prediction,
    ) -> None:
        projected_claims = [
            common_pb2.BudgetClaim(
                budget_id=budget_id,
                unit=unit,
                amount_atomic=claim_amount,
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id=window_id,
            )
        ]
        idempotency_key = derive_idempotency_key(
            tenant_id=tenant_id,
            session_id=client.session_id,
            run_id=run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )
        predicted_b = prediction.predicted_b_tokens if prediction.HasField("predicted_b_tokens") else 0
        # The local compose stack has no customer Strategy C plugin configured.
        # Populate the C audit mirror with a conservative synthetic value so
        # GA_08 exercises the locked prediction columns and delta checks while
        # keeping `prediction_strategy_used` on the real predictor response.
        predicted_c = (
            prediction.predicted_c_tokens
            if prediction.HasField("predicted_c_tokens")
            else max(1, predicted_b or prediction.predicted_a_tokens)
        )
        confidence = prediction.confidence if prediction.HasField("confidence") else 0.0
        sample_size = prediction.sample_size if prediction.HasField("sample_size") else 0
        cold_start_layer = (
            prediction.cold_start_layer_used
            if prediction.HasField("cold_start_layer_used")
            else ""
        )
        claim_estimate = adapter_pb2.ClaimEstimate(
            tokenizer_tier=tokenized.tier,
            tokenizer_version_id=tokenized.tokenizer_version_id,
            input_tokens=tokenized.input_tokens,
            predicted_a_tokens=prediction.predicted_a_tokens,
            predicted_b_tokens=predicted_b,
            predicted_c_tokens=predicted_c,
            reserved_strategy=prediction.reserved_strategy,
            prediction_strategy_used=prediction.prediction_strategy_used,
            prediction_policy_used="STRICT_CEILING",
            prediction_confidence=confidence,
            prediction_sample_size=sample_size,
            cold_start_layer_used=cold_start_layer,
            classifier_version=prediction.classifier_version,
            fingerprint_version=prediction.fingerprint_version,
            prompt_class_fingerprint=prediction.prompt_class_fingerprint_used,
            run_projection_at_decision_atomic=int(claim_amount) * max(1, planned_calls),
            run_predicted_remaining_steps=max(0, planned_calls + planned_tools - 1),
            run_steps_completed_so_far=0,
            run_code_triggered="",
            model=model,
            prompt_class=prompt_class,
        )
        outcome = await timed(
            "sidecar_decision",
            latencies,
            lambda: client.request_decision(
                trigger="LLM_CALL_PRE",
                run_id=run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                tool_call_id="",
                decision_id=decision_id,
                route=f"llm.call.{provider}",
                projected_claims=projected_claims,
                projected_unit=unit,
                idempotency_key=idempotency_key,
                prompt_text=f"{scenario['name']} provider={provider} model={model} run={run_id}",
                decision_context_json={
                    "budget_remaining_atomic": budget_remaining_atomic,
                    "integration": "ga-load",
                    "model": model,
                    "call_type": "chat.completions",
                    "mode": "load",
                },
                claim_estimate=claim_estimate,
            ),
        )
        await timed(
            "sidecar_confirm_publish_outcome",
            latencies,
            lambda: client.confirm_publish_outcome(
                decision_id=outcome.decision_id,
                effect_hash=outcome.effect_hash,
                outcome="APPLIED_NOOP",
            ),
        )
        if not outcome.reservation_ids:
            raise RuntimeError("sidecar returned no reservation_ids")
        await timed(
            "sidecar_emit_trace_events",
            latencies,
            lambda: client.emit_llm_call_post(
                run_id=run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                decision_id=outcome.decision_id,
                reservation_id=outcome.reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(actual_output_tokens),
                unit=unit,
                pricing=pricing,
                provider_event_id=f"ga-load-{llm_call_id}",
                outcome="SUCCESS",
                actual_input_tokens=actual_input_tokens,
                actual_output_tokens=actual_output_tokens,
                delta_b_ratio=(
                    float(actual_output_tokens) / float(predicted_b)
                    if predicted_b > 0
                    else None
                ),
                delta_c_ratio=(
                    float(actual_output_tokens) / float(predicted_c)
                    if predicted_c > 0
                    else None
                ),
            ),
        )

    async def run_one(index: int) -> None:
        logical_idx = index % logical_tenants
        model = models[index % len(models)]
        provider = providers[index % len(providers)]
        prompt_class = prompt_classes[index % len(prompt_classes)]
        logical_tenant = f"tenant-{logical_idx:03d}"
        agent_id = f"agent-{logical_idx % 20:02d}"
        run_id = str(new_uuid7())
        step_id = agent_id
        llm_call_id = str(new_uuid7())
        decision_id = str(new_uuid7())
        prompt = (
            f"{scenario['name']} logical_tenant={logical_tenant} "
            f"provider={provider} prompt_class={prompt_class}"
        )
        start = time.perf_counter()
        try:
            tokenized = await timed(
                "tokenizer",
                latencies,
                lambda: tokenizer.Tokenize(
                    tokenizer_pb2.TokenizeRequest(
                        model=model,
                        raw_text=prompt,
                        request_id=str(new_uuid7()),
                    ),
                    timeout=5.0,
                ),
            )
            prediction = await timed(
                "output_predictor",
                latencies,
                lambda: predictor.Predict(
                    predictor_pb2.PredictRequest(
                        tenant_id=tenant_id,
                        model=model,
                        agent_id=agent_id,
                        prompt_class=prompt_class,
                        input_tokens=tokenized.input_tokens,
                        max_tokens_requested=64,
                        model_context_window=8000,
                        prediction_policy="STRICT_CEILING",
                        decision_id=decision_id,
                        run_id=run_id,
                        prompt_class_fingerprint=f"ga08-{prompt_class}-{logical_idx % 20:02d}",
                    ),
                    timeout=5.0,
                ),
            )
            await timed(
                "run_cost_projector",
                latencies,
                lambda: projector.Project(
                    projector_pb2.ProjectRequest(
                        tenant_id=tenant_id,
                        run_id=str(new_uuid7()),
                        agent_id=agent_id,
                        model=model,
                        step_id=step_id,
                        decision_id=str(new_uuid7()),
                        this_call_reservation_atomic=int(claim_amount),
                        unit_id=unit_id,
                        budget_remaining_atomic=int(budget_remaining_atomic),
                        planned_steps_hint=planned_calls + planned_tools,
                        planned_tools_hint=planned_tools,
                    ),
                    timeout=5.0,
                ),
            )
            await call_sidecar(
                run_id=run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                decision_id=decision_id,
                provider=provider,
                model=model,
                prompt_class=prompt_class,
                tokenized=tokenized,
                prediction=prediction,
            )
            latencies.setdefault("end_to_end", []).append((time.perf_counter() - start) * 1000.0)
            completed.append(
                {
                    "logical_tenant": logical_tenant,
                    "run_id": run_id,
                    "agent_id": agent_id,
                    "model": model,
                    "provider": provider,
                    "prompt_class": prompt_class,
                }
            )
        except Exception as exc:  # noqa: BLE001
            errors.append(
                {
                    "index": index,
                    "logical_tenant": logical_tenant,
                    "provider": provider,
                    "model": model,
                    "error": repr(exc),
                }
            )

    sem = asyncio.Semaphore(concurrency)
    metrics_before = scrape_metrics()

    async def guarded(index: int) -> None:
        async with sem:
            await run_one(index)

    started = time.time()
    await asyncio.gather(*(guarded(i) for i in range(expected_ops)))
    finished = time.time()
    await client.close()
    await tokenizer_channel.close()
    await predictor_channel.close()
    await projector_channel.close()

    latency_summary = {name: summarize(values) for name, values in sorted(latencies.items())}
    metrics = scrape_metrics()
    metric_deltas = {
        name: round(float(metrics.get(name, 0.0)) - float(metrics_before.get(name, 0.0)), 6)
        for name in sorted(metrics)
    }
    cardinality = {
        "actual_tenants": 1,
        "logical_tenants": len({r["logical_tenant"] for r in completed}),
        "runs": len({r["run_id"] for r in completed}),
        "agents": len({r["agent_id"] for r in completed}),
        "models": len({r["model"] for r in completed}),
        "providers": len({r["provider"] for r in completed}),
        "prompt_classes": len({r["prompt_class"] for r in completed}),
    }

    failures: list[str] = []
    if errors:
        failures.append(f"{len(errors)} operation errors")
    if len(completed) != expected_ops:
        failures.append(f"completed {len(completed)} of expected {expected_ops} operations")
    for key, limit_key in [
        ("tokenizer", "tokenizer_p99_ms"),
        ("output_predictor", "output_predictor_p99_ms"),
        ("run_cost_projector", "run_cost_projector_p99_ms"),
        ("sidecar_decision", "sidecar_decision_p99_ms"),
        ("sidecar_confirm_publish_outcome", "sidecar_commit_p99_ms"),
        ("sidecar_emit_trace_events", "sidecar_emit_p99_ms"),
        ("end_to_end", "end_to_end_p99_ms"),
    ]:
        if limit_key not in local_smoke_limits:
            continue
        p99 = float(latency_summary.get(key, {}).get("p99_ms", 0.0))
        if p99 > float(local_smoke_limits[limit_key]):
            failures.append(
                f"{key} p99 {p99}ms exceeds local smoke limit "
                f"{limit_key}={local_smoke_limits[limit_key]}ms"
            )

    if metric_deltas["output_predictor_predict_count"] < expected_ops:
        failures.append("output_predictor metrics count below expected operation count")
    expected_projector_calls = expected_ops * 2
    if metric_deltas["run_cost_projector_project_count"] < expected_projector_calls:
        failures.append(
            "run_cost_projector project count below expected direct+sidecar call count "
            f"({metric_deltas['run_cost_projector_project_count']} < {expected_projector_calls})"
        )
    if metric_deltas["run_cost_projector_project_ok_total"] < expected_projector_calls:
        failures.append(
            "run_cost_projector ok total below expected direct+sidecar call count "
            f"({metric_deltas['run_cost_projector_project_ok_total']} < {expected_projector_calls})"
        )
    if metric_deltas["sidecar_request_decision_ok"] < expected_ops:
        failures.append("sidecar request_decision ok count below expected operation count")
    if metric_deltas["sidecar_emit_trace_events_ok"] < expected_ops:
        failures.append("sidecar emit_trace_events ok count below expected operation count")

    result = {
        "result": "pass" if not failures else "fail",
        "scenario": scenario,
        "started_at_unix": started,
        "finished_at_unix": finished,
        "duration_seconds": round(finished - started, 3),
        "expected_operations": expected_ops,
        "completed_operations": len(completed),
        "error_count": len(errors),
        "errors": errors[:20],
        "latency": latency_summary,
        "cardinality": cardinality,
        "service_metrics": metrics,
        "service_metric_deltas": metric_deltas,
        "failures": failures,
    }
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if not failures else 1


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
