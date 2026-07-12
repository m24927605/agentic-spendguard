# AgenTrust submission checklist

Status: draft; formalized in Slice 14 (`Promotion/docs`) of
`agentrust-full-conformance-plan-v1alpha1.md`. Records externally observable
submission facts and the acceptance conditions for any future tier request.
Nothing in this file claims upstream approval beyond what the linked upstream
record states.

## Community tier — achieved

- `agentrust-io/integrations` PR #30 approved for **community tier** by the
  upstream maintainer on 2026-07-12:
  <https://github.com/agentrust-io/integrations/pull/30>
- Basis recorded in the upstream review: schema-valid manifest; runs against
  released packages (upstream rule 1); `integrates_with` narrowed to `[trace]`
  to avoid an unverifiable cMCP claim (rule 2); README discloses the
  `TR-SIG-005 UNVERIFIED` unsigned-payload limitation (rule 4); conformance CI
  green on Python 3.11–3.13.
- Conformance workflow lives at the integrations-repo root
  (`.github/workflows/spendguard-conformance.yml`); the
  `integrations/_template/` sub-directory placement is not discovered by
  GitHub Actions (reported upstream in PR #30).

## Verified tier — preconditions (from the upstream review, non-blocking notes)

Do not request the Verified tier until both hold, with evidence:

1. **Graded signed path.** The level-0 pass grades the unsigned payload only:
   released `agentrust-trace-tests` 0.1.0 rejects a plain trace record carrying
   a top-level `signature` (anti-downgrade) and verifies signatures only on
   cMCP RuntimeClaim envelopes; plain records fail closed at level 1+. A
   Verified request needs either upstream trace-tests growing plain-record
   signature grading, or SpendGuard emitting a form the suite grades. Wrapping
   output as a cMCP envelope without running cMCP is prohibited (rule 4).
2. **Real build provenance.** `build_provenance.slsa_level` in the current
   records is fixture data and level 0 does not grade it. Any level-1+ claim
   requires real provenance behind that field.

Additional constraints:

- The tier flip is a deliberate upstream maintainer decision, not automatic on
  green CI; the request must include exact reproduction steps and cite the
  evidence for both preconditions.
- Internal prerequisite: the full conformance gate of
  `agentrust-full-conformance-plan-v1alpha1.md` (Slices 0–14) passes before any
  claim upgrade in public docs, per that plan's implementation rules.
