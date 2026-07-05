# AgenTrust Full Conformance Implementation Plan v1alpha1

Status: Draft docs-first plan.

## Objective

Complete the deferred AgenTrust v1alpha1 conformance work after the fixture MVP.
The end state is a truth-preserving `make verify-agentrust` full gate that runs
on supported Linux CI and produces enough evidence for upstream Community
submission preparation without claiming upstream approval.

This plan intentionally does not start from production live verification. The
track remains fixture-only.

## Baseline

Baseline commit for this track:

```text
0bd6201 feat: add AgenTrust fixture MVP
```

Current baseline capabilities:

- public CLI parser and help;
- committed allow/deny fixture verification smoke;
- committed golden output byte comparison;
- private env-gated make harness;
- package smoke guard for private module exposure;
- fixture-only GitHub Actions gate.

Known gaps from the recovery summary:

- signed-byte verification is fixture-specific and incomplete;
- negative diagnostics are currently canned;
- make report entries can overstate checks;
- Linux path/output safety is not conformance-grade;
- package guard does not scan all required surfaces;
- CI is fixture smoke, not full conformance.

## Implementation Rules

- Work in small slices.
- Keep public docs fixture-only until the final conformance gate passes.
- Do not let runtime verifier code read make-only evidence files.
- Do not mark report entries `pass` unless the check actually ran.
- Do not weaken CLI parser behavior while adding deeper runtime checks.
- Do not add unpinned dependencies or new build backends.
- Do not make macOS success a conformance signal.
- Do not claim upstream approval in commit messages, docs, package metadata, or
  PR bodies.
- Every implementation slice must cite at least one exact section heading from
  `docs/specs/agentrust-integration-spec-review-bundle-v1alpha1.md`.
- Every slice must have at least one positive test and one failure probe or make
  evidence row before it can be marked complete.
- Before Slice 13, `make verify-agentrust-full` is allowed to fail with an
  explicit pending/full-gate diagnostic because later slices are incomplete.
  Intermediate slices prove completion with focused tests or focused make lanes,
  not by requiring the full target to be green.
- Focused make lanes are introduced before they are used:
  `make verify-agentrust-preflight`, `make verify-agentrust-output-lane`,
  `make verify-agentrust-negative-lane`, `make verify-agentrust-report-lane`,
  and `make verify-agentrust-artifact-lane`.
- The full conformance make path must run artifact/vector/lock/schema preflight
  before invoking any runtime verifier subprocess. Earlier unit tests are
  non-conformance probes and do not count as a green full gate.

## Traceability Matrix

Required columns:

```text
slice,review_bundle_section,owned_files,tests,command_or_job,artifacts,report_or_evidence_row,failure_probe
```

| slice | review_bundle_section | owned_files | tests | command_or_job | artifacts | report_or_evidence_row | failure_probe |
|---|---|---|---|---|---|---|---|
| 0 Track documents | `Authority`, `Gate Clarifications` | `docs/specs/agentrust-full-conformance-*.md`, docs indexes | review gate output | `codex-review spec`, `codex-review plan` | spec/plan docs | review gate JSON | missing matrix row rejects plan |
| 1 Target migration | `Private And Make Harness`, `Generated Outputs And Make Continuation` | `Makefile`, `_make_verify.py` | make target tests | smoke/full/focused make lane tests | smoke/full/focused target outputs | target availability evidence | missing full or focused lane before dependent slices |
| 2 Runtime read boundary | `Validation Contract`, `Evidence Payload` | `verify.py`, file-open instrumentation helpers | runtime boundary tests | `python3 -m unittest ...read_boundary...` | open-attempt log/hash | runtime read-boundary evidence row | attempted open of vectors, locks, manifests, expected outputs |
| 3 Private harness | `Private And Make Harness`, `Command State Matrix` | `_verify_internal.py`, `_make_verify.py`, `cli.py` | private parser/source gate tests | `python3 -m unittest ...private...` | private diagnostics | `internal_guard`, `private_parser_matrix` | missing env, installed package realpath, bad private grammar |
| 4 Public parser | `Public CLI`, `Command State Matrix` | `cli.py`, `diagnostics.py` | `test_public_cli_parser.py`, `test_diagnostics.py` | parser and diagnostics unittest commands | diagnostic stderr/stdout bytes | `public_parser_matrix` | unsupported live flag, equals-form cases, bad diagnostic key order |
| 5 Artifact preflight | `Validation Contract`, `Evidence Payload`, `Gate Clarifications` | `Makefile`, `_make_verify.py`, `c14n.py`, `jsonio.py`, `schemas.py`, `package_contract.py`, `docs/specs/check-agentrust-vectors.py`, `integrations/agentrust/requirements-v1alpha1.lock` | `test_c14n_jsonio.py`, `test_artifact_preflight.py`, `test_artifact_preflight_order.py` | `make verify-agentrust-preflight` and vector checker | generated schema/package constants and pinned artifact hashes | `artifact_preflight_envelope` | payload drift, command evidence mismatch, proto/vector/checker/lock/manifest drift |
| 6 Linux path/output | `Local Security And Output` | `paths.py`, `output.py`, `diagnostics.py`, `.github/workflows/agentrust-path-safety.yml` | `test_linux_path_safety.py`, `test_output_commit.py` | `agentrust-path-safety.yml` Ubuntu slice job | temp/final output dirs | Linux preflight evidence | symlink output and final-exists cases |
| 7 Bundle intake | `Validation Contract` | `bundle.py`, `fixtures.py`, `verify.py` | bundle/file-set tests | `python3 -m unittest discover -s tests/agentrust` | parsed fixture bundle state | `fixture_identity` | wrong file set and raw hash mismatch |
| 8 Signed event/trust | `Validation Contract`, `Evidence Payload` | `events.py`, `trust.py`, `verify.py` | event/trust tests | `python3 -m unittest discover -s tests/agentrust` | verified signed events | `event_type_binding` | wrong key, bad signature, key-window cases |
| 9 Lifecycle/bindings/output | `Generated Outputs And Make Continuation` | `lifecycle.py`, `bindings.py`, `trace.py`, `manifest.py`, `verify.py` | `test_lifecycle_bindings.py`, `test_generated_outputs.py` | focused unittest generated-output lane | TRACE, Agent Manifest, artifacts report | generated output validation evidence, `manifest_issuer_variant` | lifecycle mismatch and output validation failure |
| 10 Negative mutations | `Private And Make Harness`, `Validation Contract` | `mutations.py`, `_make_verify.py`, `_verify_internal.py`, `verify.py` | `test_real_negative_mutations.py` | focused unittest and negative make lane | mutation bundle copies | `negative_tests` | mutation that would pass without real byte change |
| 11 Make report | `Generated Outputs And Make Continuation`, `Evidence Payload` | `report.py`, `_make_verify.py` | `test_make_verify_target.py`, `test_make_verify_report.py` | focused unittest make-report lane | aggregate report | aggregate report rows, including `internal_guard`, `manifest_issuer_variant`, `private_parser_matrix` | malformed stderr and missing report |
| 12 Package guard | `Gate Clarifications` | `package_guard.py`, `packaging/agentrust_build_backend.py`, `_make_verify.py` | `test_build_backend.py`, `test_package_guard.py`, `test_artifact_lane.py` | artifact-lane command from `_make_verify.py` | wheel, sdist, metadata, scripts | `package_guard_surfaces` | PEM bytes, private surface, unsafe archive member |
| 13 Full CI | `Gate Clarifications` | `.github/workflows/agentrust-conformance.yml`, `.github/workflows/agentrust-fixture.yml`, `Makefile`, docs if target names change | workflow run | `verify-agentrust-ubuntu-24.04` | GitHub Actions check | CI check conclusion | unsupported platform/non-root preflight failure |
| 14 Promotion/docs | `Scope`, `Gate Clarifications` | `Makefile`, `docs/integrations.md`, `docs/site/docs/integrations/agentrust.md`, `docs/site-v2/src/content/docs/docs/integrations/agentrust.mdx`, `pyproject.toml`, `docs/specs/agentrust-operator-preview-notes-v1alpha1.md`, `docs/specs/agentrust-submission-checklist-v1alpha1.md` | final review | promoted `make verify-agentrust`, post-promotion `verify-agentrust-ubuntu-24.04`, `codex-review final` | submission packet docs | final gate evidence | forbidden public claim scan |

Plan review must reject this plan if any slice row loses its review-bundle
section, owned files, tests, command/job, artifacts, evidence row, or failure
probe.

## Slice 0 - Track Documents and Review Gates

Files:

- `docs/specs/agentrust-full-conformance-spec-v1alpha1.md`
- `docs/specs/agentrust-full-conformance-plan-v1alpha1.md`
- `docs/README.md`
- `docs/integrations.md`

Acceptance:

- New spec and plan clearly separate fixture MVP from full conformance.
- Docs index links the new full conformance track.
- Governance evidence matrix still says no Community approval.
- Spec review gate passes or Staff+ Expert Recovery has executed and advanced
  the track to Plan gate.
- Plan review gate passes before Task work begins.

Verification:

```bash
git diff --check -- docs/specs/agentrust-full-conformance-spec-v1alpha1.md docs/specs/agentrust-full-conformance-plan-v1alpha1.md docs/README.md docs/integrations.md
CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh spec docs/specs/agentrust-full-conformance-spec-v1alpha1.md --project-dir "$PWD"
CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh plan docs/specs/agentrust-full-conformance-plan-v1alpha1.md --project-dir "$PWD"
```

## Slice 1 - Makefile Target Migration

Files:

- `Makefile`
- `spendguard_agentrust/_make_verify.py`
- `tests/agentrust/test_make_verify_target.py`

Acceptance:

- `make verify-agentrust-fixture` remains the fast smoke target.
- `make verify-agentrust-full` exists before any later slice depends on it.
- Focused lane targets exist before dependent slices use them.
- `make verify-agentrust` remains fixture smoke until Slice 14 promotion.
- Full-target stubs fail non-success until their owned checks are implemented,
  so no incomplete full target can look green.
- Target tests prove the smoke/full/promotion names are distinct.
- The expected pending full-target raw diagnostic before Slice 13 is produced by
  a direct make-internal Python harness command, not by parsing standard `make`
  stderr. The raw harness exits 6, writes one compact setup/final diagnostic JSON
  document to stderr, and writes empty stdout.
- `make verify-agentrust-full` remains a standard make target: before Slice 13 it
  must fail non-successfully and keep stdout empty, but tests must not require
  GNU/BSD make to preserve the harness exit code or keep stderr JSON-only.
  Make-summary stderr is reserved for exit 1 aggregate failures after report
  generation exists.

Verification:

```bash
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p '*make*target*.py'
PYTHON=python3 PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 make verify-agentrust-fixture
set +e; PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m spendguard_agentrust._make_verify verify-full > /tmp/agentrust-full.raw.out 2> /tmp/agentrust-full.raw.err; raw_status=$?; set -e; test "$raw_status" -eq 6; test ! -s /tmp/agentrust-full.raw.out; python3 -m json.tool /tmp/agentrust-full.raw.err >/dev/null
set +e; PYTHON=python3 PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 make verify-agentrust-full > /tmp/agentrust-full.make.out 2> /tmp/agentrust-full.make.err; make_status=$?; set -e; test "$make_status" -ne 0; test ! -s /tmp/agentrust-full.make.out; rg '"exit_code":6|"deferred_fixture_mvp"|Error' /tmp/agentrust-full.make.err >/dev/null
```

## Slice 2 - Runtime Read-Boundary Instrumentation

Files:

- `spendguard_agentrust/verify.py`
- `spendguard_agentrust/_verify_internal.py`
- `tests/agentrust/test_runtime_read_boundary.py`

Acceptance:

- Tests instrument verifier file opens for public success, private success, and
  private negative runs.
- Runtime verifier opens only review-bundle allowed runtime inputs.
- Runtime verifier does not open vectors, locks, fixture manifests, expected
  outputs, package guard inputs, or public-surface scan fixtures.
- A deliberate attempted open of a make-only input fails the test.
- The slice records runtime read-boundary evidence separately from package guard.
- This is an invariant, not a one-time proof. Slices 7 through 10 must rerun the
  read-boundary regression after adding bundle intake, signed-event handling,
  generated outputs, and private mutation runs.

Verification:

```bash
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p '*read_boundary*.py'
```

## Slice 3 - Private Harness and Source-Lane Gate

Files:

- `spendguard_agentrust/_verify_internal.py`
- `spendguard_agentrust/_make_verify.py`
- `spendguard_agentrust/cli.py`
- `tests/agentrust/test_private_harness.py`

Acceptance:

- `_verify_internal` requires the review-bundle private environment gate.
- Private parser matrix covers success grammar, negative grammar, invalid
  hidden paths, invalid mutation enums, and public-only flags.
- Source-checkout realpath checks reject installed wheel/sdist contexts.
- Private runs execute in fresh source-lane subprocesses before negative
  mutations depend on them.
- Source-lane tests prove `_verify_internal` is not reachable without the
  private gate.
- Make report evidence for `internal_guard` and `private_parser_matrix` is added
  in Slice 11 after truth-preserving report generation exists.

Verification:

```bash
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p '*private*harne*.py'
```

## Slice 4 - Diagnostics and Parser Preservation

Files:

- `spendguard_agentrust/diagnostics.py`
- `spendguard_agentrust/cli.py`
- `tests/agentrust/test_public_cli_parser.py`
- `tests/agentrust/test_diagnostics.py`

Acceptance:

- Current public parser contract remains byte-stable.
- Diagnostic schema key order remains:
  `schema,exit_code,phase,code,message,json_pointer`.
- Diagnostic precedence is centralized and tested independently from bundle
  parsing.
- Parser tests cover unsupported capabilities, equals forms, duplicates,
  missing values, required flags, bad paths, manifest issuer grammar, and
  non-subprocess-only token cases.

Verification:

```bash
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p 'test_public_cli_parser.py'
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p 'test_diagnostics.py'
```

## Slice 5 - Artifact Preflight, SG-C14N, JSON IO, Schema Ownership

Files:

- `spendguard_agentrust/c14n.py`
- `spendguard_agentrust/jsonio.py`
- `spendguard_agentrust/schemas.py`
- `spendguard_agentrust/package_contract.py`
- `docs/specs/check-agentrust-vectors.py`
- `integrations/agentrust/requirements-v1alpha1.lock`
- `tests/agentrust/test_c14n_jsonio.py`
- `tests/agentrust/test_artifact_preflight.py`
- `tests/agentrust/test_artifact_preflight_order.py`

Acceptance:

- Duplicate JSON keys are rejected.
- SG-C14N uses bytewise sorted object keys.
- Integer grammar is exact: `0|-?[1-9][0-9]*`.
- `-0`, fractions, exponents, NaN, Infinity, and out-of-range integers fail.
- No implicit Unicode normalization occurs.
- Schema-source SHA and generated schema constants match the review bundle.
- Proto, CloudEvent vector, checker script, lock file, fixture manifest,
  generated-output manifest, and expected-output manifests are SHA-pinned before
  runtime verifier execution.
- Make decodes and verifies the review-bundle `PAYLOAD_B64`, decoded evidence
  JSON SHA, unique 19 artifact paths, replayed command argv/stream hashes, and
  `embedded_fresh_replay=pass`.
- Drift failures map to the review-bundle diagnostics, including
  `schema_source_mismatch` and `package_probe_drift`.
- Public/private verifier runtime does not read make-only vectors, locks,
  manifests, or expected outputs.
- A make-level order probe proves artifact/vector/lock/schema preflight runs
  before any verifier subprocess is spawned.

Verification:

```bash
python3 docs/specs/check-agentrust-vectors.py
PYTHON=python3 PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 make verify-agentrust-preflight
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p '*preflight*order*.py'
```

## Slice 6 - Linux Path Safety and Output Commit

Files:

- `spendguard_agentrust/paths.py`
- `spendguard_agentrust/output.py`
- `spendguard_agentrust/diagnostics.py`
- `.github/workflows/agentrust-path-safety.yml`
- `tests/agentrust/test_linux_path_safety.py`
- `tests/agentrust/test_output_commit.py`

Acceptance:

- Public paths are repository-relative POSIX literals before filesystem access.
- Absolute paths, repeated separators, backslashes, `.`, `..`, NUL, and hidden
  public segments fail.
- Existing ancestors are checked with `lstat` and no symlink following.
- Linux conformance uses dirfd/openat/no-follow checks.
- Output commit writes to a temp directory, readbacks bytes, and commits with
  `renameat2(RENAME_NOREPLACE)`.
- No copy/link/replace fallback exists for the conformance path.
- Unsupported Linux environments fail with explicit diagnostics.

Verification:

```bash
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p '*path*.py'
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p '*output*commit*.py'
gh workflow run agentrust-path-safety.yml --ref "$(git branch --show-current)"
RUN_ID=$(gh run list --workflow agentrust-path-safety.yml --branch "$(git branch --show-current)" --commit "$(git rev-parse HEAD)" --event workflow_dispatch --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch "$RUN_ID" --exit-status
```

Slice completion requires focused Ubuntu 24.04 evidence from these tests or an
equivalent slice job. Full-target success is not required until Slice 13. macOS
unit tests may check parser behavior, but they must report unsupported rather
than passing as conformance evidence.

## Slice 7 - Bundle Intake and Phase Ownership

Files:

- `spendguard_agentrust/bundle.py`
- `spendguard_agentrust/fixtures.py`
- `spendguard_agentrust/verify.py`
- `tests/agentrust/test_bundle_intake.py`

Acceptance:

- `bundle.json` index parse owns only the ordering/index phase.
- File-set, forbidden-file, media-type/profile, raw SHA, privacy scan, JSON
  syntax, duplicate-key, canonical JSON, and closed-schema checks have distinct
  phase ownership.
- Allow requires `outcome.json`; deny requires it absent.
- Wrong file-set or hash failures cannot accidentally reach trust or output
  phases.

Verification:

```bash
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust
```

## Slice 8 - Signed Event and Trust Verification

Files:

- `spendguard_agentrust/events.py`
- `spendguard_agentrust/trust.py`
- `spendguard_agentrust/verify.py`
- `tests/agentrust/test_security_recovery.py`
- `tests/agentrust/test_signed_events.py`
- `tests/agentrust/test_trust_store.py`

Acceptance:

- Decision and outcome events validate `data_b64` against `data_json`.
- Canonical signed bytes are reconstructed exactly.
- `canonical_bytes_sha256` is checked before signature verification.
- Trust store key selection checks key id, algorithm, producer/tenant/subject
  authorization, and time window.
- Ed25519 verification fails closed.
- Event type and payload schema are bound.

Verification:

```bash
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust
```

## Slice 9 - Lifecycle and Artifact Bindings

Files:

- `spendguard_agentrust/lifecycle.py`
- `spendguard_agentrust/bindings.py`
- `spendguard_agentrust/trace.py`
- `spendguard_agentrust/manifest.py`
- `spendguard_agentrust/verify.py`
- `tests/agentrust/test_lifecycle_bindings.py`
- `tests/agentrust/test_generated_outputs.py`

Acceptance:

- Allow and deny lifecycle derivation is deterministic.
- Decision/outcome/run/tenant/producer fields are coherent.
- Policy, tool catalog, build provenance, model identity, and evidence-bundle
  root bindings derive only from signed or explicitly labeled fixture inputs.
- Operator-supplied context remains labeled and is never promoted into signed
  SpendGuard evidence.
- Generated TRACE and Agent Manifest outputs validate before commit.
- Phase-11 output coverage includes `trace-level0.json`, `agent-manifest.json`,
  `spendguard-artifacts.json`, `verification-report.json`, selected-output order,
  readback, final commit, byte-for-byte golden comparison, and expected aggregate
  failure mapping for golden mismatch.

Verification:

```bash
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p '*generated*output*.py'
PYTHON=python3 PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 make verify-agentrust-output-lane
```

## Slice 10 - Real Negative Mutations

Files:

- `spendguard_agentrust/mutations.py`
- `spendguard_agentrust/_make_verify.py`
- `spendguard_agentrust/_verify_internal.py`
- `spendguard_agentrust/verify.py`
- `tests/agentrust/test_real_negative_mutations.py`

Acceptance:

- Make copies fixture material into the run root before mutation.
- Each mutation changes real bytes and runs through the same verifier path as
  success fixtures.
- Mutations reach intended phases:
  - `mutated_signature`: phase-9 signature invalid.
  - `wrong_key`: phase-9 wrong public key.
  - `mutated_policy_hash`: phase-10 binding mismatch.
  - `missing_trace_signature`: phase-11 TRACE package validation failure.
- Each mutation asserts exact exit code, diagnostic code, JSON pointer, empty
  stdout, single diagnostic stderr, and selected output absence according to the
  review bundle.
- Mutation output roots are scanned for absence of selected outputs.
- Symlinks under mutation output roots count as present.
- Canned direct diagnostic paths are removed.
- Tests prove the previous canned-direct diagnostic path is impossible.

Verification:

```bash
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p '*negative*mutation*.py'
PYTHON=python3 PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 make verify-agentrust-negative-lane
```

## Slice 11 - Truth-Preserving Make Report

Files:

- `spendguard_agentrust/report.py`
- `spendguard_agentrust/_make_verify.py`
- `tests/agentrust/test_make_verify_target.py`
- `tests/agentrust/test_make_verify_report.py`

Acceptance:

- Every `pass` entry corresponds to an executed check.
- `deferred_fixture_mvp` is not a passing full-gate status.
- Exit 0 stdout remains exact.
- Exit 1 aggregate failures emit one compact summary JSON on stderr.
- Exit 6 setup/final aborts emit one compact diagnostic JSON on stderr.
- Report top-level keys, nested key order, LF behavior, and output absence
  fields match the review bundle.
- Required `conformance_checks` rows are first-class:
  `artifact_preflight_envelope`, `event_type_binding`, `fixture_identity`,
  `internal_guard`, `manifest_issuer_variant`, `package_guard_surfaces`,
  `private_parser_matrix`, and `public_parser_matrix`.
- Unexpected signal, malformed stderr, and missing report cases are tested.

Verification:

```bash
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p '*make*report*.py'
PYTHON=python3 PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 make verify-agentrust-report-lane
```

## Slice 12 - Final Artifact Package Guard

Files:

- `spendguard_agentrust/package_guard.py`
- `packaging/agentrust_build_backend.py`
- `spendguard_agentrust/_make_verify.py`
- `tests/agentrust/test_build_backend.py`
- `tests/agentrust/test_package_guard.py`
- `tests/agentrust/test_artifact_lane.py`

Acceptance:

- Build, source-test, wheel install, sdist rebuild, and sdist install use
  separate clean venvs.
- The artifact lane is invoked by `make verify-agentrust-full` and records
  captured realpaths for source tree, wheel, sdist, rebuilt sdist, venv entry
  scripts, installed metadata, and scanned public docs.
- Package dependency pins come from `integrations/agentrust/requirements-v1alpha1.lock`.
- Wheel/sdist are selected by captured realpath, not globs after cleanup.
- Wheel and sdist member names are scanned in byte order.
- Unsafe archive paths, symlink entries, nested archives, sourcemaps, private
  PEM bytes, private modules, `docs/specs/**`, forbidden claims, generated
  entry scripts, metadata, public help/docs, generated outputs, make reports,
  diagnostics, and context tar entries are scanned.
- Security scan units include absolute sensitive paths in diagnostics/logs,
  environment influence outside documented variables, raw PEM/secret bytes
  before binary/text classification, and installed public package docs/help.
- Setup package guard runs before runtime verifier subprocesses. Final
  forbidden-claim/report/generated-output scans run after
  `make verify-agentrust-full` has produced outputs, aggregate report, and copied
  diagnostics.
- Exact public scan units are:
  - wheel and sdist archive member names and bytes;
  - installed `METADATA`, `WHEEL`, `entry_points.txt`, `RECORD`;
  - generated venv `bin/spendguard*` scripts;
  - public CLI help stdout;
  - `docs/integrations.md`;
  - `docs/site/docs/integrations/agentrust.md`;
  - `docs/site-v2/src/content/docs/docs/integrations/agentrust.mdx`;
  - generated allow/deny output files;
  - aggregate make report and copied diagnostics;
  - declared context tar entries.
- Installed public package proves `_verify_internal` and `_make_verify` are not
  public surfaces.

Verification:

```bash
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p 'test_build_backend.py'
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p '*package*guard*.py'
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust -p '*artifact*lane*.py'
PYTHON=python3 PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 make verify-agentrust-artifact-lane
```

## Slice 13 - Full Conformance CI

Files:

- `.github/workflows/agentrust-conformance.yml`
- `.github/workflows/agentrust-fixture.yml`
- `Makefile`
- docs if target names change

Acceptance:

- Fixture smoke CI remains available for fast PR feedback.
- Full conformance CI pins `ubuntu-24.04` and names the job
  `verify-agentrust-ubuntu-24.04`.
- Full CI runs non-root environment preflight before conformance steps.
- Full CI fails or reports non-success for unsupported filesystem/runtime
  conditions.
- If GitHub-hosted `ubuntu-24.04` fails the review-bundle filesystem preflight
  because of overlayfs, root, network/FUSE, virtual/idmapped, or user namespace
  conditions, this slice must configure a supported self-hosted Ubuntu 24.04
  runner before it can be green. Hosted runner success is evidence only if the
  preflight row passes.
- Full CI runs:
  - `python3 docs/specs/check-agentrust-vectors.py`
  - stdlib `unittest`
  - clean artifact lane
  - full `make verify-agentrust-full` before promotion
  - public forbidden-claim scan
- Debian 12 support is optional local/self-hosted unless a supported runner is
  explicitly configured.

Verification:

```bash
BRANCH=$(git branch --show-current)
SHA=$(git rev-parse HEAD)
gh workflow run agentrust-conformance.yml --ref "$BRANCH"
RUN_ID=$(gh run list --workflow agentrust-conformance.yml --branch "$BRANCH" --commit "$SHA" --event workflow_dispatch --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch "$RUN_ID" --exit-status
JOB_ID=$(gh run view "$RUN_ID" --json jobs --jq '.jobs[] | select(.name=="verify-agentrust-ubuntu-24.04") | .databaseId')
gh run view "$RUN_ID" --job "$JOB_ID" --log
```

or PR-triggered GitHub Actions once the workflow is introduced.

## Slice 14 - Promote Full Gate and Submission Packet

Files:

- `Makefile`
- `docs/integrations.md`
- `docs/site/docs/integrations/agentrust.md`
- `docs/site-v2/src/content/docs/docs/integrations/agentrust.mdx`
- `pyproject.toml`
- `docs/specs/agentrust-operator-preview-notes-v1alpha1.md`
- `docs/specs/agentrust-submission-checklist-v1alpha1.md`

Acceptance:

- `make verify-agentrust` is the full conformance gate.
- `make verify-agentrust-fixture` remains as a smoke target and is documented as
  non-conformance-only.
- `make verify-agentrust-full` remains available as the implementation target or
  becomes an alias of `make verify-agentrust`.
- The `verify-agentrust-ubuntu-24.04` GitHub Actions job is rerun after
  promotion and proves the promoted `make verify-agentrust` target, not only the
  pre-promotion `make verify-agentrust-full` target.
- Public docs may say "ready for upstream Community submission preparation" only
  after all gates pass.
- Public docs still do not say upstream approved unless upstream approval has
  actually happened.
- Forbidden-claim scan covers the exact public scan units listed in Slice 12 and
  the submission packet docs added in this slice.
- Final review gate passes against the merge base.

Verification:

```bash
python3 docs/specs/check-agentrust-vectors.py
PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 python3 -m unittest discover -s tests/agentrust
PYTHON=python3 PYTHONPATH="$PWD" PYTHONNOUSERSITE=1 make verify-agentrust
BRANCH=$(git branch --show-current)
SHA=$(git rev-parse HEAD)
gh workflow run agentrust-conformance.yml --ref "$BRANCH"
RUN_ID=$(gh run list --workflow agentrust-conformance.yml --branch "$BRANCH" --commit "$SHA" --event workflow_dispatch --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch "$RUN_ID" --exit-status
JOB_ID=$(gh run view "$RUN_ID" --json jobs --jq '.jobs[] | select(.name=="verify-agentrust-ubuntu-24.04") | .databaseId')
gh run view "$RUN_ID" --job "$JOB_ID" --log | rg 'make verify-agentrust([^_-]|$)'
git diff --check origin/main...HEAD
CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh final branch:main --project-dir "$PWD"
```

Final PR and release text must include the traceability matrix summary and pass
the forbidden-claim scan before merge.

## Review Gate Sequence

Before implementation:

```bash
CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh spec docs/specs/agentrust-full-conformance-spec-v1alpha1.md --project-dir "$PWD"
CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh plan docs/specs/agentrust-full-conformance-plan-v1alpha1.md --project-dir "$PWD"
BASELINE=$(git rev-parse HEAD)
```

During implementation:

```bash
CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh task "$BASELINE" --project-dir "$PWD"
```

Final:

```bash
CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh final branch:main --project-dir "$PWD"
```

If a stage reaches the fifth failed review, use the Staff+ Expert Recovery Cycle
from `AGENTS.md`; after the expert team decides and executes the fix, advance to
the next stage unless the team identifies an external permission or product
decision blocker.

## No Human Decisions Required To Start

Two choices are fixed for this plan:

- Required hosted conformance CI is `ubuntu-24.04`.
- Debian 12 is optional through local/self-hosted execution when filesystem
  preflight passes.

The next human decision is only needed if Debian 12 should become a required
hosted gate and an appropriate runner is available.
