# AgenTrust v1alpha1 Spec Gate Artifact

Normative review target for SpendGuard `community-submission`. The full design
`docs/specs/agentrust-integration-spec-v1alpha1.md` is background only and cannot
override this packet. The Evidence Payload embeds SHA-pinned normative artifact
bytes and the replay evidence for commands named in the payload. Live scan inputs
are listed explicitly and checked from the current checkout/package build, not
treated as payload authority. If the payload cannot be decoded, if any embedded
artifact hash differs from its path row, or if payload command evidence is
missing/non-replayable, Spec approval fails.

## Scope

v1alpha1 is fixture-only Community submission:

- TRACE L0 software records signed with committed non-production fixture keys.
- Unsigned Agent Manifest bindings through `agent-manifest==0.1.1`.
- No live, historical, replay-cache, hardware, CMCP, MCP gateway, SLSA, certified,
  compliant, or production trust/key-distribution claim.
- Conformance is Linux-only local fixture tooling. macOS/non-Linux developers use
  conforming Linux CI/container; local non-Linux failure is expected.

Forbidden-claim surfaces are closed: generated verifier outputs including make
report/diagnostic JSON; public help; wheel/sdist member names; dist metadata
(`METADATA,WHEEL,entry_points.txt,RECORD`); generated entry-point scripts; public
package docs; declared context tar member names/text; and public submission docs
only `docs/integrations.md`, `docs/site/docs/integrations/agentrust.md`,
`docs/site-v2/src/content/docs/docs/integrations/agentrust.mdx`. Other `docs/**`,
generated site assets, sourcemaps, internal/customer/operations docs, private
mutation workdirs, copied TRACE keys, and source `.diagnostics` files are out of
scope unless copied into generated/package artifacts. If wheel/sdist/context tar
contains `docs/specs/**`, this review bundle, sourcemaps, generated site assets,
internal/customer/operations docs, or nested archives, make setup fails
`packaging_guard_failed` before scanning.

Scan units are archive member names, public UTF-8 text surfaces, text extensions
`.py,.txt,.md,.rst,.toml,.cfg,.ini,.json,.yaml,.yml,.sh`, dist metadata, JSON raw
UTF-8, then decoded JSON keys/string values in SG-C14N pointer order. Invalid
UTF-8 or NUL in required text is `packaging_guard_failed`; UTF-8 BOM is stripped;
CRLF is not normalized; Unicode lookalikes are distinct. Member names use stored
POSIX paths; backslash, drive/absolute path, empty, `..`, repeated slash, or nested
archive by extension or magic is `packaging_guard_failed`. Binary members not
banned above are opaque; only names are scanned. ASCII-fold A-Z to a-z, remove
allowlist snippets `not production trust evidence` and `not yet community/verified`
repeatedly with ASCII non-alnum/start/end bounds, apply structural TRACE JSON
`/build_provenance/slsa_level = 1` only to that field/value, then deny-match with
Python 3.12 `re` ASCII semantics:
`agentrust verified`, `trace[ -]?(level[ -]?)?[12]`, `hardware attestation`,
`signed manifest trust`, `cmcp`, `mcp gateway`, `live verification`,
`historical replay`, `replay-cache`, `production (trust|key distribution)`,
`slsa[_ -]?(level[_ -]?)?1`, `slsa[_ -]?l1`, `certified`, `compliant`,
`attested`. TRACE L0 wording is allowed.

## Authority

Schema source, package probe, vectors, proto, checker, fixture manifest,
expected-output manifest/files, and requirements lock are embedded in the Evidence
Payload. Prose-vs-artifact conflict is a Spec-gate
failure, not implementer choice. Inside the payload, authority order is: path rows
and SHA/size metadata; artifact bytes; executable checker/vector/probe contracts;
schema/proto documents; fixture/expected-output manifests; command evidence. If
two embedded artifacts conflict, make preflight fails before runnable checks using
the drift code for the higher-authority class, never by choosing one artifact.
Product diagnostics for artifact drift are: `schema_source_mismatch` for
schema/fixture/committed-golden manifest or file drift before runnable checks,
`package_probe_drift` for probe/proto/vector/checker, `package_version_mismatch` for lock,
`trace_key_invalid` for key bytes/format, and `packaging_guard_failed` for
package guard. Public/private verifier runtime never opens the fixture manifest,
vectors, lock, expected outputs, or package guard; it reads only CLI inputs and
phase-4 runtime prerequisites. Live scan inputs are the three public docs, public
help text, generated outputs, and freshly built wheel/sdist surfaces after proving
`docs/specs/**` and this review bundle are absent from built artifacts; missing or
unreadable live inputs fail make setup.

Spec-gate comparison algorithm is closed: decode payload; require unique artifact
paths; verify every artifact size/SHA against embedded bytes; verify command argv,
exit, stream lengths, and stream hashes; parse each artifact by path-mapped format
(`*.json` JSON, `*.proto` proto text, `*.py` Python source, `*.lock`
requirements text); then compare every prose-named path, version, hash, fixture
id, mutation constant, and expected output filename against parsed payload values.
Any mismatch is a Spec-gate failure. Product drift diagnostics above apply only
when implementation-time live files or generated bytes disagree with this packet.

## Command State Matrix

Public `verify --help` reads argv only, has no phases, exits 0 with exact help
stdout and empty stderr. Public `verify [flags]` runs phases 1-11, reads accepted
CLI paths plus phase-4 prerequisites, and emits either pass JSON stdout or one
diagnostic stderr. Private `_verify_internal verify` is make-only: success writes
private outputs with empty streams; negatives stop at the expected phase with one
diagnostic and selected outputs absent. `make verify-agentrust` runs setup,
parser/help, allow/deny, negatives, issuer, golden/schema/package guards; exit 0
prints `agentrust_make_verify=pass\n`, exit 1 writes summary/report, exit 6 emits
setup/final diagnostic.

## Public CLI

Only command: `python -m spendguard_agentrust verify [flags]`. Required flags
exactly once: `--bundle`, `--trust-store`, `--trace-signing-key`, `--out-dir`.
Optional flags at most once: `--mode fixture`, `--fixture-profile upstream`,
`--manifest-issuer SPIFFE_URI`. Values are separate tokens; supported `--x=y`
forms fail `cli_equals_form_unsupported` except closed unsupported capability
lexemes. Defaults: `--mode fixture`, `--fixture-profile upstream`,
`--manifest-issuer spiffe://spendguard.local/agentrust-exporter`.

Supported public path literals:

- `--bundle`: `integrations/agentrust/examples/allow` or
  `integrations/agentrust/examples/deny`.
- `--trust-store`: `integrations/agentrust/examples/trust-store.json`.
- `--trace-signing-key`:
  `integrations/agentrust/test-private-keys/spendguard-agentrust-fixture.test.ed25519.pem`.
- `--out-dir`: `integrations/agentrust/out`.

The public TRACE key literal is authorized only by embedded fixture-manifest
bytes at Spec/make preflight. Public verifier does not discover paths from the
manifest. CLI tokens are Python `sys.argv` strings; empty, NUL, surrogate, control,
or non-ASCII tokens fail before normalization. Value-position failures are
`cli_value_invalid`; flag/command-position failures are `cli_parse_error`. Public
path values match the literals above by ASCII string equality before filesystem
access. Any other path value, including `./...`, wrong case, absolute path, `..`,
hidden segment, repeated slash, glob/env/tilde, alternate existing repo path, or
the wrong-key path, is phase-1 `cli_value_invalid`. Exact literals may contain `/`;
slash rejection applies only to traversal, leading/trailing/repeated slash, or
non-literal values.

Exact `verify --help` is out-of-band: exit 0, deterministic help stdout, empty
stderr, no application data reads, no JSON. Interpreter/module imports are outside
this read claim. Any other raw `--help`/`--help=*` is
`cli_parse_error`, except as the missing value after a value-taking flag
(`--bundle --help` -> `cli_missing_value`). Help lists only public flags and no
internal sentinel.

Public parser precedence after `verify`:

| Order | Condition | Diagnostic |
|---:|---|---|
| 1 | supported value flag missing value or followed by token starting `--` | `cli_missing_value` |
| 2 | missing `verify`, flag before `verify`, unsupported subcommand, non-exact help, or positional garbage before the first unsupported-capability lexeme and not part of its listed pair | `cli_parse_error` |
| 3 | unsupported capability token/equals/pair: `--mode live|historical`, `--mode=live|historical`, `--fixture-profile local|=local`, `--force`, `--force=*`, `--cmcp-*`, `--cmcp-*=*`, `--mcp-*`, `--mcp-*=*`, `--tested-against`, `--tested-against=*`, `--tested-against <value>` | `unsupported_capability` |
| 4 | unknown flag name | `cli_parse_error` |
| 5 | duplicate supported flag | `cli_parse_error` |
| 6 | supported equals form | `cli_equals_form_unsupported` |
| 7 | omitted required flag | `cli_missing_required_flag` |
| 8 | invalid supported value | `cli_value_invalid` |

The table above is the sole parser selection algorithm. Build row candidates with
the triggering token index after `verify`; missing `verify` has index -1. Row 1
wins first by lowest token index. Let `u` be the lowest row-3 unsupported
capability token; row 2 candidates are eligible only when their index is `< u`
or no `u` exists. If eligible row 2 exists, emit its lowest index. Else if `u`
exists, emit row 3 at `u`; row-3 pairs consume at most one following non-flag
value and suppress positional parsing only for that value. Else select the lowest
row 4-8, then lowest token index. Later failures are ignored.

Fixture assertions include: `--mode --force` -> `cli_missing_value`;
`--tested-against foo --bundle x` -> `unsupported_capability`;
`--bundle value extra --mode live` -> `cli_parse_error`; `--unknown --force` ->
`unsupported_capability`; `--fixture-profile=upstream` and `--mode=fixture` ->
`cli_equals_form_unsupported`; `--help --bundle x`, `--help --help`, `--help=x` ->
`cli_parse_error`.

`--manifest-issuer` is raw ASCII `spiffe://<host>/<segment>(/<segment>)*`; raw and
decoded byte lengths are each max 256. Host is lowercase `[a-z0-9.-]`, no empty
label, and labels may not start or end with `-`. Validate order:
scheme/host/segment grammar, uppercase `%HH` syntax, percent-decode each segment,
then decoded-byte policy. Segments use raw `[A-Za-z0-9._~!$&'()*+,;=:@-]` or
uppercase percent escapes. Query, fragment, non-ASCII, control, backslash, empty
segment, lowercase/invalid escapes, decoded slash/query/fragment/backslash, or
decoded bytes outside `0x21..0x7e` are `cli_value_invalid`; decoded bytes
`0x2f,0x3f,0x23,0x5c` are always rejected. Examples: `%7E` valid; `%2f`, `%20`, `%2F`,
`%3F`, `%23`, and `%5C` invalid. Non-default issuer runs are valid but
non-golden.

## Private And Make Harness

`python -m spendguard_agentrust._verify_internal` is make-only source-checkout
harness code. It requires `SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT=1`, exact
private grammar, internal paths under `.verify-agentrust/<run-id>`, and no public
exposure through console scripts, package entry points, generated scripts,
packaged docs, public docs, or help.

Make has two isolated lanes:

- Source lane for private verifier: fresh Python process for each probe/run,
  `PYTHONPATH=<repo-root>`, `PYTHONNOUSERSITE=1`, no module cache reuse, and every
  `spendguard_agentrust*` realpath under repo root. Editable/namespace providers,
  `.pth`/`.egg-link`, `build/`, `dist/`, site-packages, wheel, or zip realpath
  outside repo fail make setup `packaging_guard_failed`.
- Artifact lane for built wheel/sdist/public package surfaces: verifies public
  help/archive/docs/probe behavior and proves `_verify_internal` is absent from
  wheel/sdist/production installs. Built artifacts are never imported for private
  runs.

Private success grammar requires `--bundle integrations/agentrust/examples/<allow|deny>`,
trust store fixture path, copied TRACE key
`integrations/agentrust/out/.verify-agentrust/<run-id>/.trace-key/trace-exporter.test.ed25519.pem`,
`--out-dir integrations/agentrust/out/.verify-agentrust/<run-id>`, `--fixture
<allow|deny>`, optional `--manifest-issuer`, and no `--mutation`. Negative grammar
adds `--mutation mutated_signature|wrong_key|mutated_policy_hash|missing_trace_signature`
and uses `.mutations/<mutation>/<fixture>/{bundle,trust-store.json,out}` paths.
Private path/fixture/mutation cross-field mismatch is `cli_value_invalid`.

Negative construction is deterministic. Copy the named fixture bundle to
`<mutation-bundle-root>`, write `<mutation-trust-store>` as SG-C14N, create
`<mutation-out-dir>` mode 0700, then invoke `_verify_internal verify` with the
private grammar above:
`_verify_internal verify --bundle <mutation-bundle-root> --trust-store <mutation-trust-store> --trace-signing-key <trace-key-copy> --out-dir <mutation-out-dir> --fixture <fixture> --mutation <mutation>`.
Expected stderr is the verifier diagnostic with this
`exit_code`, `code`, and `json_pointer`; stdout is empty and selected outputs are
absent.

| Mutation | Fixture | Exact construction | Exit/code/pointer |
|---|---|---|---|
| `mutated_signature` | `allow` | In copied `decision.json`, replace `/signature/value_base64url` leading `B` with `A`, serialize SG-C14N+LF, set copied bundle `/files/0/sha256` to `b69c31fdf426bbef43d8bafb6c6cb49b91822c9dae7af035524debc50df41a5a`, and set copied bundle `/evidence_bundle_hash` to `sha256:7988892cf4d09c60df8fd151b138696872e64a4a7fde67230c3b1e633d95db05`. | 3 / `spendguard_signature_invalid` / `/signature/value_base64url` |
| `wrong_key` | `deny` | Copied bundle bytes unchanged. In copied trust store, keep all fields except `/keys/0/public_key_base64url`, replacing it with `GX9rI-FshTLGq8g4-s1ep4m-DHaykgM0A5v6iz02jWE`; SG-C14N+LF trust-store SHA-256 is `16591f61ff2ded3eddbee7163e2d721196baaaf30e8f385f397f8e001dce4dab`. | 3 / `spendguard_signature_invalid` / `/keys/0/public_key_base64url` |
| `mutated_policy_hash` | `allow` | In copied `policy-bundle.json`, replace `/policy_bundle_hash` with `sha256:9999999999999999999999999999999999999999999999999999999999999999`, leave signed decision data unchanged, serialize SG-C14N+LF, set copied bundle `/files/2/sha256` to `4fda930976fdee07abddb8f905fb9da53e6d33dce8757cbaa82317db6023cb00`, and set copied bundle `/evidence_bundle_hash` to `sha256:338a485da0a5e572c8ac91a66dcb5fd03371619e99cbb25a85f5037ab724d170`. | 4 / `policy_binding_mismatch` / `/policy_bundle_hash` |
| `missing_trace_signature` | `deny` | Do not mutate input files. In the make-only generation hook, remove `/signature` from the in-memory TRACE dict before TRACE package validation and before any temp output directory is created. | 5 / `trace_package_validation_failed` / `/signature` |

Make copies the manifest `trace_test_key` to `.trace-key` with mkdir 0700 and
exclusive no-follow key open 0600, verifies SHA against embedded manifest bytes,
and unlinks it before final package guard. Verifier phase 4 reads only selected
key bytes and validates unencrypted PKCS#8 Ed25519 format.

## Local Security And Output

Repo root is `realpath(git rev-parse --show-toplevel)` from process cwd. Git
failure, outside-worktree cwd, empty output, or unresolvable realpath is phase-1
`output_target_invalid`. Repo root `/` or UID 0 is phase-1 `path_safety_error`.
Checked ancestors run from repo root inclusive down to each leaf; OS ancestors
above repo root are out of scope.

Supported runner: non-root Ubuntu 24.04/Debian 12 on local
`ext2|ext3|ext4|xfs|btrfs|zfs`, or container bind-mounted to such workspace;
overlayfs/root/network/FUSE/virtual mounts are expected setup failures. Mount
algorithm: parse `/proc/self/mountinfo` (unreadable/malformed/no matching or
conflicting longest match -> `output_target_invalid`), unescape, longest
component-boundary prefix, require same `st_dev` and allowlisted fstype. Missing
xattr support, deleted cwd, `flistxattr` `ENOTSUP|EOPNOTSUPP|EPERM|EACCES`, or
`renameat2(RENAME_NOREPLACE)` failure (`EEXIST|EXDEV|EINVAL|ENOSYS|EPERM|EACCES`)
is `output_target_invalid`; present `system.posix_acl_*` is `path_safety_error`;
no rename/copy/link fallback is conformant.

Checked paths: repo root, accepted inputs, ancestors, output/run/mutation roots,
listed files, temp/final children. Descriptor sequence: open repo dir fd; descend
relative components with `openat(...,O_NOFOLLOW)`; `fstat` each fd; reject `..`,
empty segment, hidden segment except closed internal dot paths below,
symlink, device/FIFO/socket, hardlink (`nlink!=1` for files), owner not effective
UID, different `st_dev`, group/other write bit, setuid/setgid/sticky bit, ACL
xattr, dir mode not `0700|0750|0755`. These are `path_safety_error`; generic
permission/busy/IO/probe/cleanup failures are `output_target_invalid`.

Threat model: fixture hygiene under no concurrent malicious writer, not a sandbox
or same-UID race defense. Malicious worktree contents, import poisoning before
package import completes, local DoS, fsync loss, and byte changes after checked
`lstat/open` are outside claim; later hashes may detect tampering but are not
TOCTOU defense. `make` constrains imports by source/artifact lanes; direct public
invocation trusts caller Python environment.

Public callers pre-create `integrations/agentrust/out` mode 0700/0750/0755. Make
pre-creates mutation `out` roots mode 0700. `run-id` is
`agentrust-YYYYMMDDTHHMMSSZ-[0-9a-f]{12}`. Verifier never creates supplied
`--out-dir`. Dot exceptions: `.verify-agentrust/<run-id>` under out; direct
`.probe-<32hex>` in phase 1; direct `.tmp-<bundle_id>-<32hex>` in phases 2/11;
under make run root `.trace-key`, `.mutations/<mutation>/<fixture>/{bundle,trust-store.json,out}`,
and `.diagnostics`. Phase 1 creates/deletes/cleans probes; phase 2 owns final
absence and stale temp cleanup; phase 11 creates/readbacks/commits temp/final
children. Existing final or stale probe/temp cleanup failure -> `output_target_invalid`;
unsafe dot path/child -> `path_safety_error`.

## Validation Contract

Primary phase order is fixed and first failing phase wins:

| Phase | Exit | Owner |
|---:|---:|---|
| 1 | 2 | argv, literal path values, repo/output-root safety, capability probe |
| 2 | 2 | `bundle.json` UTF-8/JSON/duplicate/top object plus `bundle_id`, `decision_path`, `files[].path`, fixture identity, final-target/stale-temp cleanup |
| 3 | 2 | listed path order, physical file set, forbidden bundle files, child path safety |
| 4 | 6 | schema generation, package imports/versions/probe, proto availability, selected TRACE key format |
| 5 | 2/4 | full `bundle.json` canonical/closed schema/media type/evidence root |
| 6 | 4 | raw SHA-256 for listed non-`bundle.json` files |
| 7 | 2 | non-bundle JSON syntax, duplicate keys, privacy scan, canonical JSON, local schemas, trust-store schema |
| 8 | 2/4 | `data_b64`, signed-byte reconstruction, CloudEvent canonical hash |
| 9 | 3/4 | trust key selection, authorization, Ed25519 signature, key time window |
| 10 | 4 | lifecycle and policy/tool/build binding checks |
| 11 | 2/5 | TRACE, Agent Manifest, spendguard-artifacts, verification report, readback, final commit |

`bundle.files`: phase 2 owns array/object shape and `path` string only; phase 3
owns path order/membership/physical files; phase 5 owns `media_type`/`sha256`
schema/profile; phase 6 owns digest comparison.

Multi-phase code mapping is fixed: `output_target_invalid` reports phase 1 for
repo/supplied-root/probe failures, phase 2 for final-exists or stale-temp cleanup,
and phase 11 for readback/commit/cleanup. `schema_invalid` and
`canonical_json_invalid` report phase 5 for `bundle.json`, phase 7 for non-bundle
input/trust-store JSON, and phase 11 for generated outputs. `path_safety_error`
reports the phase that first touches the unsafe checked path.

Verifier/private failure stream: exactly one compact diagnostic JSON to stderr,
empty stdout. Keys are `schema,exit_code,phase,code,message,json_pointer`; schema
is `spendguard-agentrust-diagnostic/v1`; `phase` is `phase-<n>`; `message` is the
code with underscores replaced by spaces; pointer is RFC 6901 or null. Success
public verifier emits one compact pass JSON stdout and empty stderr. Public
verifier never emits `packaging_guard_failed`. Make setup/final exit 6 uses the
same compact diagnostic with `phase=make-setup|make-final`; make aggregate exit 1
writes empty stdout and one compact make-summary JSON stderr with `report_path`.

Diagnostic codes by family: CLI/path/output `cli_parse_error`,
`cli_missing_value`, `cli_missing_required_flag`, `cli_value_invalid`,
`cli_equals_form_unsupported`, `unsupported_capability`, `path_safety_error`,
`output_target_invalid`; package prereq `package_missing`,
`package_version_mismatch`, `package_probe_drift`, `schema_source_mismatch`,
`trace_key_invalid`; make-only `packaging_guard_failed`; bundle/schema/hash
`bundle_index_invalid`, `file_set_invalid`, `fixture_privacy_violation`,
`schema_invalid`, `canonical_json_invalid`, `media_type_invalid`,
`file_hash_mismatch`, `evidence_bundle_hash_mismatch`, `data_b64_mismatch`,
`cloudevent_canonical_hash_mismatch`, `trust_store_invalid`; trust/signature
`trust_key_not_found`, `trust_authorization_failed`, `trust_key_time_mismatch`,
`spendguard_signature_invalid`; lifecycle/binding `lifecycle_mismatch`,
`policy_binding_mismatch`, `tool_binding_mismatch`, `build_binding_mismatch`;
output `trace_package_validation_failed`, `manifest_package_validation_failed`,
`manifest_layout_invalid`, `generated_output_schema_invalid`.

Intra-phase precedence is deterministic. `make-setup` sorts before phase 1 and
`make-final` sorts after phase 11. After phase-specific parser precedence, emit
the failing diagnostic with the lowest tuple
`(phase_rank, precedence_index, json_pointer_bytes)`, where null pointer sorts
last. Precedence index follows this list: `cli_missing_value`,
`cli_parse_error`, `unsupported_capability`, `cli_equals_form_unsupported`,
`cli_missing_required_flag`, `cli_value_invalid`, `path_safety_error`,
`output_target_invalid`, `packaging_guard_failed`, `package_missing`,
`package_version_mismatch`, `package_probe_drift`, `schema_source_mismatch`,
`trace_key_invalid`, `bundle_index_invalid`, `file_set_invalid`,
`fixture_privacy_violation`, `schema_invalid`, `canonical_json_invalid`,
`media_type_invalid`, `file_hash_mismatch`, `evidence_bundle_hash_mismatch`,
`data_b64_mismatch`, `cloudevent_canonical_hash_mismatch`,
`trust_store_invalid`, `trust_key_not_found`, `trust_authorization_failed`,
`trust_key_time_mismatch`, `spendguard_signature_invalid`,
`lifecycle_mismatch`, `policy_binding_mismatch`, `tool_binding_mismatch`,
`build_binding_mismatch`,
`trace_package_validation_failed`, `manifest_package_validation_failed`,
`manifest_layout_invalid`, `generated_output_schema_invalid`. Codes not listed
in the diagnostic family registry are Spec defects.

Exit map: exit 2 for CLI/path/output, bundle/file/privacy, local schema,
canonical JSON, media type, `data_b64_mismatch`, trust-store shape, and late
commit errors; exit 3 for key-not-found, authorization, and signature invalid;
exit 4 for raw hash, evidence root, CloudEvent canonical hash, key time,
lifecycle, policy/tool/build binding mismatch; exit 5 for phase-11 generated
package/schema validation; exit 6 for package prereq and make setup/final guard.

Canonical JSON is `SG-C14N`: UTF-8, closed JSON, no duplicate keys, sorted object
keys by bytewise lexicographic order, arrays in order, shortest valid JSON
numbers, LF-terminated files where specified, no Unicode normalization.
CloudEvent signature vector and checker in payload are mandatory executable
contracts.

## Generated Outputs And Make Continuation

Verifier selected outputs are SG-C14N without LF; make report uses exact payload
order and LF below. All validate against embedded schemas/probe behavior. Public
verifier never reads expected-output files. Boundary table:

| Case | Result |
|---|---|
| committed expected-output file or generated-output manifest SHA mismatch before runnable checks | make setup exit 6 `schema_source_mismatch` |
| verifier-generated bytes differ from committed golden after phase 11 | make aggregate exit 1 with `generated_output_schema_invalid` |
| phase-11 generated schema/package validation failure | verifier exit 5; make aggregate exit 1 before golden comparison |

Make exit 1 records `status=fail` and continues for allow/deny diagnostics,
parser/help/issuer miss, negative mismatch, malformed negative stderr, unexpected
negative exit/signal, or golden mismatch unless run root becomes unsafe. Make exit
6 aborts for setup/preflight artifact, package/schema-source/trace-key/package
guard, unsafe root/cleanup, or final package guard. Existing exit-6 report is stale.

Make report is closed compact JSON+LF at
`<fresh-out-dir>/verify-agentrust-report.json`; it is not SG-C14N. Top-level key
order is exact payload order:
`conformance_checks,mode,negative_tests,package_versions,schema,success_fixtures,status`.
`schema` is `spendguard-agentrust-make-verification-report/v1`; `status` is
`pass|fail`; `mode` is `fixture`. `conformance_checks` keys are
`artifact_preflight_envelope,event_type_binding,fixture_identity,internal_guard,manifest_issuer_variant,package_guard_surfaces,private_parser_matrix,public_parser_matrix`,
each `diagnostic_code,status`. `success_fixtures` keys are `allow,deny`, each
`bundle_id,diagnostic_code,outputs_match,status`. `negative_tests` keys are
`missing_trace_signature,mutated_policy_hash,mutated_signature,wrong_key`, each
`diagnostic_code,expected_exit_code,observed_exit_code,outputs_absent,status`.
Malformed report schema/key/order is `generated_output_schema_invalid`. Exact help
stdout is `usage: python -m spendguard_agentrust verify --bundle BUNDLE --trust-store TRUST_STORE --trace-signing-key TRACE_SIGNING_KEY --out-dir OUT_DIR [--mode fixture] [--fixture-profile upstream] [--manifest-issuer SPIFFE_URI]\n`.
Pass stdout is SG-C14N no-LF JSON with keys
`bundle_id,outputs,schema,status`, schema `spendguard-agentrust-verify-result/v1`,
and output filenames array in selected-output order. Exit 1 make-summary stderr is
compact JSON+LF keys `report_path,schema,status`, schema
`spendguard-agentrust-make-summary/v1`, status `fail`.

`<fresh-out-dir>` is `<repo-root>/integrations/agentrust/out/.verify-agentrust/<run-id>`.
Negative output absence scan root is exactly `<mutation-out-root>`. Walk
no-follow; symlinks count as present and are not followed. `outputs_absent=false`
if any `.probe-*`, `.tmp-*`, final `<bundle_id>` directory, selected output file
(`trace-level0.json`, `agent-manifest.json`, `spendguard-artifacts.json`), or
`verification-report.json` exists under that root. Diagnostic files under
`<fresh-out-dir>/.diagnostics/**` are outside the scan root.

## Glossary

`package-public archive/help surfaces`: wheel/sdist names/bytes, entry points,
scripts, public package docs/help. `declared container context tars`: build-config
listed tars only. `public submission docs`: the three public docs named in Scope.
`authority conflict`: mismatch among packet prose, embedded metadata/bytes, schema/
probe/vector/checker/fixture/expected-output evidence, or command evidence.
`selected outputs`: `trace-level0.json`, `agent-manifest.json`,
`spendguard-artifacts.json`.

## Gate Clarifications

Canonical artifact is this UTF-8 packet and sole `PAYLOAD_B64`. Decoded JSON also
lives at `docs/specs/agentrust-required-artifact-evidence-2026-07-01.json`; file
SHA must equal `json_sha256`, and approval requires review-log
`embedded_fresh_replay=pass`. First-class targets: 5 `docs/specs/agentrust-*.json`,
`proto/.../common.proto`, checker, fixture/generated manifests, 9 expected outputs,
lock. Replay materializes only those 19 files; checker reads no checkout-only
files. Formats path-map to JSON/proto/Python/requirements. Prose extraction compares
only backticked repo paths, `==` pins, `sha256[:=]` hashes, fixture ids
`allow|deny|demo-allow|demo-deny`, mutation names, and generated-manifest filenames.
Path classes: payload-bound=19 artifacts; live-only=public docs/help/package/context;
generated/run/glob/placeholders are never payload constants.

Authority drift split: payload path/size/SHA or prose-vs-payload mismatch rejects
this packet with `{approved:false,suggestions:[...]}`. Runtime source drift:
schema/fixture/committed-golden -> `schema_source_mismatch`; vector/checker/probe
-> `package_probe_drift`; lock -> `package_version_mismatch`; fixture key ->
`trace_key_invalid`; package/public leak -> `packaging_guard_failed`; make-only
verifier-vs-golden after phase 11 -> `generated_output_schema_invalid`.

Parser candidates are token-classified once. Row 1 wins by earliest supported value
flag missing/flag-valued value. Let `u` be earliest row-3 unsupported trigger.
Row 2 wins only if its structural token index is `<u` or no `u` exists. If `u`
exists, row 3 wins over all row 4-8 candidates, including earlier unknown flags;
`--tested-against` consumes at most one following non-flag. Else rows 4-8 apply
by row then token. Examples: `--unknown --force` and `--force --unknown` ->
`unsupported_capability`; `--tested-against --unknown`, `--mode live extra`, and
`--force --force` -> first unsupported token; `extra --force` -> `cli_parse_error`.

FS algorithm is dirfd-only: dirs `O_RDONLY|O_DIRECTORY|O_CLOEXEC|O_NOFOLLOW`,
files `O_RDONLY|O_CLOEXEC|O_NOFOLLOW`, `fstat` after open, probes/temp exclusive
create, promote only by `renameat2(RENAME_NOREPLACE)`. `ENOENT,ELOOP,ENOTDIR,EXDEV,EEXIST`
on safety invariants -> current phase path/output diagnostic; `EACCES,EPERM,EBUSY,EIO,ENOSYS,EINVAL`
in env/probe/cleanup -> `output_target_invalid`; no fallback path API.

Scan enumerators: wheel/sdist archive directory entries; installed dist metadata,
entry scripts, public package docs/help; explicit context tar entries; generated
outputs; three public docs. Symlink entries, unsafe member names, non-UTF8 required
text, sourcemaps, nested archives, internal docs, `docs/specs/**`, or this bundle
in shipped/context artifacts fail `packaging_guard_failed`; binary allowed members
scan name only.
Build/inspect sequence: delete `build,dist,*.egg-info`; run
`python -m build --sdist --wheel`; inspect archives in bytewise member order;
install wheel into clean venv with `PYTHONNOUSERSITE=1`; discover console scripts
from `entry_points.txt`, `RECORD`, and venv `bin/spendguard*`.

Command exits: public/private verifier emit one diagnostic and stop; public never
emits `packaging_guard_failed`. Make setup exit 6 aborts for prereq, guard, drift,
unsafe root, or trace key. Make aggregate exit 1 records parser/help/issuer/allow/
deny/negative/golden failures and continues unless root safety fails. Make final
exit 6 covers final guard/report schema. Pass diagnostics null; fail records observed.
Security checks deny `_verify_internal` in public artifact/help/scripts, private
PEM bytes or unallowlisted `*.test.ed25519.pem`, absolute sensitive paths in
diagnostics/logs, and env influence except documented
`SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT`, `PYTHONPATH`, `PYTHONNOUSERSITE`.
PEM/secret scan runs on raw bytes of every wheel/sdist/context/generated member
before text/binary classification; binary opacity applies only to claim text scan.

SG-C14N numbers are integers only: grammar `0|-?[1-9][0-9]*`, `-0`, fractions,
exponents, NaN, and Infinity invalid; range is signed 64-bit unless schema narrows.
Generated outputs preserve raw issuer; no percent re-encode. Linux CI must first
pass smoke checks `id -u`, mountinfo fstype, owner, xattr listing, ACL absence,
and `renameat2`; use a non-root Ubuntu 24.04/Debian 12 VM on ext4/xfs/btrfs/zfs,
not overlay/idmapped/userns. Otherwise setup exit 6. Probe temp paths,
`module_file`, Python minor, and lock generation comment/platform are provenance
only; drift compares versions, exports, signatures, accepted shapes, names, pins,
and hashes. TRACE L0 means software-only fixture trace; CMCP/MCP/live/historical/
SLSA claims are forbidden public claim boundaries for Community submission.
Structural keys `replay_cache` and `replay_cache_report` are allowed only in the
embedded schema and verification-report golden with status `not_applicable_fixture`;
public claim text `replay-cache` remains forbidden.

## Evidence Payload

Decoded manifest summary: schema `spendguard-agentrust-review-evidence/v1`, 19
SHA-pinned artifacts, and two replayed commands: `shasum -a 256` over every
artifact path in payload order, and `python3 docs/specs/check-agentrust-vectors.py`.

The following line is `base64(xz(JSON))`. `json_sha256=f7e6bee25cbfc67616d56b38ff9c4a07e559177d8299124db661f829aba3d9e9`;
`xz_sha256=65817fa8319be60c2941ac546d379697b6dcc00f1a3cd7fa3ab79c8a3f522f6d`; `payload_b64_sha256=c84bb68e83c786b436186ba3605567a7ac5440daa67164ed8a092f171b24edfa`.
Decode and verify with:

```bash
python3 - <<'PY'
import base64,hashlib,json,lzma,os,pathlib,re
p=pathlib.Path(os.environ.get('PACKET','docs/specs/agentrust-integration-spec-review-bundle-v1alpha1.md'))
s=p.read_text()
decl=re.search(r'json_sha256=([0-9a-f]+).*xz_sha256=([0-9a-f]+).*payload_b64_sha256=([0-9a-f]+)',s,re.S).groups()
b64=re.search(r'^PAYLOAD_B64:([A-Za-z0-9+/=]+)$',s,re.M).group(1)
assert hashlib.sha256(b64.encode()).hexdigest()==decl[2]
xz=base64.b64decode(b64); assert hashlib.sha256(xz).hexdigest()==decl[1]
raw=lzma.decompress(xz); assert hashlib.sha256(raw).hexdigest()==decl[0]
e=json.loads(raw)
assert e['schema']=='spendguard-agentrust-review-evidence/v1'
for a in e['artifacts']:
 b=base64.b64decode(a['bytes_b64']); assert len(b)==a['size']; assert hashlib.sha256(b).hexdigest()==a['sha256']
expected=[['shasum','-a','256']+[a['path'] for a in e['artifacts']],['python3','docs/specs/check-agentrust-vectors.py']]
assert [c['argv'] for c in e['commands']]==expected
for c in e['commands']:
 out=base64.b64decode(c['stdout_b64']); err=base64.b64decode(c['stderr_b64'])
 assert c['exit']==0 and len(out)==c['stdout_len'] and len(err)==c['stderr_len']
 assert hashlib.sha256(out).hexdigest()==c['stdout_sha256']; assert hashlib.sha256(err).hexdigest()==c['stderr_sha256']
print('embedded_evidence=pass')
PY
```

PAYLOAD_B64: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
