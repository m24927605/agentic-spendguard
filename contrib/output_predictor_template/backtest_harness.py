"""Offline calibration harness for the Strategy C plugin template.

Reads a CSV of historical SpendGuard audit rows and reports how well
the configured model would have predicted ``actual_output_tokens``.

CSV schema
----------

Required columns (per `audit-chain-prediction-extension-v1alpha1.md` §2):

- ``tenant_id``                      str
- ``model``                          str (canonical model id)
- ``prompt_class``                   one of the 7 enum values
- ``input_tokens``                   int
- ``max_tokens_requested``           int (0 if unset)
- ``actual_output_tokens``           int (ground truth)

Optional columns the harness will use if present:

- ``conversation_depth``             int
- ``has_tool_calls``                 bool ("true"/"false"/0/1)
- ``has_system_message``             bool
- ``num_tool_definitions``           int
- ``user_role_hint``                 one of "first"/"continuation"/"tool_response"

Output
------

A text report covering:

- Sample size and per-prompt-class breakdown.
- P50 / P95 / P99 of the **calibration ratio** ``actual / predicted``.
  A perfectly calibrated model has a P50 of 1.0.
- A retraining recommendation flag (per spec calibration ratio > 1.05).
- Optional JSON output via ``--json-out`` for CI ingestion.

Usage
-----

::

    python backtest_harness.py --csv data/sample_audit_data.csv
    python backtest_harness.py --csv my_audit.csv --json-out report.json

This script is intentionally model-agnostic via ``--model-module``:
point it at a module exposing a ``StubModel``-shaped class to backtest
your replacement model without editing this file.
"""
from __future__ import annotations

import argparse
import csv
import importlib
import json
import logging
import statistics
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import numpy as np

from feature_extractor import vectorize_dict

LOGGER = logging.getLogger("spendguard.backtest")

# Spec: predicted / actual within ±5% is "well calibrated". Once the
# P95 ratio exceeds 1.05 (or drops below 0.95) the harness recommends
# retraining.
CALIBRATION_BAND_LOW = 0.95
CALIBRATION_BAND_HIGH = 1.05

REQUIRED_COLUMNS: tuple[str, ...] = (
    "tenant_id",
    "model",
    "prompt_class",
    "input_tokens",
    "max_tokens_requested",
    "actual_output_tokens",
)


@dataclass
class CalibrationStats:
    """Aggregate metrics for a slice of the backtest."""

    n: int = 0
    ratios: list[float] = field(default_factory=list)
    abs_errors: list[float] = field(default_factory=list)

    def add(self, *, predicted: int, actual: int) -> None:
        if predicted <= 0 or actual <= 0:
            return
        self.n += 1
        self.ratios.append(actual / predicted)
        self.abs_errors.append(abs(actual - predicted))

    def percentiles(self) -> dict[str, float]:
        if not self.ratios:
            return {"p50": float("nan"), "p95": float("nan"), "p99": float("nan")}
        sorted_ratios = sorted(self.ratios)
        return {
            "p50": _quantile(sorted_ratios, 0.50),
            "p95": _quantile(sorted_ratios, 0.95),
            "p99": _quantile(sorted_ratios, 0.99),
        }

    def mean_abs_error(self) -> float:
        return statistics.fmean(self.abs_errors) if self.abs_errors else float("nan")


def _quantile(sorted_values: list[float], q: float) -> float:
    if not sorted_values:
        return float("nan")
    if q <= 0:
        return sorted_values[0]
    if q >= 1:
        return sorted_values[-1]
    # Linear interpolation between adjacent ranks.
    pos = q * (len(sorted_values) - 1)
    lo = int(pos)
    hi = min(lo + 1, len(sorted_values) - 1)
    frac = pos - lo
    return sorted_values[lo] + (sorted_values[hi] - sorted_values[lo]) * frac


def _truthy(v: object) -> bool:
    if isinstance(v, bool):
        return v
    if isinstance(v, (int, float)):
        return v != 0
    if isinstance(v, str):
        return v.strip().lower() in {"1", "true", "yes", "y", "t"}
    return bool(v)


def _load_model(module_name: str) -> Any:
    """Import ``module_name`` and return its ``StubModel``-shaped class."""
    module = importlib.import_module(module_name)
    # Convention: the module exposes a single 0-arg class. We pick the
    # one named "StubModel" if present, otherwise the first attribute
    # ending in "Model".
    if hasattr(module, "StubModel"):
        return module.StubModel()
    for attr in dir(module):
        if attr.endswith("Model") and callable(getattr(module, attr)):
            return getattr(module, attr)()
    raise AttributeError(f"module {module_name!r} has no StubModel-like class")


def _validate_header(header: list[str]) -> None:
    missing = [c for c in REQUIRED_COLUMNS if c not in header]
    if missing:
        raise ValueError(
            f"CSV is missing required columns: {missing}. "
            f"Expected at least {list(REQUIRED_COLUMNS)}."
        )


def run_backtest(
    *,
    csv_path: Path,
    model: Any,
) -> dict[str, Any]:
    """Run the backtest and return a report dict (also suitable for JSON)."""
    with csv_path.open("r", newline="", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        if reader.fieldnames is None:
            raise ValueError(f"CSV at {csv_path} has no header row")
        _validate_header(list(reader.fieldnames))
        rows = list(reader)

    if not rows:
        raise ValueError(f"CSV at {csv_path} is empty")

    overall = CalibrationStats()
    per_class: dict[str, CalibrationStats] = defaultdict(CalibrationStats)
    per_model_family: dict[str, CalibrationStats] = defaultdict(CalibrationStats)

    # Batch the vector encoding so the model sees ``np.ndarray`` (faster
    # than 1-row predict calls; matches what a production predictor
    # would do for cron-style retraining backtests).
    vectors: list[np.ndarray] = []
    for row in rows:
        # Normalize a few CSV idiosyncrasies on the way in.
        cleaned = {
            **row,
            "input_tokens": int(row["input_tokens"] or 0),
            "max_tokens_requested": int(row["max_tokens_requested"] or 0),
            "conversation_depth": int(row.get("conversation_depth", 0) or 0),
            "num_tool_definitions": int(row.get("num_tool_definitions", 0) or 0),
            "has_tool_calls": _truthy(row.get("has_tool_calls", False)),
            "has_system_message": _truthy(row.get("has_system_message", False)),
        }
        vectors.append(vectorize_dict(cleaned))
    X = np.vstack(vectors)
    predictions = model.predict(X)

    for row, predicted in zip(rows, predictions, strict=True):
        actual = int(row["actual_output_tokens"] or 0)
        overall.add(predicted=int(predicted), actual=actual)
        per_class[row["prompt_class"]].add(predicted=int(predicted), actual=actual)
        from feature_extractor import model_family_of

        per_model_family[model_family_of(row["model"])].add(
            predicted=int(predicted), actual=actual
        )

    overall_pct = overall.percentiles()
    out_of_band = (
        overall_pct["p95"] > CALIBRATION_BAND_HIGH
        or overall_pct["p95"] < CALIBRATION_BAND_LOW
    )

    recommendation = (
        "RETRAIN recommended: P95 calibration ratio is outside the [0.95, 1.05] band."
        if out_of_band
        else "Calibration looks OK at the current confidence band."
    )

    return {
        "samples_total": overall.n,
        "samples_skipped": len(rows) - overall.n,
        "calibration": {
            "overall": {
                **overall_pct,
                "mean_abs_error_tokens": overall.mean_abs_error(),
            },
            "per_prompt_class": {
                cls: {
                    "n": s.n,
                    **s.percentiles(),
                    "mean_abs_error_tokens": s.mean_abs_error(),
                }
                for cls, s in sorted(per_class.items())
            },
            "per_model_family": {
                fam: {
                    "n": s.n,
                    **s.percentiles(),
                    "mean_abs_error_tokens": s.mean_abs_error(),
                }
                for fam, s in sorted(per_model_family.items())
            },
        },
        "calibration_band": {
            "low": CALIBRATION_BAND_LOW,
            "high": CALIBRATION_BAND_HIGH,
        },
        "recommendation": recommendation,
        "retrain_recommended": out_of_band,
    }


def render_text(report: dict[str, Any]) -> str:
    lines: list[str] = []
    lines.append("=== SpendGuard Output Predictor Backtest ===")
    lines.append(f"Samples: {report['samples_total']} (skipped {report['samples_skipped']})")
    o = report["calibration"]["overall"]
    lines.append("")
    lines.append("Overall calibration ratio (actual / predicted):")
    lines.append(f"  P50: {o['p50']:.3f}")
    lines.append(f"  P95: {o['p95']:.3f}")
    lines.append(f"  P99: {o['p99']:.3f}")
    lines.append(f"  Mean |error|: {o['mean_abs_error_tokens']:.1f} tokens")
    lines.append("")
    lines.append("Per prompt_class P95 ratios:")
    for cls, s in report["calibration"]["per_prompt_class"].items():
        lines.append(f"  {cls:>16s} (n={s['n']:>4d})  P50={s['p50']:.3f}  P95={s['p95']:.3f}")
    lines.append("")
    lines.append("Per model family P95 ratios:")
    for fam, s in report["calibration"]["per_model_family"].items():
        lines.append(f"  {fam:>20s} (n={s['n']:>4d})  P50={s['p50']:.3f}  P95={s['p95']:.3f}")
    lines.append("")
    band = report["calibration_band"]
    lines.append(f"Target band: P95 in [{band['low']:.2f}, {band['high']:.2f}]")
    lines.append(report["recommendation"])
    return "\n".join(lines)


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else "")
    parser.add_argument(
        "--csv",
        type=Path,
        default=Path(__file__).parent / "data" / "sample_audit_data.csv",
        help="Path to historical audit CSV (default: bundled sample).",
    )
    parser.add_argument(
        "--model-module",
        default="model_predictor_stub",
        help="Python module exposing a StubModel-shaped class.",
    )
    parser.add_argument(
        "--json-out",
        type=Path,
        default=None,
        help="If given, write the JSON report to this path in addition to stdout.",
    )
    parser.add_argument("--quiet", action="store_true", help="Suppress text output.")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
    model = _load_model(args.model_module)
    report = run_backtest(csv_path=args.csv, model=model)
    if not args.quiet:
        print(render_text(report))
    if args.json_out:
        args.json_out.write_text(json.dumps(report, indent=2))
        LOGGER.info("Wrote JSON report to %s", args.json_out)
    # Exit non-zero so CI catches calibration drift.
    return 0 if not report["retrain_recommended"] else 3


if __name__ == "__main__":  # pragma: no cover - CLI entrypoint
    sys.exit(main())
