# HARDEN_01 Retrospective — SLICE_08 cold_start_baseline_table

- Slice doc: `docs/slices/SLICE_08_cold_start_baseline_table.md`
- Merge commit: `6adb6f0`
- Merge base / first parent: `d6e81e9`
- Topic branch tip / second parent: `585c2dd`
- Diff command: `git diff 6adb6f0^1..6adb6f0`
- Diff size: 11 files, +2536/-47

## Review Focus

- L2 TOML source coverage and loader behavior
- Cold-start fallback from Strategy B to L2 and then L1
- Audit population of `cold_start_layer_used`
- Helm/demo wiring for output_predictor data asset

## Findings

No HARDEN_01 Blocker/Major findings in the static retrospective pass. The diff is mostly data curation plus loader tests, and the risky runtime integration is revalidated through SLICE_10/13/15 findings below.

## Residual Checks Routed Later

- HARDEN_02 must prove demo rows include `cold_start_layer_used='L2'` or a documented L1 fallback case.
- HARDEN_07 must verify the embedded TOML asset is included in release builds.

