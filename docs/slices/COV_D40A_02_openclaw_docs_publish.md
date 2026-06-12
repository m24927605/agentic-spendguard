# COV_D40A_02 - OpenClaw base-URL docs publish and closeout

> **Deliverable:** D40a OpenClaw base-URL recipe
> **Slice:** 2 of 2
> **Spec set:** [`docs/specs/coverage/D40a_openclaw_base_url_recipe/`](../specs/coverage/D40a_openclaw_base_url_recipe/)
> **Precedence:** `design.md` is LOCKED and trumps this doc.

## Scope

Close D40a after the live recipe smoke passes: add README table row, CHANGELOG entry, docs cross-links, and memory writeback. This slice does not change demo behavior.

## LOCKED design quotes

From `design.md` §10:

> D40a is shipped when both slices are on main, the demo has physically run, `verify_step_openclaw_base_url.sql` is green, OpenClaw is present in the README table as base-URL recipe coverage, and `project_coverage_d40a_shipped.md` exists in memory.

From `design.md` §7:

> The doc must not claim plugin-level coverage. Use "base-URL recipe" and "egress proxy" wording.

## Files touched

| File | Why |
|---|---|
| `README.md` | Add OpenClaw base-URL recipe row. |
| `CHANGELOG.md` | Add D40a entry. |
| `docs/site-v2/src/content/docs/docs/drop-in/openclaw.mdx` | Final cross-links and polish only. |
| `/Users/michael.chen/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/project_coverage_d40a_shipped.md` | Memory closeout. |
| `/Users/michael.chen/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/MEMORY.md` | Index entry. |

## Test/verification plan

- TA-D40A-06 docs build.
- A2.1..A2.4 from acceptance.
- Confirm slice 1 demo evidence is present in commit history.

## Anti-scope

- No demo runner behavior changes.
- No OpenClaw plugin code.
- No broader strategy memo rewrite.
