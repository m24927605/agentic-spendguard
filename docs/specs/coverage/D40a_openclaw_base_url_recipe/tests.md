# D40a - Tests

Numbered tests use **TP-D40A-XX** for unit/static checks and **TA-D40A-XX** for acceptance gates.

## 1. Static and docs tests

| ID | Test | Verifies |
|---|---|---|
| TP-D40A-01 | Docs page contains the locked wording "base-URL recipe" and "egress proxy"; it does not contain "provider plugin coverage". | Claim discipline. |
| TP-D40A-02 | Example config uses the pinned `OA-V1` key names and `http://localhost:9000/v1`. | Config accuracy. |
| TP-D40A-03 | `deploy/demo/Makefile` has `DEMO_MODE=openclaw_base_url` and `demo-verify-openclaw-base-url`. | Make wiring. |
| TP-D40A-04 | Verify SQL uses `ON_ERROR_STOP=1` and `COV_D40A_GATE` assertion labels. | Hard gate style. |
| TP-D40A-05 | No files under `sdk/fixtures/cross-language/` change. | Frozen corpus invariant. |

## 2. Demo behavior tests

| ID | Test | Verifies |
|---|---|---|
| TP-D40A-06 | Runner ALLOW call returns content and increments counting stub once. | OpenClaw traffic uses proxy path. |
| TP-D40A-07 | Runner DENY call returns a SpendGuard denial and counting stub remains unchanged. | Pre-dispatch hard gate. |
| TP-D40A-08 | Runner STREAM call receives stream chunks and commits once at stream close. | Streaming path. |
| TP-D40A-09 | Runtime env includes `unitId`, `windowInstanceId`, and pricing tuple. | HARDEN_D05_UR/WI day-1 discipline. |

## 3. Acceptance gates

| ID | Command | Pass condition |
|---|---|---|
| TA-D40A-01 | `make demo-down` | exits 0; stale volume state wiped before the live demo. |
| TA-D40A-02 | `make demo-up DEMO_MODE=openclaw_base_url` | exits 0 and prints `[demo] openclaw_base_url ALL 3 steps PASS (ALLOW + DENY + STREAM)`. |
| TA-D40A-03 | `make -C deploy/demo demo-verify-openclaw-base-url` | SQL hard gates pass. |
| TA-D40A-04 | `rg -n "openclaw_base_url" deploy/demo/Makefile` | mode branch and verify target present. |
| TA-D40A-05 | `git diff --stat -- sdk/fixtures/cross-language` | empty. |
| TA-D40A-06 | docs site build command used by the repo | exits 0; OpenClaw page renders without MDX/Astro parsing errors. |

## 4. Slice mapping

| Slice | Tests |
|---|---|
| `COV_D40A_01_openclaw_recipe_smoke` | TP-D40A-01..09, TA-D40A-01..05 |
| `COV_D40A_02_openclaw_docs_publish` | TA-D40A-06 plus README/CHANGELOG presence checks |
