# AgenTrust Full Conformance Track v1alpha1

Status: Draft docs-first track after fixture MVP landing.

## Purpose

This document defines the delivery track that upgrades the landed AgenTrust
fixture MVP into full v1alpha1 fixture conformance for a future upstream
Community submission attempt.

The current repository state is intentionally narrower. It proves public CLI
parsing, committed allow/deny fixture replay, golden outputs, package smoke
exclusion of private modules, and a focused fixture CI gate. It does not prove
full v1alpha1 verifier conformance.

This track exists to close that gap without changing the public claim boundary
too early.

## Authority

The authority documents for this track are:

1. `docs/specs/agentrust-integration-spec-review-bundle-v1alpha1.md` for
   protocol behavior.
2. `docs/specs/agentrust-integration-plan-v1alpha1.md` for existing delivery
   sequencing context.
3. `docs/specs/agentrust-task-recovery-summary-v1alpha1.md` for the current
   fixture MVP boundary.

`docs/specs/agentrust-integration-spec-review-bundle-v1alpha1.md` is the only
normative source for public/private command grammar, path safety, runtime read
boundaries, validation phases, diagnostics, negative mutations, generated
outputs, package guard behavior, report truth, and evidence payloads.

This file is a delivery delta. It MUST NOT override or restate protocol details
from the review bundle. If this file appears to conflict with the review bundle,
the review bundle wins and this file must be patched before implementation
continues.

`docs/specs/agentrust-integration-spec-v1alpha1.md` is background only unless
the review bundle explicitly incorporates a rule from it.

## Traceability Requirement

The implementation plan, tasks, tests, CI, and final PR description MUST include
a traceability matrix. A slice is not ready unless its row names:

- the exact review-bundle section heading it implements;
- owned production files;
- owned test files;
- expected command or CI job;
- generated artifacts;
- make report row or package/public-scan evidence;
- failure behavior that proves the check is not a no-op.

The required matrix columns are exact:

```text
slice,review_bundle_section,owned_files,tests,command_or_job,artifacts,report_or_evidence_row,failure_probe
```

Plan review MUST reject any slice that lacks this row. Task and Final review
MUST reject any implemented behavior that is not traceable to at least one
review-bundle section and one test or make evidence row.

## Claim Boundary

Until this track is complete and accepted by review gates, public surfaces MUST
continue to say:

- AgenTrust support is planned or fixture-only.
- The repository is not upstream Community approved.
- The current verifier is not a general artifact verifier.
- TRACE output is Level 0 software-only fixture output signed by committed test
  keys.
- Agent Manifest output is unsigned artifact binding, not signed manifest trust.

Public docs, package metadata, CLI help, generated reports, and submission notes
MUST NOT claim:

- AgenTrust Verified status.
- Community approved or community-approved status.
- TRACE Level 1 or TRACE Level 2.
- hardware attestation.
- live verification.
- historical replay or replay-cache protection.
- signed Agent Manifest trust.
- CMCP or MCP gateway integration.
- production key distribution.
- SLSA, certification, compliance, or attested status.

## Required Delivery Outcome

The track is complete only when `make verify-agentrust` is promoted from the
fixture smoke harness into the full review-bundle conformance gate and passes on
a supported Linux CI runner.

Completion requires all of the following:

1. Public and private verifier behavior conforms to the review bundle.
2. Runtime verifier code respects the review-bundle runtime read boundary.
3. Make owns vectors, locks, expected outputs, fixture manifests, package probes,
   public-surface scans, aggregate report validation, and final package guard
   checks exactly as specified by the review bundle.
4. The allow and deny fixtures execute through real verifier code, including
   signed-byte reconstruction, trust-store authorization, Ed25519 verification,
   lifecycle derivation, artifact binding, and generated output validation.
5. The required make-only negative mutations are real mutations of fixture
   material or generated TRACE material, not canned diagnostics.
6. The make aggregate report is truth-preserving according to the review bundle.
7. Linux path and output handling uses the review-bundle local security and
   output contract.
8. Final artifact-lane package guard and public forbidden-claim scans satisfy the
   review bundle.
9. Full conformance CI runs on pinned `ubuntu-24.04`. Debian 12 is a supported
   local or self-hosted target only when the review-bundle environment preflight
   passes.

## Acceptance Gates

Observable delivery gates are:

| Gate | Required observable result |
|---|---|
| Public CLI/parser | public parser matrix tests cover the review-bundle command state matrix and assert exit, stdout, stderr, and diagnostic JSON bytes |
| Private harness | private entry point is gated to make/source checkout and is absent from installed public surfaces |
| Runtime read boundary | tests instrument file opens and prove runtime does not read make-owned vectors, locks, expected outputs, fixture manifests, or package guard inputs |
| Fixture success | allow and deny fixtures pass through verifier code and produce generated outputs validated by review-bundle rules |
| Negative mutations | each required mutation changes real fixture or generated TRACE material and reaches its expected verifier phase |
| Report truth | aggregate report rows are generated only from executed checks and validated against the review-bundle report contract |
| Linux output safety | `ubuntu-24.04` conformance job proves path traversal rejection, symlink rejection, temp/final output isolation, and cleanup behavior |
| Package guard | wheel, sdist, metadata, scripts, public help/docs, generated outputs, reports, diagnostics, and declared context entries are scanned |
| Public claims | public surfaces retain fixture-only disclaimers and do not claim approval, higher TRACE levels, hardware attestation, live verification, replay protection, signed manifest trust, CMCP/MCP gateway, production keys, SLSA, certification, compliance, or attestation |

These gates intentionally name delivery-level checks only. The exact protocol
bytes, diagnostic precedence, path algorithms, report schema, scan patterns, and
mutation construction remain owned by the review bundle.

## Migration Sequence

The target migration sequence is:

1. Keep the existing fast fixture smoke target as `make verify-agentrust-fixture`.
2. Add a separate full target `make verify-agentrust-full` while implementation is
   incomplete.
3. Add `.github/workflows/agentrust-conformance.yml` job
   `verify-agentrust-ubuntu-24.04` running the full target on pinned
   `ubuntu-24.04`.
4. When every acceptance gate above is green, promote `make verify-agentrust` to
   call the full target.
5. Keep `make verify-agentrust-fixture` as the fast smoke target after promotion.

Before promotion, green fixture smoke CI is not a full conformance signal. After
promotion, a green `verify-agentrust-ubuntu-24.04` job is required before any
public submission note can say the repository is ready for upstream review.

## Security Acceptance Gates

Delivery-level security gates are:

- public and private path traversal attempts are rejected;
- symlink bundle files, symlink output targets, and unsafe temp cleanup
  candidates are rejected;
- trust-store keys remain fixture scoped and committed test-key only;
- public verifier and installed artifacts do not expose private harness controls;
- private-key PEM bytes are excluded from wheel, sdist, declared context, docs,
  reports, diagnostics, and public generated artifacts;
- negative runs leave no selected output artifacts according to the review-bundle
  absence rules;
- package and public-claim scans run in the final artifact lane.

Every gate must have at least one positive test and one failure probe in the
traceability matrix.

## Non-Goals

This track does not add:

- live evidence verification;
- historical replay;
- replay-cache protection;
- TRACE Level 1 or Level 2;
- hardware attestation;
- signed Agent Manifest trust;
- CMCP or MCP gateway integration;
- production key distribution; or
- new SpendGuard runtime enforcement semantics.

The input remains committed synthetic allow/deny evidence bundles for a
fixture-only Community submission lane.

## Recovery Decisions

This section records the Staff+ Expert Recovery Cycle decision made after five
failed Spec review gate attempts.

The root cause was local protocol duplication. Earlier drafts copied low-level
rules from the review bundle into this delivery spec, then drifted on TRACE key
handling, runtime read boundaries, mutation constants, parser precedence,
negative-output cleanup, Linux path safety, aggregate report evidence, package
claim scanning, and byte-stable golden behavior.

The recovery decision is:

- Keep this file as a delivery delta only.
- Let `docs/specs/agentrust-integration-spec-review-bundle-v1alpha1.md` own all
  protocol details.
- Delete local protocol restatements from this file instead of adding more
  competing rules.
- Require implementation and tests to cite review-bundle sections directly when
  they need protocol behavior.
- Because the fifth failed Spec gate triggered Staff+ recovery, the panel's
  executed solution advances this track to the Plan gate. Future review-stage
  failures follow the Staff+ recovery rule in `AGENTS.md` and the user's
  clarification that the expert team decides, executes, and moves the work to
  the next stage unless an external permission or product decision is required.

## Implementation Slices

The implementation plan MUST map each slice to the review-bundle section it
implements and to at least one acceptance test or make report row.

Required slice families:

1. Public CLI and parser matrix.
2. Private make harness and source-checkout gate.
3. Runtime read-boundary instrumentation.
4. Schema, package, proto, vector, and fixture preflight.
5. Signed fixture verification for allow and deny.
6. Real negative mutations.
7. Generated output validation and golden comparison.
8. Truth-preserving aggregate make report.
9. Linux local-security and output hardening.
10. Final artifact-lane package guard and public forbidden-claim scan.
11. `ubuntu-24.04` full conformance CI.
12. Public docs and submission-note claim-boundary updates.

## Review Gates

This track uses the existing `AGENTS.md` gates:

```bash
CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh spec docs/specs/agentrust-full-conformance-spec-v1alpha1.md --project-dir "$PWD"
CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh plan docs/specs/agentrust-full-conformance-plan-v1alpha1.md --project-dir "$PWD"
BASELINE=$(git rev-parse HEAD)
CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh task "$BASELINE" --project-dir "$PWD"
CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh final branch:main --project-dir "$PWD"
```

If a stage reaches the fifth failed review, the Staff+ Expert Recovery Cycle in
`AGENTS.md` decides the fix, executes it, and advances according to that rule.

## Human Decisions

No human decision is required to start this track.

Decisions made for this spec:

- `ubuntu-24.04` is the required green CI lane.
- Debian 12 is optional and non-blocking unless a suitable local or self-hosted
  runner is explicitly configured.
- Keep a fast fixture smoke target during migration.
- Promote `make verify-agentrust` to full conformance only in the final track
  slice.
