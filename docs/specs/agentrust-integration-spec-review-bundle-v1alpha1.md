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

The following line is `base64(xz(JSON))`. `json_sha256=686255e01f317727523a5afb02b6447d3f5278d1adc190e75e6b43f9beb68f8d`;
`xz_sha256=57faa6feb741e2f09ed7c8a4c0a9e449ea5f84e5569f74bbf18f7f8a522ba931`; `payload_b64_sha256=f4962e71738eb5f70711951b742c98559d988472efd256598e49943c3e273609`.
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
