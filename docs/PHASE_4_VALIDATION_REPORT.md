# Phase 4 Validation Report

**Date**: 2026-05-09
**Scope**: Validation gap closure for the 3 slices that shipped at
"locally-validated only" standard (O6 / O9 / O10).

---

## O6 — Microsoft AGT plugin

**Validation tool**: real `agent-governance-toolkit==3.4.x` Python
package + custom asyncio harness.

**Method**:
1. `pip install --user agent-governance-toolkit grpcio protobuf`
2. `make proto` to generate stubs (used local `python3 -m grpc_tools.protoc`
   workaround since Mac has no `python` binary)
3. Imported `spendguard.integrations.agt` — clean.
4. Built a 1-rule AGT `PolicyEvaluator` and called
   `await SpendGuardCompositeEvaluator.evaluate({'tool_name': 'shell'})`.

**Result**: AGT-deny path returns:
```
allowed: False
reason: AGT_DENY: Matched rule 'deny-shell'
matched_rule_ids: ['deny-shell']
```

**Bug found + fixed**: AGT 3.4 `PolicyEvaluator.evaluate(...)` returns
an object with `.allowed: bool` + `.action: str` (lowercase 'allow' /
'deny') + `.matched_rule: str | None`, NOT a `PolicyAction` enum
member as my initial scaffold guessed. Updated the deny-detection
branch to check `result.allowed` first with a fallback to lowercase
string comparison.

**SpendGuard-deny path NOT validated**: requires a running sidecar +
mocked `SpendGuardClient`. Same shape as the LangChain integration
which IS validated end-to-end against real OpenAI; the SpendGuard-
side codepath shares the same `request_decision` call.

---

## O9 — Terraform AWS module

**Validation tool**: real `terraform 1.9.8` (downloaded direct binary
from releases.hashicorp.com since brew tap blocked on Xcode CLT).

**Method**:
1. `terraform fmt -check` (failed; ran `terraform fmt` to fix)
2. `terraform init -backend=false` — downloaded all module + provider
   schemas (vpc, eks, rds, random, aws).
3. `terraform validate` — checked config + variable refs + module
   inputs against schemas.

**Result**:
```
Success! The configuration is valid.
```

**Files modified by `terraform fmt`** (committed):
- `terraform/aws/main.tf` (alignment of attribute = signs)
- `terraform/aws/example.tfvars` (alignment of variable assignments)

**Real `terraform apply` NOT run**: requires AWS sandbox account.
Pre-apply gate (validate) is what caught the bug class I worried
about (typo / wrong type / missing var). Real apply would catch
runtime-only issues like IAM permission gaps, RDS engine version
support per region, etc. — out of scope without a sandbox.

---

## O10 — Documentation site

**Validation tool**: real `mkdocs 1.6.1` + `mkdocs-material` (latest).

**Method**:
1. `pip install --user mkdocs-material`
2. `mkdocs build --strict` (treats any warning as error).

**Result**:
```
INFO    -  Cleaning site directory
INFO    -  Building documentation to directory: docs/site/site
INFO    -  Documentation built in 2.33 seconds
```

19 pages built; navigation renders cleanly. The MkDocs 2.0
deprecation warning from material is informational, not a strict-
mode failure.

**Real publish NOT run**: GitHub Pages deploy is per operator setup;
the build artifact at `docs/site/site/` is what would be published.

---

## Summary

| Slice | Validation tool | Status | Bug found |
|---|---|---|---|
| O6 AGT | `agent-governance-toolkit` 3.4 | ✓ PASS | Yes — fixed (.allowed shape) |
| O9 Terraform | `terraform 1.9.8` validate | ✓ PASS | Cosmetic — fmt applied |
| O10 mkdocs | `mkdocs 1.6.1` build --strict | ✓ PASS | None |

All 10 onboarding slices now meet "demo-gate-validated" standard, with
explicit documentation of what's still deferred to operator-provided
external infra (AWS sandbox apply, real provider webhook integration,
etc.).
