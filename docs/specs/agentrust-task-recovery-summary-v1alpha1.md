# AgenTrust Task Recovery Summary v1alpha1

Status: Accepted by Staff+ Expert Recovery Cycle on 2026-07-04.

## Decision

Advance the Task stage as a fixture-only docs-first MVP, not as full
cryptographic or Linux-hardening conformance.

The current implementation is acceptable only for this narrowed milestone:

- public CLI parsing for the v1alpha1 fixture command;
- stdlib build backend smoke coverage;
- fixture hash/root/trust-key binding checks over committed allow/deny bundles;
- fixture-specific CloudEvent canonical hash and Ed25519 checks over committed
  decision/outcome events;
- generated golden TRACE Level 0, Agent Manifest, SpendGuard artifact, and
  verification-report outputs;
- private env-gated make harness; and
- `make verify-agentrust` smoke/report output for the committed fixtures.

This decision does not approve the implementation for upstream Community
submission, verified trust claims, live evidence verification, production key
distribution, or general-purpose artifact verification.

Per the `AGENTS.md` Staff+ Expert Recovery Cycle rule, the fifth failed Task
review is resolved by this decision. Do not run a sixth Task review for the same
blocker set. The next workflow stage is Final/submission, with the action list
below.

## Deduplicated Blockers

The five review rounds reported four blocker classes:

1. Cryptographic verification depth: the current recovery pass includes
   fixture-specific signed-event checks, but it still does not satisfy the full
   v1alpha1 contract for all phase ownership, event-type binding, trust
   authorization, key-window, schema, and generated-package checks.
2. Negative test integrity: make-only mutations are deterministic/canned and
   include direct diagnostic emission paths, so they prove report plumbing more
   than validator behavior.
3. Report truthfulness: aggregate report entries can be marked `pass` without
   proof that the corresponding package/public-surface guard or parser matrix
   fully ran against the required surfaces.
4. Local safety and shipping guard depth: Linux dirfd/no-follow path safety,
   output commit hardening, source/artifact lane isolation, and final
   wheel/sdist/package/public-surface scans are incomplete.

## Root Cause

The Task implementation mixed two different milestones:

- the approved product scope is fixture-only and docs-first; but
- the Task acceptance plan evolved into a production-grade verifier and package
  guard contract.

The result is a fixture renderer/verifier smoke harness being judged against a
broader conformance bar. The fixture MVP is useful, but the current report and
public docs must not present it as full signature-verification or artifact-lane
conformance.

## Scope Boundary For Stage Advance

The stage may advance after recording this recovery decision and keeping public
surfaces explicit that:

- AgenTrust remains planned / not yet community-approved;
- the current command is fixture-only;
- generated outputs are golden fixture artifacts signed with committed test
  keys; and
- full v1alpha1 verifier conformance, real negative mutations, Linux dirfd path
  hardening, and final package/public-surface guard are deferred work.

No additional implementation is required before stage advance for the
architecture/spec role.

## Must-Have vs Deferred Classification

Must-have to close Task stage, now accepted:

1. Record this recovery decision in the repository.
2. Keep the public scope fixture-only and docs-first.
3. Verify committed vectors still replay.
4. Verify the public allow/deny fixture command path and generated golden
   outputs.
5. Verify `make verify-agentrust` succeeds for the fixture smoke/report lane.
6. Verify package smoke coverage keeps private make/internal modules and
   `docs/specs/**` out of wheel/sdist surfaces.
7. Keep public docs free of upstream approval, live verification, production
   trust, hardware, CMCP/MCP gateway, and higher TRACE-level claims.

Deferred, but required before any upstream Community submission or stronger
trust claim:

1. Expand the current fixture-specific signed-event verification into full
   v1alpha1 verifier conformance, including event-type binding, trust
   authorization, key-window checks, schema ownership, and generated-package
   validation.
2. Replace canned negative diagnostics with mutated fixture material that flows
   through the same verifier path as success fixtures.
3. Make the aggregate make report truth-preserving: each `pass` entry must
   correspond to an executed check. In particular, `package_guard_surfaces` must
   not be marked pass until the package guard actually runs.
4. Implement Linux dirfd/no-follow path safety, output commit hardening, and
   explicit unsupported-platform setup failures.
5. Implement final artifact-lane guard scans over wheel/sdist names and bytes,
   installed metadata, entry scripts, public help/docs, generated outputs, make
   reports, and declared context archives.
6. Wire non-root Ubuntu 24.04 or Debian 12 CI on a supported local filesystem
   for the full conformance lane.

Out of scope for v1alpha1 unless a new spec is approved:

1. Live verification.
2. Historical replay and replay-cache protection.
3. TRACE Level 1/2, hardware attestation, SLSA, certification, or compliance
   claims.
4. Signed Agent Manifest trust.
5. CMCP/MCP gateway integration.
6. Production key distribution.

## Deferred Work

Before any upstream submission or stronger trust claim, implement the deferred
conformance track:

1. Expand signed-event verification from the current fixture-specific checks to
   the whole v1alpha1 fixture contract, including event-type binding, trust
   authorization, key-window checks, schema ownership, and generated-package
   validation.
2. Replace canned negative diagnostics with mutated fixture material that flows
   through the same verifier path as successful fixtures.
3. Make the aggregate report truth-preserving: every `pass` entry must be backed
   by an executed check; otherwise mark the check failed or do not include it in
   the approved schema.
4. Add Linux dirfd/no-follow path safety, output commit hardening, and explicit
   non-Linux setup failure behavior.
5. Add final artifact-lane package guard scans over wheel/sdist metadata, public
   package help/docs, generated outputs, public docs, and declared context
   archives.
6. Wire non-root supported-Linux CI for the full conformance lane.

## Final/Submission Stage Actions

Exact remaining actions for the next stage:

1. Include this file in the final change set and cite it as the Task recovery
   disposition.
2. Re-run the fixture-MVP checks sequentially, not in parallel, because the
   tests and make target both clean `integrations/agentrust/out`:

   ```bash
   python3 docs/specs/check-agentrust-vectors.py
   PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust
   rm -rf integrations/agentrust/out
   PYTHON=python3 PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 make verify-agentrust
   ```

3. Re-run a package smoke inspection from the stdlib backend and confirm wheel
   and sdist exclude `_verify_internal.py`, `_make_verify.py`, and
   `docs/specs/**`.
4. Re-run the public forbidden-claim scan over the three public AgenTrust docs:

   ```bash
   rg -n "agentrust verified|trace[ -]?(level[ -]?)?[12]|hardware attestation|signed manifest trust|cmcp|mcp gateway|live verification|historical replay|replay-cache|production (trust|key distribution)|slsa[_ -]?(level[_ -]?)?1|slsa[_ -]?l1|certified|compliant|attested|AgenTrust Verified|TRACE Level 1|TRACE Level 2|Community approved|community-approved" \
     docs/integrations.md \
     docs/site/docs/integrations/agentrust.md \
     docs/site-v2/src/content/docs/docs/integrations/agentrust.mdx
   ```

   Expected result for this fixture-MVP stage is no matches.
5. Clean generated outputs before final review or commit:

   ```bash
   rm -rf integrations/agentrust/out build dist
   find . -maxdepth 1 -name '*.egg-info' -exec rm -rf {} +
   git status --short
   ```

6. Run the Final review gate against the final change set:

   ```bash
   CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh final branch:main --project-dir "$PWD"
   ```

7. If the Final/submission stage means internal repo closeout only, submit the
   fixture-only MVP with this recovery scope and do not claim upstream Community
   approval.
8. If the Final/submission stage means actual upstream AgenTrust submission,
   complete the deferred submission blockers above first, then rebuild the
   upstream submission package from those verified artifacts.

## Verification Recorded During Recovery

The following fixture-MVP checks passed locally on the recovery pass:

```bash
python3 docs/specs/check-agentrust-vectors.py
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust
make verify-agentrust
```

These commands verify the current fixture-MVP behavior only. They are not
evidence that the deferred conformance track is complete.
