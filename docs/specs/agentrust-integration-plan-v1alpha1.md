# AgenTrust integration implementation plan v1alpha1

Status: Plan stage resolved by Staff+ Expert Recovery Cycle after the approved
Spec gate. Task stage later advanced as a fixture-only docs-first MVP by
`docs/specs/agentrust-task-recovery-summary-v1alpha1.md`; full verifier
conformance remains deferred work.

## Source of truth

Implement only the contract in
`docs/specs/agentrust-integration-spec-review-bundle-v1alpha1.md`. The older
background design in `docs/specs/agentrust-integration-spec-v1alpha1.md` is
non-normative when it differs from the review bundle.

Spec gate evidence already approved:

- Normative scope: fixture-only AgenTrust Community submission.
- Public command: `python -m spendguard_agentrust verify [flags]`.
- Make command: `make verify-agentrust`.
- Embedded evidence replay: `embedded_fresh_replay=pass`.
- Required package pins and artifact hashes come from the spec bundle payload
  and `integrations/agentrust/requirements-v1alpha1.lock`.

Do not add claims for live verification, historical replay, replay-cache
protection, CMCP/MCP gateway integration, hardware attestation, signed manifest
trust, SLSA, certification, compliance, production key distribution, or higher
TRACE levels. Public docs remain "planned / not yet community-approved" until
the implementation and submission are accepted upstream.

## Implementation shape

Add a root-level pure Python package while leaving the Rust workspace unchanged:

- `pyproject.toml`: minimal PEP 517 project for `spendguard-agentrust`, Python
  `>=3.12`, using an in-repo stdlib-only build backend via
  `build-system.requires = []`, `build-backend = "agentrust_build_backend"`,
  and `backend-path = ["packaging"]`. Do not introduce Hatchling, setuptools,
  wheel, flit, poetry-core, or any backend dependency absent from the hashed
  lock. Exact direct runtime pins align to
  `integrations/agentrust/requirements-v1alpha1.lock`:
  `agent-manifest==0.1.1`, `agentrust-trace==0.2.0`,
  `cryptography==49.0.0`, and `jsonschema==4.26.0`. The verification extra or
  make environment installs the lock-pinned probe/test dependencies, including
  `agentrust-trace-tests==0.1.0`, `build==1.5.0`, `pydantic==2.13.4`, and their
  hashed transitives from the lock. No unpinned JSON/schema/security dependency
  is allowed in the conformance lane.
- `spendguard_agentrust/`: public package imported by `python -m
  spendguard_agentrust`.
- `packaging/agentrust_build_backend.py`: stdlib-only wheel/sdist builder used
  by `python -m build --sdist --wheel`; wheels emit only public package files,
  root package metadata, README/license if needed, and no spec/private surfaces.
  Sdists additionally include `pyproject.toml` and the build backend so they are
  rebuildable, and those backend files are scanned as public package text.
- `spendguard_agentrust/_verify_internal.py` and make-only support modules:
  present in the source checkout for `PYTHONPATH=<repo-root>` runs, but excluded
  from wheel, sdist, console scripts, package docs, generated scripts, and public
  help.
- `tests/agentrust/`: focused stdlib `unittest` parser, canonical JSON,
  issuer, fixture, negative, and packaging tests.
- `Makefile`: add `verify-agentrust` and help text. The target delegates to a
  source-checkout make harness and does not rely on editable installs.

The implementation has two lanes:

Source-lane import isolation contract:

- Every source-lane probe, private verifier run, parser/help run, package
  probe, allow/deny fixture run, issuer variant, and negative mutation run
  executes in a fresh Python process via subprocess. No source-lane check may
  call verifier code in-process through `runpy`, `importlib`, cached modules, or
  reused `sys.modules`.
- Pure parser unit tests may call parser functions in-process only for argv
  values that cannot be represented through OS subprocess argv, such as NUL,
  surrogate, or control-character cases. These tests cannot read files, import
  verifier runtime side effects, or count as source-lane conformance runs.
- Make setup performs the source-realpath gate before private verifier work. It
  fails setup with `packaging_guard_failed` for `.pth`, `.egg-link`, editable or
  namespace providers, `build/`, `dist/`, site-packages, wheel, zip/zipimport,
  or any `spendguard_agentrust*` provider/module realpath outside the repo root.

- Source lane: runs private verifier and make harness only from the checkout.
  The make harness sets `PYTHONPATH=<repo-root>` and `PYTHONNOUSERSITE=1`,
  starts a fresh Python process for every probe, public/private verifier run,
  package probe, and package guard invocation, and never reuses `sys.modules`,
  `importlib` caches, or an already-imported `spendguard_agentrust*` module
  between checks. Make setup enumerates every loaded/importable
  `spendguard_agentrust*` provider and fails `packaging_guard_failed` if any
  provider is supplied by `.pth`, `.egg-link`, namespace package segments,
  `build/`, `dist/`, site-packages, wheel files, zip/zipimport, or any realpath
  outside `<repo-root>`. Built artifacts are never imported in the source lane.
  The `build/`, `dist/`, and root `*.egg-info` rejection is a prebuild
  source-lane guard. Final artifact checks delete those paths first, then scan
  only the freshly captured artifact paths produced by that build.
- Final artifact lane: builds wheel/sdist from a lock-pinned build venv with
  `python -m build --no-isolation --sdist --wheel`, installs
  `integrations/agentrust/requirements-v1alpha1.lock` into a clean install venv
  with `pip --require-hashes`, installs the captured wheel with `pip install
  --no-deps`, asserts installed versions match the approved pins, inspects public
  package surfaces, and proves private harness files and forbidden public claims
  are absent. The scan inputs are wheel/sdist member names and bytes, dist metadata
  (`METADATA`, `WHEEL`, `entry_points.txt`, `RECORD`), generated entry-point
  scripts, public package docs/help, generated outputs, the three public docs,
  and declared context tar entries. Guard failures include unsafe archive names,
  symlink entries, nested archives by extension or magic, sourcemaps,
  `docs/specs/**`, this review bundle, private PEM bytes, unallowlisted
  `*.test.ed25519.pem`, absolute sensitive paths, `_verify_internal` public
  exposure, and any forbidden public claim text.

## Work slices

1. Package skeleton and public CLI parser

   Files:
   `pyproject.toml`, `spendguard_agentrust/__init__.py`,
   `spendguard_agentrust/__main__.py`, `spendguard_agentrust/cli.py`,
   `spendguard_agentrust/diagnostics.py`.

   Acceptance:
   - `python -m spendguard_agentrust verify --help` emits the exact help line
     from the spec, exits 0, and reads no application data.
   - Parser precedence matches the review bundle table, including unsupported
     capability lexemes, unsupported equals forms, missing values, duplicates,
     and required flag checks.
   - Parser tests are table-driven and cover raw invalid argv tokens, `verify`
     placement, exact `verify --help` as the only help success path, non-exact
     help failures, all review-bundle precedence examples, unsupported
     capability tokens/equals/pairs, `--tested-against` value consumption,
     unknown flags, duplicate supported flags, supported equals forms, missing
     required flags, invalid supported path literals, and
     `--manifest-issuer` grammar/percent-decoding/length validation order.
   - Public failures emit one compact diagnostic JSON on stderr, empty stdout,
     deterministic key order, and the correct exit code.
   - `diagnostics.py` is the central registry for code family, exit map,
     phase rank, precedence index, message derivation, and RFC 6901 pointer
     handling. Tests assert exact diagnostic schema
     `spendguard-agentrust-diagnostic/v1`, key order
     `schema,exit_code,phase,code,message,json_pointer`, message as code with
     underscores replaced by spaces, null pointer sorting, LF/no-LF behavior,
     and the global precedence tuple
     `(phase_rank, precedence_index, json_pointer_bytes)`.
   - Slice-1 diagnostic precedence tests are synthetic-only: they use dummy
     phase/code/pointer rows to test diagnostic sorting, key order, schema,
     LF/no-LF behavior, and null-pointer ordering. They must not construct
     `bundle.json`, `bundle.files`, fixture paths, media-type rows, hash rows,
     or any real bundle parser inputs. `bundle.files` phase-ownership and
     multi-error tests live only in slice 6 after bundle parsing, schema checks,
     file-set checks, media-type checks, and hashing exist.

2. Stdlib package backend smoke

   Files:
   `packaging/agentrust_build_backend.py`, `pyproject.toml`,
   packaging unit tests under `tests/agentrust/`.

   Acceptance:
   - Backend smoke tests build a minimal current tree after installing only the
     hashed requirements lock; no network backend bootstrap and no unpinned
     build backend dependency is required.
   - The backend has deterministic helpers for wheel/sdist metadata, `WHEEL`,
     `METADATA`, `RECORD`, archive member ordering, file modes, and package
     filenames.
   - Full wheel/sdist contents, public scans, venv install, and sdist rebuild are
     deferred to the final artifact-lane slice after public modules, docs,
     generated-output code, and package guard logic exist.

3. Linux path safety and output commit

   Files:
   `spendguard_agentrust/paths.py`, `spendguard_agentrust/output.py`.

   Acceptance:
   - Public path values are checked by exact repo-relative literals before
     filesystem access.
   - Linux conformance uses dirfd/no-follow checks, owner/mode/device/link
     checks, mountinfo fstype validation, xattr/ACL checks, and
     `renameat2(RENAME_NOREPLACE)`.
   - Non-Linux or unsupported local filesystems fail as specified instead of
     silently degrading to weaker path handling.
   - Output temp directories, stale cleanup, readback, and final commit follow
     the phase-specific diagnostics in the review bundle.
   - Linux tests cover final-exists and stale-temp cleanup phase mapping,
     symlink, hardlink, device/FIFO/socket, owner/mode/group-write/setuid/sticky,
     other-write bits, invalid directory modes outside `0700|0750|0755`, ACL
     xattr, unsafe dot path, UID-0 rejection, unsupported fstype,
     malformed/conflicting mountinfo, `renameat2` failure, xattr probe failure,
     deleted cwd, no rename/copy/link fallback behavior, and expected
     `path_safety_error` versus `output_target_invalid` selection.

4. Canonical JSON, schema, and fixture preflight

   Files:
   `spendguard_agentrust/c14n.py`, `spendguard_agentrust/jsonio.py`,
   `spendguard_agentrust/schemas.py`, `spendguard_agentrust/fixtures.py`,
   `spendguard_agentrust/package_contract.py`.

   Acceptance:
   - SG-C14N supports closed JSON objects, duplicate-key rejection, bytewise
     sorted keys, exact integer grammar `0|-?[1-9][0-9]*`, no `-0`, no
     fractions/exponents/NaN/Infinity, signed 64-bit integer range unless schema
     narrows it, and no implicit Unicode normalization.
   - Make preflight checks every first-class payload artifact path, size, SHA,
     and drift code before runnable checks:
     `docs/specs/agentrust-package-probe-2026-07-01.json`,
     `docs/specs/agentrust-bundle-root-vector-2026-07-01.json`,
     `docs/specs/agentrust-manifest-uuidv7-vector-2026-07-01.json`,
     `docs/specs/agentrust-schema-source-v1alpha1.json`,
     `docs/specs/agentrust-cloudevent-signature-vector-2026-07-01.json`,
     `proto/spendguard/common/v1/common.proto`,
     `docs/specs/check-agentrust-vectors.py`,
     `integrations/agentrust/examples/fixture-sha256-manifest.v1alpha1.json`,
     `integrations/agentrust/examples/generated-output-sha256-manifest.v1alpha1.json`,
     all nine files under
     `integrations/agentrust/examples/expected-outputs/**`, and
     `integrations/agentrust/requirements-v1alpha1.lock`.
   - Drift code mapping is fixed: schema/fixture/committed-golden artifacts use
     `schema_source_mismatch`; package probe, vectors, proto, checker, and
     checked-in package constants use `package_probe_drift`; the lock uses
     `package_version_mismatch`; fixture TRACE key bytes/format use
     `trace_key_invalid`.
   - Make setup also checks
     `docs/specs/agentrust-required-artifact-evidence-2026-07-01.json` and
     requires its SHA-256 to equal the review bundle `json_sha256`; this decoded
     evidence file is setup evidence, not a verifier runtime input.
   - Schema, vectors, fixture/generated-output manifests, expected outputs, and
     lock are loaded only by make preflight, package-probe checks, and make
     golden comparison. Public verifier runtime and `_verify_internal` runtime
     must not open fixture manifests, vectors, the lock, expected outputs, or
     package guard files.
   - Tests instrument file opens for both public `verify` and
     `_verify_internal verify` success/failure runs to prove those forbidden
     read surfaces stay confined to make preflight/probe/golden code.
   - `python3 docs/specs/check-agentrust-vectors.py` remains passing.

5. Phase-4 package prerequisite contract

   Files:
   `spendguard_agentrust/package_contract.py`,
   `spendguard_agentrust/package_probe.py`.

   Acceptance:
   - Runtime phase 4 uses checked-in constants copied from the approved payload
     for required package names, versions, exported symbol names, signatures,
     accepted JSON shapes, proto availability, and TRACE key format checks; it
     does not open the lock, probe, vectors, manifests, expected outputs, or
     package guard files.
   - Make preflight verifies those constants still match the approved payload
     files before any verifier run.
   - Tests force `package_missing`, `package_version_mismatch`,
     `package_probe_drift`, and `trace_key_invalid` without forbidden runtime
     file reads.

6. Bundle intake through phase-10 lifecycle and binding validation

   Files:
   `spendguard_agentrust/bundle.py`, `spendguard_agentrust/privacy.py`,
   `spendguard_agentrust/trust.py`, `spendguard_agentrust/lifecycle.py`,
   `spendguard_agentrust/bindings.py`.

   Acceptance:
   - Phase order 1-10 is preserved: bundle index before file set, package
     prerequisite checks before raw hash, raw hash before non-bundle JSON parse,
     and trust/signature checks before lifecycle/binding checks.
   - Unit and fixture tests cover every phase boundary and diagnostic family,
     not only conformance mutations: bundle index, file set, privacy,
     full-schema/media type, raw file hash, evidence bundle root, non-bundle
     canonical JSON, `data_b64`, CloudEvent canonical hash, trust key missing,
     trust authorization, key time window, signature invalid, lifecycle,
     and policy/tool/build binding.
   - `bundle.files` ownership multi-error tests assert that phase 2 owns
     array/object/path shape, phase 3 owns order/membership/physical files,
     phase 5 owns media type and SHA schema/profile, and phase 6 owns digest
     comparison.
   - These `bundle.files` tests are introduced in slice 6, not slice 1. They
     create real multi-error bundle inputs only after the bundle parser,
     file-set validator, media-type/schema validator, and raw hash checker are
     implemented.
   - Allow and deny fixture physical file sets match `bundle.json`.
   - CloudEvent signed-byte reconstruction and Ed25519 verification use the
     trust store and signed payload bytes, not regenerated semantic objects.
   - Phase-10 ownership is explicit. `trust.py` stops at phase 9 and may only
     emit trust-key, authorization, key-window, and signature diagnostics.
     `lifecycle.py` owns lifecycle derivation and emits only
     `lifecycle_mismatch`. `bindings.py` owns signed policy, tool, and build
     binding checks and emits only `policy_binding_mismatch`,
     `tool_binding_mismatch`, and `build_binding_mismatch`. Phase-10 checks
     consume parsed bundle/evidence and phase-9 trust results; they are not
     folded into bundle, privacy, or signature code.
   - Mutated signature, wrong key, and mutated policy hash fail at the expected
     phase/code/pointer. `missing_trace_signature` is phase-11 output-package
     validation and is owned by slices 7/9, not this phase-1 through phase-10
     slice.

7. TRACE, Agent Manifest, and generated outputs

   Files:
   `spendguard_agentrust/trace.py`, `spendguard_agentrust/manifest.py`,
   `spendguard_agentrust/report.py`.

   Acceptance:
   - Public success writes all four fixture outputs under
     `integrations/agentrust/out/<bundle_id>/`.
   - Required per-fixture outputs are `trace-level0.json`,
     `agent-manifest.json`, `spendguard-artifacts.json`, and
     `verification-report.json`. The selected output subset used in pass stdout
     remains `trace-level0.json`, `agent-manifest.json`, and
     `spendguard-artifacts.json`.
   - All four fixture outputs are SG-C14N without LF and byte-match committed
     expected outputs for the default issuer.
   - Phase-11 tests cover TRACE validation, Agent Manifest validation, manifest
     layout, generated output schema, readback, final commit, and late cleanup
     diagnostics after output modules exist.
   - The `missing_trace_signature` mutation is tested here: the make-only hook
     removes `/signature` from the in-memory TRACE dict before TRACE package
     validation and before any temp output directory is created, expecting
     `trace_package_validation_failed` at `/signature`.
   - Non-default valid `--manifest-issuer` runs are accepted but not golden.
   - Pass stdout is the exact compact no-LF JSON result with keys
     `bundle_id,outputs,schema,status`, schema
     `spendguard-agentrust-verify-result/v1`, and selected-output filename order
     `trace-level0.json,agent-manifest.json,spendguard-artifacts.json`.

8. Public docs and submission materials

   Files:
   `docs/integrations.md`,
   `docs/site/docs/integrations/agentrust.md`,
   `docs/site-v2/src/content/docs/docs/integrations/agentrust.mdx`,
   plus upstream submission notes under `docs/specs/` if needed.

   Acceptance:
   - The three public docs describe fixture-only Community scope and avoid all
     forbidden claim text before the full package guard runs, because missing or
     unreadable live docs are make setup failures.
   - The docs link to the verifier command and spec without claiming upstream
     approval.
   - Public docs do not expose private harness flags, private key material, or
     production trust/key-distribution language.
   - Any upstream submission notes under `docs/specs/` are internal drafts unless
     they pass the same forbidden-claim scan and are explicitly promoted into
     the public submission artifact list. Draft notes cannot be submitted as-is.

9. Make harness, negative tests, and aggregate report

   Files:
   `spendguard_agentrust/_verify_internal.py`,
   `spendguard_agentrust/_make_verify.py`,
   `spendguard_agentrust/package_guard.py`, `Makefile`.

   Acceptance:
   - `_verify_internal` requires
     `SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT=1` and source-checkout realpaths.
   - `_make_verify package-guard` is a make-only/internal entry point protected
     by the same env/source-realpath gate; it is absent from wheel/sdist public
     entry points and help.
   - `make verify-agentrust` honors `PYTHON=<venv>/bin/python`; all Python
     subprocesses run with `PYTHONNOUSERSITE=1`. The source lane does not
     mutate the caller's venv; artifact-lane build/install venvs are fresh temp
     directories under the make run root and are not reused across lanes.
   - Private parser tests cover exact `_verify_internal verify` grammar,
     internal path roots under `.verify-agentrust/<run-id>`, fixture/mutation
     cross-field mismatches, unsupported public paths, missing env gate, and
     source-realpath rejection.
   - Positive private grammar is exactly:
     `_verify_internal verify --bundle integrations/agentrust/examples/<allow|deny> --trust-store integrations/agentrust/examples/trust-store.json --trace-signing-key integrations/agentrust/out/.verify-agentrust/<run-id>/.trace-key/trace-exporter.test.ed25519.pem --out-dir integrations/agentrust/out/.verify-agentrust/<run-id> --fixture <allow|deny> [--manifest-issuer SPIFFE_URI]`.
   - Negative private grammar is exactly:
     `_verify_internal verify --bundle integrations/agentrust/out/.verify-agentrust/<run-id>/.mutations/<mutation>/<fixture>/bundle --trust-store integrations/agentrust/out/.verify-agentrust/<run-id>/.mutations/<mutation>/<fixture>/trust-store.json --trace-signing-key integrations/agentrust/out/.verify-agentrust/<run-id>/.trace-key/trace-exporter.test.ed25519.pem --out-dir integrations/agentrust/out/.verify-agentrust/<run-id>/.mutations/<mutation>/<fixture>/out --fixture <fixture> --mutation <mutation>`.
   - Private flags are separate tokens only, exactly once, no equals forms, no
     public `--mode` or `--fixture-profile`, no extra args; fixture/mutation/path
     cross-field mismatches are `cli_value_invalid`.
   - Private success stream tests assert that allow, deny, and issuer-variant
     `_verify_internal verify` success runs write private outputs with exact
     empty stdout and empty stderr.
   - Source-lane isolation tests spawn a new Python interpreter per probe/run,
     prove no module cache reuse across checks, and include poisoning fixtures
     for `.pth`, `.egg-link`, namespace providers, `build/`, `dist/`,
     site-packages, wheel files, zip/zipimport, and outside-repo realpaths.
     Every poisoning case fails make setup with `packaging_guard_failed` before
     any verifier run. This is make-setup ownership, not a verifier phase
     diagnostic.
   - Make copies the TRACE test key from the fixture manifest into `.trace-key`
     with directory mode `0700`, exclusive no-follow file open `0600`, verifies
     SHA/format, uses only the copied key for private runs, and unlinks it before
     final package guard.
   - Negative tests implement exact mutation byte construction and expected
     hashes for `mutated_signature`, `wrong_key`, `mutated_policy_hash`, and
     `missing_trace_signature`; malformed negative stderr, unexpected
     exit/signal, and mismatch cases are reportable aggregate failures.
   - Negative output-absence scan walks exactly each `<mutation-out-root>` with
     no-follow semantics; symlinks count as present, selected outputs/final
     bundle dirs/probe/temp dirs fail `outputs_absent`, and `.diagnostics/**`
     outside the scan root is ignored.
   - `make verify-agentrust` runs setup, parser/help, allow, deny, issuer
     variant, four deterministic negatives, golden comparison, schema checks,
     source lane checks, and private/public parser matrices. Full artifact-lane
     package guard scans are completed in the final artifact-lane slice.
   - Make exit 0 has stdout exactly `agentrust_make_verify=pass\n` and empty
     stderr.
   - Make report path is
     `integrations/agentrust/out/.verify-agentrust/<run-id>/verify-agentrust-report.json`.
     It is compact JSON+LF, not SG-C14N, with exact top-level key order:
     `conformance_checks,mode,negative_tests,package_versions,schema,success_fixtures,status`.
     Report serialization order is normative from the review bundle; do not
     derive top-level order from `agentrust-schema-source-v1alpha1.json`.
   - `conformance_checks` contains exactly
     `artifact_preflight_envelope,event_type_binding,fixture_identity,internal_guard,manifest_issuer_variant,package_guard_surfaces,private_parser_matrix,public_parser_matrix`,
     each with `diagnostic_code,status`.
   - `success_fixtures` contains `allow,deny`, each with
     `bundle_id,diagnostic_code,outputs_match,status`.
   - `negative_tests` contains
     `missing_trace_signature,mutated_policy_hash,mutated_signature,wrong_key`,
     each with `diagnostic_code,expected_exit_code,observed_exit_code,outputs_absent,status`.
   - Make exit 1 aggregate failures have empty stdout and exactly one compact
     JSON object plus LF on stderr, no other bytes. Key order is
     `report_path,schema,status`; schema is
     `spendguard-agentrust-make-summary/v1`; status is `fail`; `report_path`
     points to the closed report at
     `<fresh-out-dir>/verify-agentrust-report.json`. They continue through
     runnable parser/help, issuer, allow/deny, negative,
     malformed-negative-stderr, unexpected exit/signal, and golden failures.
   - Make exit 6 setup/final aborts have empty stdout and exactly one compact
     diagnostic JSON plus LF on stderr with key order
     `schema,exit_code,phase,code,message,json_pointer`, schema
     `spendguard-agentrust-diagnostic/v1`, `exit_code=6`, and `phase` exactly
     `make-setup` or `make-final`. Malformed or missing final report
     schema/key/order is `make-final` exit 6, not aggregate exit 1.
   - Package guard tests use fixtures for the exact scan algorithm: raw byte
     PEM/secret scan before text/binary classification, invalid UTF-8 or NUL in
     required text as `packaging_guard_failed`, UTF-8 BOM stripping, no CRLF
     normalization, ASCII A-Z folding, repeated allowlist-snippet removal with
     ASCII boundaries, JSON raw UTF-8 scan, decoded JSON key/string-value scan
     in SG-C14N pointer order, structural SLSA exception only for
     `/build_provenance/slsa_level = 1`, and the exact Python 3.12 ASCII regex
     deny list from the review bundle.
   - Package guard fixtures also cover generated site assets, internal docs,
     customer docs, operations docs, and the structural `replay_cache` /
     `replay_cache_report` allowance. Those keys are allowed only in the
     embedded schema and verification-report golden with status
     `not_applicable_fixture`; public claim text `replay-cache` remains
     forbidden.
   - Guard coverage also asserts documented env influence only
     (`SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT`, `PYTHONPATH`,
     `PYTHONNOUSERSITE`), no absolute sensitive paths in diagnostics/logs,
     entry-script discovery from `entry_points.txt`, `RECORD`, and venv
     `bin/spendguard*`, explicit context-tar enumeration sources, and that the
     public verifier never emits `packaging_guard_failed`.
   - Test file ownership is fixed under `tests/agentrust/`:
     `test_make_streams.py` covers exit 0/1/6 stdout/stderr contracts;
     `test_make_report_contract.py` covers report schema, key order, status,
     and summary stderr; `test_private_parser.py` covers exact private grammar
     and cross-field mismatches; `test_package_guard.py` covers raw-byte scans,
     public-surface leaks, and final-lane failures; `test_artifact_lane.py`
     covers wheel/sdist build, clean venv install, and private absence. Fixtures
     may live under `tests/agentrust/fixtures/**`; committed AgenTrust evidence
     fixtures stay under `integrations/agentrust/**`.

10. Final artifact lane and full make verification

   Files:
   `packaging/agentrust_build_backend.py`,
   `spendguard_agentrust/package_guard.py`, `Makefile`,
   public docs and package metadata.

   Acceptance:
   - This final artifact lane is part of `make verify-agentrust`, not an
     optional post-check. It runs after aggregate runnable checks and before
     exit 0. Any package/public-surface leak, private harness exposure,
     forbidden claim, malformed report, or final report schema/key/order failure
     aborts with exit 6 and `phase=make-final`.
   - Build, source-test/make, installed-wheel, sdist-build, and sdist-install
     checks use separate clean venvs. No lane reuses another lane's interpreter
     or site-packages.
   - Before each artifact-lane build, delete `build/`, `dist/`, and every
     root-level `*.egg-info`; fail if any remains after cleanup. Build
     wheel/sdist after all public modules, docs, generated-output code, and
     package guard logic exist from a clean lock-pinned build venv only:
     `PYTHONNOUSERSITE=1 <build-venv>/bin/python -m pip install --require-hashes -r integrations/agentrust/requirements-v1alpha1.lock`,
     then
     `PYTHONNOUSERSITE=1 <build-venv>/bin/python -m build --no-isolation --sdist --wheel`.
   - Inspect archive members in bytewise order and discover entry scripts from
     `entry_points.txt`, `RECORD`, and venv `bin/spendguard*` before scanning.
   - Artifact-lane build, install, help, and rebuilt-sdist checks run with
     `PYTHONPATH` cleared and `PYTHONNOUSERSITE=1`. Tests assert installed
     `spendguard_agentrust` module realpaths are under the clean venv
     site-packages, never the source checkout.
   - Wheel install path: create a clean venv with `PYTHONNOUSERSITE=1`, install
     `integrations/agentrust/requirements-v1alpha1.lock` with
     `pip --require-hashes`, select exactly one freshly built
     `spendguard_agentrust-*.whl` by captured realpath rather than `dist/*.whl`,
     install that wheel with `--no-deps`, assert package and dependency versions
     match approved pins, run public help, and prove `_verify_internal` is absent.
   - Sdist path: verify the sdist includes only rebuild-required public files,
     `pyproject.toml`, and `packaging/agentrust_build_backend.py`; capture
     exactly one freshly built `spendguard_agentrust-*.tar.gz` by realpath;
     rebuild a wheel from the captured sdist with `--no-isolation` under a
     separate hashed-lock build venv; select exactly one rebuilt wheel by
     captured realpath; install that rebuilt wheel with `--no-deps` into a
     separate clean install venv; rerun public help and package-surface scans.
   - Package guard scans wheel/sdist names and bytes, dist metadata, generated
     entry scripts, public package docs/help, generated verifier outputs, make
     report, aggregate summary stderr, captured diagnostic JSON, the three
     public docs, and declared context tar entries using the exact scan
     algorithm.
   - Full `make verify-agentrust` passes only after this final artifact lane,
     parser/help, allow/deny, issuer variant, negative tests, golden comparison,
     report schema, and package guard all pass.

11. CI wiring

   Files:
   `.github/workflows/agentrust.yml` or a narrowly scoped existing workflow.

   Acceptance:
   - CI runs on non-root Ubuntu 24.04 or Debian 12 with a supported filesystem
     and first performs smoke checks for UID, mountinfo fstype, owner, xattr
     support, ACL absence, and `renameat2`.
   - CI installs from `integrations/agentrust/requirements-v1alpha1.lock` with
     hashes, runs `python3 docs/specs/check-agentrust-vectors.py`, stdlib
     `unittest` tests,
     package build/inspect, and `make verify-agentrust`.
   - CI deletes `build/`, `dist/`, and `*.egg-info`, uses separate
     lock-pinned build/test/install venvs with `PYTHONNOUSERSITE=1`, captures
     the exact single built wheel path, and installs wheels with `--no-deps`.
   - CI treats drifting `ubuntu-latest`, macOS, overlayfs-only containers, root,
     userns/idmapped, FUSE, or network filesystems as non-success for
     conformance. Skip is allowed only as an explicit non-success signal, not as
     a green conformance result.

## Test plan

Local tests before Task review:

```bash
set -euo pipefail

ROOT="$(pwd -P)"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/spendguard-agentrust.XXXXXX")"
trap 'rm -rf "$TMP_ROOT"' EXIT
BUILD_VENV="$TMP_ROOT/build-venv"
TEST_VENV="$TMP_ROOT/test-venv"
INSTALL_VENV="$TMP_ROOT/install-venv"
LOCK="integrations/agentrust/requirements-v1alpha1.lock"

python3 - <<'PY'
import sys
assert sys.version_info >= (3, 12), sys.version
PY
export PYTHONNOUSERSITE=1 PIP_DISABLE_PIP_VERSION_CHECK=1
unset PYTHONPATH || true

python3 docs/specs/check-agentrust-vectors.py

rm -rf build dist ./*.egg-info
python3 -m venv "$TEST_VENV"
"$TEST_VENV/bin/python" -m pip install --require-hashes -r "$LOCK"
PYTHONPATH="$ROOT" "$TEST_VENV/bin/python" -m unittest discover -s tests/agentrust

python3 -m venv "$BUILD_VENV"
"$BUILD_VENV/bin/python" -m pip install --require-hashes -r "$LOCK"
rm -rf build dist ./*.egg-info
"$BUILD_VENV/bin/python" -m build --no-isolation --sdist --wheel --outdir dist

mapfile -t WHEELS < <(find dist -maxdepth 1 -type f -name 'spendguard_agentrust-*.whl' | LC_ALL=C sort)
mapfile -t SDISTS < <(find dist -maxdepth 1 -type f -name 'spendguard_agentrust-*.tar.gz' | LC_ALL=C sort)
[[ "${#WHEELS[@]}" -eq 1 && "${#SDISTS[@]}" -eq 1 ]]
WHEEL_PATH="$(realpath "${WHEELS[0]}")"
SDIST_PATH="$(realpath "${SDISTS[0]}")"

python3 -m venv "$INSTALL_VENV"
env -u PYTHONPATH "$INSTALL_VENV/bin/python" -m pip install --require-hashes -r "$LOCK"
env -u PYTHONPATH "$INSTALL_VENV/bin/python" -m pip install --no-deps "$WHEEL_PATH"
env -u PYTHONPATH "$INSTALL_VENV/bin/python" -m pip check
env -u PYTHONPATH "$INSTALL_VENV/bin/python" -m spendguard_agentrust verify --help
env -u PYTHONPATH "$INSTALL_VENV/bin/python" - <<'PY'
from pathlib import Path
import spendguard_agentrust, sys
mod = Path(spendguard_agentrust.__file__).resolve()
prefix = Path(sys.prefix).resolve()
assert mod.is_relative_to(prefix), (mod, prefix)
PY

rm -rf build dist ./*.egg-info
PYTHON="$TEST_VENV/bin/python" PYTHONPATH="$ROOT" make verify-agentrust
git diff --check -- pyproject.toml packaging spendguard_agentrust tests/agentrust Makefile .github/workflows/agentrust.yml integrations/agentrust docs/specs/agentrust-integration-plan-v1alpha1.md docs/specs/agentrust-integration-spec-review-bundle-v1alpha1.md docs/specs/agentrust-integration-spec-v1alpha1.md docs/specs/agentrust-*.json docs/specs/check-agentrust-vectors.py docs/integrations.md docs/site/docs/integrations/agentrust.md docs/site-v2/src/content/docs/docs/integrations/agentrust.mdx proto/spendguard/common/v1/common.proto
```

When running from macOS or another unsupported local filesystem, record the
expected setup failure and require the Linux CI job above for conformance.

## Staff+ Plan Recovery Summary

Plan review reached the fifth failed iteration. The Staff+ Expert Recovery Cycle
deduplicated the blockers into five root causes:

- Phase ownership was ambiguous for `missing_trace_signature` and lifecycle
  binding diagnostics.
- Source-lane and artifact-lane import isolation could conflict with build
  outputs, installed wheels, and parser-only unit tests.
- Package guard scan inputs did not fully name generated verifier outputs,
  reports, public docs, context archives, structural SLSA exceptions, and
  replay-cache fixtures.
- Local verification order could let `build/`, `dist/`, or root `*.egg-info`
  pollute source-lane setup checks.
- CI environment requirements were under-specified for supported Linux and
  filesystem behavior.

The Staff+ decision is to proceed with this plan as the authoritative Task input:
the plan now assigns those responsibilities to explicit work slices, separates
source and artifact lanes, clears build artifacts before each lane transition,
requires `PYTHONPATH` isolation for installed-wheel checks, and pins Linux CI to
non-root Ubuntu 24.04 or Debian 12. Per `AGENTS.md`, this resolves the Plan
stage and advances the workflow to Task implementation.

## Staff+ Task Recovery Summary

Task review reached the fifth failed iteration. The Staff+ Expert Recovery Cycle
for architecture/spec scope deduplicated the remaining blockers into four root
causes:

- cryptographic verification depth: the failed review implementation did not
  satisfy the full v1alpha1 contract for raw CloudEvent signed-byte
  reconstruction and Ed25519 verification from the trust store;
- negative-test integrity: make-only negatives prove deterministic diagnostics
  and report wiring more than end-to-end validator behavior;
- report truthfulness: aggregate report entries can be marked `pass` without
  proof that the corresponding package/public-surface guard fully ran against
  the required surfaces; and
- hardening gap: Linux dirfd/no-follow path safety, output commit hardening,
  source/artifact isolation, and final package guard scans are incomplete.

The Staff+ decision is to advance this Task stage only as a fixture-only
docs-first MVP. This approves the public parser, stdlib build backend, committed
allow/deny fixture checks, golden output generation, private env-gated harness,
and make smoke report as useful scaffolding. It does not approve upstream
Community submission, general artifact verification, live evidence verification,
production key distribution, or stronger trust claims.

The deferred conformance track is recorded in
`docs/specs/agentrust-task-recovery-summary-v1alpha1.md`. Before any upstream
submission or stronger public claim, complete full v1alpha1 signed-byte
reconstruction, real verifier-path negative mutations, truth-preserving
aggregate reporting, Linux path/output hardening, final artifact-lane package
scans, and supported Linux CI.

## Review gates

Plan gate command:

```bash
CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh plan docs/specs/agentrust-integration-plan-v1alpha1.md --project-dir $PWD
```

After Plan approval, capture the implementation baseline:

```bash
BASELINE=$(git rev-parse HEAD)
```

Task and Final gates follow `AGENTS.md`. If any stage reaches the fifth failed
review iteration, run the Staff+ Expert Recovery Cycle from `AGENTS.md`: the
expert team deduplicates blockers, decides the fix, executes it, records the
recovery summary, and advances to the next stage after necessary verification.
Only stop for repeated recovery deadlock or external permissions/product
decisions.

## Exit criteria

The implementation is ready for upstream community-submission preparation when:

- Spec, Plan, Task, and Final review gates approve or are resolved by the
  Staff+ Expert Recovery Cycle in `AGENTS.md`.
- `make verify-agentrust` passes in the supported Linux CI environment.
- Public docs and package artifacts pass forbidden-claim and private-surface
  scans.
- The generated fixture outputs byte-match committed goldens.
- Submission materials honestly state fixture-only Community scope and do not
  assert upstream approval.
