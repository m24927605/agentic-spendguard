# AgenTrust integration spec v1alpha1

Status: docs-first design, not implemented.

## Objective

Publish an AgenTrust Community integration proving that committed synthetic
Agentic SpendGuard allow and deny evidence can be converted into TRACE Level 0
software-only records signed with a committed test exporter key and into an
unsigned Agent Manifest containing artifact bindings.

Submission target: `agentrust-io/integrations` Community tier.

## Scope

Normative v1alpha1 conformance is `community-submission`.

`community-submission` means fixture allow/deny evidence bundles, TRACE Level 0
software-only records, an unsigned Agent Manifest, package probe evidence,
deterministic vectors, compact schema source, and offline `make
verify-agentrust`.

The only conformant execution mode is fixture upstream:

```bash
python -m spendguard_agentrust verify \
  --mode fixture \
  --fixture-profile upstream \
  --bundle integrations/agentrust/examples/{allow|deny} \
  --trust-store integrations/agentrust/examples/trust-store.json \
  --trace-signing-key integrations/agentrust/test-private-keys/spendguard-agentrust-fixture.test.ed25519.pem \
  --out-dir integrations/agentrust/out
```

`--bundle` is exactly one of `integrations/agentrust/examples/allow` or
`integrations/agentrust/examples/deny`. v1alpha1 has no per-bundle subdirectory
profile.

The phrase "execution mode" above refers to evidence semantics: fixture evidence
with upstream AgenTrust Community packages. v1alpha1 conformance has two command
surfaces: the public single-bundle verifier shown above, and the private
`_verify_internal` entry point used only by `make verify-agentrust` for isolated
success runs and negative tests. `_verify_internal` is part of the make-level
conformance contract, not a supported user-facing capability.

Terminology: public CLI, private internal verifier, setup/prerequisite checks,
public output, make scratch, scanned members, unsafe archive paths, source fixture
key, and `<trace-key-copy>` are defined by the command, guard, manifest, and path
safety rules below.

v1alpha1 must not claim AgenTrust Verified status, TRACE Level 1/2, hardware
attestation, signed Agent Manifest verification, cMCP gateway integration, live
operator verification, historical replay, replay-cache protection, conversion
record trust, or production key distribution.

## Make Conformance Summary

`make verify-agentrust` is part of v1alpha1 conformance. It creates a fresh
internal output root at
`integrations/agentrust/out/.verify-agentrust/<run-id>` and executes this fixed
workflow:

| Step | Required contract |
|---:|---|
| 1 | Run make-level SHA/vector, lock/offline, and package/container guard checks. Per-invocation package/probe/key checks still repeat after phase-3 fixture intake. |
| 2 | Verify the public `allow` fixture. |
| 3 | Verify the public `deny` fixture. |
| 4 | Write per-fixture success outputs only for the two public fixture runs. |
| 5 | Run exactly four make-only negative invocations in the order below. |

Required negative invocations:

| Order | Mutation | Fixture | Expected exit | Expected diagnostic |
|---:|---|---|---:|---|
| 1 | `mutated_signature` | `allow` | 3 | `spendguard_signature_invalid` |
| 2 | `wrong_key` | `deny` | 3 | `spendguard_signature_invalid` |
| 3 | `mutated_policy_hash` | `allow` | 4 | `policy_binding_mismatch` |
| 4 | `missing_trace_signature` | `deny` | 5 | `trace_package_validation_failed` |

Success fixture outputs are written under `<fresh-out-dir>/<bundle_id>/`.
The make aggregate report is
`<fresh-out-dir>/verify-agentrust-report.json`; optional diagnostic copies are
under `<fresh-out-dir>/.diagnostics/`; mutation working copies are under
`<fresh-out-dir>/.mutations/`. Each negative uses its own
`<mutation-out-dir>` under that tree, so existing success outputs cannot cause an
early final-target collision. Mutation working copies and mutation output roots
are removed best-effort before make returns.
The make target exits 0 only when setup checks, both success fixtures, every
negative mutation, and every output-absence check pass. If an aggregate report can
be written and any required entry fails or is unexpectedly `not_run`, make exits
1 after completing all runnable mutations. Setup failures before aggregate report
creation emit the same single stderr diagnostic JSON shape as verifier failures
and exit with the setup failure code.

The private `_verify_internal` entry point exists only for this make workflow.
Make invokes it from the source checkout with that checkout on `PYTHONPATH`, not
from an installed wheel or sdist. It must require
`SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT=1` and must not be exposed by installed
console scripts, production entry points, public help, docs, or declared
container contexts. Module presence inside a built artifact is allowed only when
the runtime gate below remains enforced and no public surface exposes it.
`_verify_internal` must also abort unless the loaded package and module realpaths
are inside the source checkout, not site-packages, a wheel, zipimport, or an
installed sdist.

## CLI Parse Order

The verifier parses CLI arguments before importing packages or reading paths.
Public grammar is exactly `python -m spendguard_agentrust verify [flags]`.
`verify` is the only public subcommand. Supported flags are `--bundle`,
`--trust-store`, `--trace-signing-key`, `--out-dir`, `--mode fixture`,
`--fixture-profile upstream`, and `--manifest-issuer`.

Token diagnostics run in this fixed order:

| Order | Condition | Diagnostic |
|---:|---|---|
| 1 | missing `verify` or unsupported subcommand | `cli_parse_error` |
| 2 | flag before `verify` | `cli_parse_error` |
| 3 | explicit unsupported capability lexeme, including `--mode=live`, `--mode=historical`, `--force`, or repeated unsupported flags | `unsupported_capability` |
| 4 | unknown flag name, including unknown `--flag=value` | `cli_parse_error` |
| 5 | duplicate supported flag | `cli_parse_error` |
| 6 | supported `--flag=value`, including `--bundle=...`, `--fixture-profile=upstream`, `--mode=fixture`, or `--manifest-issuer=...` | `cli_equals_form_unsupported` |
| 7 | supported flag missing its separate value token or followed by another flag | `cli_missing_value` |
| 8 | unsupported separate-token capability value, such as `--mode live`, `--mode historical`, or `--fixture-profile local` | `unsupported_capability` |
| 9 | omitted required supported flag: `--bundle`, `--trust-store`, `--trace-signing-key`, or `--out-dir` | `cli_missing_required_flag` |
| 10 | invalid supported value after token parsing, including malformed `--manifest-issuer` | `cli_value_invalid` |

The parser scans the full token list and reports the lowest-order matching row,
not the first left-to-right error. Duplicate detection tracks supported flags only
after unsupported/unknown classification. Therefore `--bundle a --bundle b
--mode=live` reports `unsupported_capability`, `--bundle a --bundle b --mode
live` reports `cli_parse_error`, `--bundle=foo --mode live` reports
`cli_equals_form_unsupported`, and `--unknown=live --mode=live` reports
`unsupported_capability`. Boolean-like unsupported flags such as `--force`,
`--cmcp-gateway`, `--mcp-gateway`, or `--tested-against` fail on the flag token;
any following token is ignored for precedence.

Validation order after successful parsing is:

1. Path safety, CLI value validation, output-root checks, output-root
   capability probe, and output target checks that do not require parsed bundle
   contents. The probe uses `.tmp-probe-<nonce>` under the effective output root
   to prove native non-replacing same-directory directory rename and cleanup;
   unsupported filesystems, `EXDEV`, missing syscalls, or cleanup failure exit 2
   with `output_target_invalid`.
2. `bundle.json` index parse for ordering only: parse UTF-8 JSON with
   duplicate-key rejection and require a top-level object with a string
   `bundle_id` matching the `bundle_id` token plus a `files` array whose entries
   are objects exposing string `path`, `media_type`, and `sha256` members.
   Non-object entries, duplicate keys inside file entries, missing members,
   malformed `bundle_id`, or non-string member values exit 2 in this phase. Once
   `bundle_id` is available, validate `--out-dir/<bundle_id>` absence and clean
   stale temp directories for that bundle. This phase does not perform
   closed-schema, canonical-byte, hash,
   media-type/profile, lifecycle, trust, or artifact-binding validation.
3. File-set and forbidden-file checks comparing actual bundle entries to
   `bundle.json.files`. The `examples/allow` root requires listed
   `outcome.json`; the `examples/deny` root requires it absent and unlisted.
4. Schema-source SHA/generation checks, AgenTrust package imports,
   pinned-version checks, package probe drift checks, and TRACE key format
   checks.
5. Full `bundle.json` schema/canonical checks and evidence-bundle root.
6. Raw file hash checks for listed non-`bundle.json` files, before parsing those
   files.
7. Privacy scanning, JSON syntax, duplicate-key rejection, canonical JSON, local
   closed-schema validation, media-type/profile checks, and full trust-store
   validation. For `decision.json` and `outcome.json`, this includes wrapper
   schema, `spendguard_cloudevent`, and event-type `data_json` payload schema.
8. Procedural signed-event checks not expressible in local JSON Schema:
   `data_b64` byte comparison, canonical signed-byte reconstruction, and
   `canonical_bytes_sha256` comparison.
9. SpendGuard trust-store key selection, authorization, selected-key time-window
   check, and signature verification.
10. Lifecycle derivation and signed artifact-binding checks.
11. TRACE and Agent Manifest generation, generated output validation, and output
   commit.

Thus malformed CLI/path/output/bundle-index/file-set input exits 2 before missing
packages are reported. Missing packages exit 6 after phase-3 intake but before
raw hashes, privacy scanning, full schema, trust, lifecycle, output validation,
or selected output writes.

Defaults are explicit: omitted `--mode` means `fixture`; omitted
`--fixture-profile` means `upstream`; omitted `--manifest-issuer` means
`spiffe://spendguard.local/agentrust-exporter`. `--manifest-issuer`, when
supplied, must be an absolute `spiffe://` URI without query or fragment and must
not be used as a trust input.

`--manifest-issuer` grammar is byte-for-byte and performs no URI
normalization:
`^spiffe://[a-z0-9](?:[a-z0-9.-]{0,251}[a-z0-9])?(?:/[A-Za-z0-9._~!$&'()*+,;=:@-]+)*$`,
maximum 256 bytes, no query, no fragment, no percent encoding, no ASCII control
characters, lowercase scheme, lowercase trust domain, no repeated slash, no empty
path segment, and no trailing slash. Uppercase trust-domain bytes, percent
sequences such as `%2f`, and equivalent-looking normalized forms are rejected
rather than normalized. Violations exit 2 during CLI value validation in phase 1
after token parsing; unsupported flags, duplicate flags, equals-form errors, and
missing values still take precedence.

Examples: `spiffe://spendguard.local/agentrust-exporter` and
`spiffe://tenant.example/workload/agent-1` pass; trailing slash, repeated slash,
percent encoding, uppercase scheme, or uppercase trust-domain bytes fail.

Unsupported flag values and unsupported capability flags include `--mode live`,
`--mode=live`, `--mode historical`, `--mode=historical`,
`--conversion-public-key`, `--historical-replay-at`, `--max-evidence-age`,
`--replay-cache`, `--replay-cache-ttl-days`, `--unsafe-no-replay-cache`,
`--force`, `--manifest-signing-key`, `--manifest-trust-context`,
`--cmcp-output`, `--cmcp-gateway`, `--cmcp-manifest`, `--mcp-gateway`, and
`--tested-against`.

Both `--cmcp-*` and `--mcp-*` gateway aliases are rejected because v1alpha1 does
not claim cMCP or MCP gateway integration.

## Path Safety

v1alpha1 path handling is POSIX-only. Repository root is the canonical output of
`git rev-parse --show-toplevel`, resolved with strict realpath semantics. The
verifier must not derive repository root from the current working directory or
from CLI path prefixes.

Every public user-supplied CLI path is rejected before joining to the repository
root if it is empty, absolute, contains NUL, backslash, repeated separators, `.`
or `..` segments, or any hidden segment beginning with `.`. This hidden-segment
ban applies to CLI arguments and bundle entry names only. It does not apply to
verifier-created reserved children under an already validated public `--out-dir`,
such as `.tmp-<bundle_id>-<nonce>`, because those names are never accepted as
public CLI input. After joining, the
implementation resolves each existing ancestor with `lstat`/realpath semantics
and rejects any symlink component. The final repo-relative canonical path must be
byte-identical to one allowed public path below; case-folded or
Unicode-normalized equivalence is not accepted. Violations exit 2.

All public CLI path arguments must be repository-relative POSIX paths before
resolution; absolute `--bundle`, `--trust-store`, `--trace-signing-key`, and
`--out-dir` values fail exit 2 even if they resolve under the repository root.
Implementations must `lstat` each path component before following it; resolving a
symlink and then comparing the resolved path to the allowlist is non-conformant.

Allowed resolved paths:

| Flag | Allowed path |
|---|---|
| `--bundle` | Exactly `integrations/agentrust/examples/allow` or `integrations/agentrust/examples/deny`. |
| `--trust-store` | Exactly `integrations/agentrust/examples/trust-store.json`. |
| `--trace-signing-key` | Public CLI: exactly the manifest entry with `required_by="trace_test_key"` and `role="trace_signing_private_key"`; `wrong_key_negative` entries are never valid signing keys. Internal make: exactly `<trace-key-copy>`, copied from that manifest key after SHA-256 verification. |
| `--out-dir` | Public CLI: exactly `integrations/agentrust/out`. Internal make success: `<fresh-out-dir>`. Internal make negative invocations: `<mutation-out-dir>`. Internal paths require `_verify_internal`, `SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT=1`, and the internal hidden-path grammar. |

The internal make harness uses a separate path grammar because its workspace is a
make-owned scratch area under the public output root. Internal paths are allowed
only when both conditions are true: the private internal entry point is used and
`SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT=1` is present. Internal paths still must
be repository-relative POSIX paths with no NUL, backslash, repeated separators,
`.` segments, `..` segments, symlink component, or absolute prefix. Hidden
segments are allowed only for these exact make-owned names:
`.verify-agentrust`, `.mutations`, `.diagnostics`, `.trace-key`, and
`.tmp-<bundle_id>-<nonce>`.

Internal path variables use these grammars:

```text
<run-id> = verify-<nonce>
<mutation> = mutated_signature | wrong_key | mutated_policy_hash | missing_trace_signature
<fixture> = allow | deny
<nonce> = 32 lowercase hex characters generated from at least 128 bits of OS CSPRNG entropy
<fresh-out-dir> = integrations/agentrust/out/.verify-agentrust/<run-id>
<mutation-bundle-root> = <fresh-out-dir>/.mutations/<mutation>/<fixture>/bundle
<mutation-trust-store> = <fresh-out-dir>/.mutations/<mutation>/<fixture>/trust-store.json
<mutation-out-dir> = <fresh-out-dir>/.mutations/<mutation>/<fixture>/out
<trace-key-copy> = <fresh-out-dir>/.trace-key/trace-exporter.test.ed25519.pem
```

Within `.mutations`, only `<mutation>/<fixture>/bundle`,
`<mutation>/<fixture>/trust-store.json`, and `<mutation>/<fixture>/out` are valid
descendants; `<mutation>` and `<fixture>` are the enums above. Within
`.trace-key`, only the single `<trace-key-copy>` file is valid.

The internal verifier may accept `--bundle <mutation-bundle-root>`,
`--trust-store <mutation-trust-store>`, and `--out-dir <mutation-out-dir>` for
negative invocations only under that private make entry point. Success internal
invocations use `--out-dir <fresh-out-dir>`. Public
`python -m spendguard_agentrust verify` must reject every hidden segment and
every mutation path above.

`--out-dir` must already exist as a directory; the verifier does not create the
top-level `--out-dir`. Case-insensitive duplicate detection is performed by ASCII
lowercasing the fixed allowed file names; any non-ASCII path byte in a bundle
entry name exits 2 before Unicode normalization questions arise. Every direct
bundle file, trust store, and signing key read by the verifier must be a regular
non-symlink file with `st_nlink == 1`.
Internal mode keeps the same preexistence rule: `make verify-agentrust` creates
`<fresh-out-dir>` before invoking the internal verifier, and the internal verifier
rejects it if it is missing, not a directory, or a symlink. The internal
`<fresh-out-dir>` and every ancestor are subject to the same `lstat` symlink
checks, stale-temp cleanup, final-target checks, and output atomicity rules as the
public `--out-dir`.
These rules are deterministic POSIX fixture hygiene for a local conformance
command, not a security sandbox, not a symlink-race-proof sandbox, and not
protection against concurrent malicious filesystem writers. The attacker model is
limited to accidental local misconfiguration and stale verifier outputs in a
developer checkout. For sensitive deletion and promotion, implementations must
either use fd-relative traversal primitives such as `openat`/`unlinkat` plus an
exclusive native rename primitive, or fail closed with `path_safety_error` or
`output_target_invalid` on platforms where those primitives are unavailable.
The required defense is against stale symlinks, replaced cleanup candidates, and
existing final targets observed by the verifier's own checks. v1alpha1 does not
claim to keep operating safely if an untrusted writer can modify checked
directories between those checks; ownership/mode preconditions below make that an
unsupported checkout.
The verifier must not be used on repositories or output roots writable by
untrusted users. Both public CLI and make check that the repository root,
`integrations/`, `integrations/agentrust/`, every existing ancestor of the
selected bundle root, the selected bundle root and every direct bundle file,
`trust-store.json`, `--trace-signing-key`, the active `--out-dir`, every existing
ancestor of active `--out-dir`, and every directory that may be deleted or
promoted are owned by the current effective UID and are not group-writable or
world-writable. Trust-store and signing-key files must not be group/world-writable;
the selected TRACE signing key and `<trace-key-copy>` must have no group/other
permission bits, and `<trace-key-copy>` must be mode `0600`. Repo root, bundle
root, trust store, TRACE key, effective output root, temp directories, final
output directory, and cleanup candidates must stay on the expected `st_dev`;
device or mountpoint crossings exit 2. Violations exit 2 with
`path_safety_error`. This is a local fixture-conformance precondition, not a
production isolation claim.

Symlink checks apply to every ancestor of `--out-dir`, the `--out-dir` directory
itself, and the final target immediately before rename. After a valid `bundle_id`
is parsed, check `<out-dir>/<bundle_id>` first; if it exists, emit
`output_target_invalid` and do not perform stale-temp cleanup. Only if the final
target is absent may cleanup remove direct `.tmp-<bundle_id>-<nonce>` children.
The implementation opens or creates the sibling temp directory under the already
validated `--out-dir` using an unpredictable `.tmp-<bundle_id>-<nonce>` name and
owner-only permissions where supported. Before atomic rename, it verifies with
`lstat` that the temp path is a directory, not a symlink, and that the final
target does not exist. Any race or permission failure exits 2 and removes the
temp directory on a best-effort basis.

Stale temp cleanup only removes direct children of `--out-dir` whose names match
`.tmp-<bundle_id>-<nonce>`, where `<nonce>` is 32 lowercase hex characters, and
whose `lstat` type is directory. These names are reserved for the verifier inside
the dedicated output root; a matching directory may be removed even if this
process did not create it. Symlinks or other file types matching that name cause
exit 2 and are not followed.
Recursive deletion of a matching stale temp directory walks bottom-up with
`lstat` on every entry and `followlinks=false`. Encountering a symlink, device
change, or entry whose type changes during cleanup exits 2 without following or
deleting the changed entry. Every nested entry must be owned by the current
effective UID and not group/world-writable before deletion; unreadable entries,
permission errors, or deletion failures exit 2.

`<run-id>` for the internal make harness is exactly `verify-<nonce>` and is not
derived from any fixture `bundle_id`. Without
`SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT=1`, any `--out-dir` other than
`integrations/agentrust/out` fails exit 2. This environment variable is accepted
only by `make verify-agentrust`; production/package entry points must not
document or enable it.

## Input Bundle

The input bundle contains exactly these files:

- `bundle.json`
- `decision.json`
- optional `outcome.json`
- `policy-bundle.json`
- `tool-catalog.json`
- `build-provenance.json`

`conversion-record.json` is forbidden and fails exit 2 if listed or present.
v1alpha1 does not ingest live or historical SpendGuard exports.
The public allow fixture `bundle_id` is `demo-allow`; the public deny fixture
`bundle_id` is `demo-deny`. They are intentionally distinct so success outputs
under one `<fresh-out-dir>` cannot collide.
Each input JSON file must be at most 1 MiB; a bundle may contain only the six
allowed direct JSON filenames above and no subdirectories.

All fixture files are committed synthetic examples. They must not contain real
tenant names, customer prompts, API keys, bearer tokens, cookies, passwords,
mTLS/private key material, raw tool arguments, raw tool results, or proprietary
policy contents. Tenant UUIDs in fixtures are allowed only when generated as
synthetic identifiers for this test data. The deterministic scanner does not
infer UUID provenance; fixture authors must not copy real customer or tenant UUIDs.

Fixture privacy checks are deterministic and intentionally shallow for
v1alpha1. They apply only to UTF-8 JSON files in the input bundle and
`trust-store.json` after duplicate-key rejection. The scanner does not run an
unstructured regex over raw JSON text; it separately checks parsed property names
and parsed string values.

Privacy scan order is deterministic: `bundle.json`, `decision.json`,
`outcome.json` if listed, `policy-bundle.json`, `tool-catalog.json`,
`build-provenance.json`, then `trust-store.json`. The scanner recursively visits
every object property name and string leaf. Within one file, diagnostics sort by
RFC 6901 JSON Pointer. Regex matching uses ASCII case-folding, no Unicode
normalization, and unanchored search unless explicitly anchored. The
property-name allowlist is exact and case-sensitive and bypasses property-name
checks only.

Forbidden property-name patterns are case-insensitive; a matching property name
fails immediately regardless of its value or descendants:
`api[_-]?key`, `authorization`, `cookie`, `password`, `secret`, `token`,
`private[_-]?key`, `client[_-]?secret`, `customer[_-]?prompt`,
`raw[_-]?tool[_-]?(args|arguments|result|results)`, and
`tenant[_-]?(name|email|domain)`. Explicit schema allowlist names win for property
names only: `public_key_base64url`, `key_id`, `allowed_tenant_ids`,
`allowed_producer_ids`, and `allowed_subject_prefixes`.
The allowlist bypasses only the property-name regex. For example, a
`public_key_base64url` property name is allowed, but its string value is still
checked against forbidden string-value patterns; strings inside
`allowed_tenant_ids` arrays are also still scanned.

Forbidden string-value patterns are case-insensitive: `bearer `,
`sk-[A-Za-z0-9_-]{20,}`, `BEGIN .*PRIVATE KEY`, and cookie-pair strings matching
`(^|;\\s*)[A-Za-z0-9_.$-]{1,64}=[^;\\s]{16,}`. The property-name allowlist never
exempts string values. v1alpha1 does not perform entropy detection, binary
scanning, or nested/base64 decoding. The scanner does not scan the TRACE exporter
PEM bytes; PEM placement is enforced by path safety and packaging checks below.
For the `BEGIN .*PRIVATE KEY` pattern, `.` does not match newline; multiline
private-key values are still caught by matching the first line containing
`BEGIN ` and `PRIVATE KEY`.
The scanner walks parsed JSON recursively through objects and arrays after JSON
escape decoding. Non-string scalars are ignored. Any privacy scanner hit exits 2
with diagnostic code `fixture_privacy_violation`.

Privacy scanning runs in phase 7 after raw file hash checks for listed bundle
files and before canonical-byte/full-schema validation for any file whose hash
matched. Within phase 7, file order is primary; inside one file, JSON parse and
duplicate-key rejection run first, privacy hits then beat canonical-byte, schema,
media-type/profile, and trust-store semantic diagnostics, and privacy hits sort
by JSON pointer. Thus an earlier file's canonical/schema error beats a later
file's privacy hit. A phase-6 hash mismatch suppresses all phase-7 diagnostics
for that listed file.

The only PEM files allowed in the repository for this integration are test TRACE
exporter keys under `integrations/agentrust/test-private-keys/*.test.ed25519.pem`.
`make verify-agentrust` must run a packaging/container guard that fails if any
`*.test.ed25519.pem` path, exact allowlisted source PEM bytes, or literal
private-key PEM-looking material are present in final generated output
directories, built Python wheels or sdists, declared container build-context
tars, or tracked and unignored repository paths other than the allowlisted source
fixture paths. The temporary `<trace-key-copy>` is a checked make input, not a
generated output, and must be deleted before final generated-output scanning.

## Canonical JSON And Bundle Root

All SpendGuard-owned input and generated JSON files, except `trace-level0.json`
and `agent-manifest.json`, must be UTF-8 RFC 8785 canonical JSON bytes with no
trailing newline. Parsers must reject duplicate keys before value validation.
Unknown fields fail validation. RFC 8785 outputs use UTF-8 JSON bytes; fixture
values are ASCII-only by policy. `trace-level0.json` and `agent-manifest.json`
are AgenTrust package objects and use their section-specific serialization rules.

`bundle.json` validation order is:

1. Parse `bundle.json` with duplicate-key rejection.
2. Validate the closed `bundle` object shape.
3. Compare raw `bundle.json` bytes against RFC 8785 canonical serialization of
   the full object including `evidence_bundle_hash`; mismatch exits 2.
4. Remove top-level `evidence_bundle_hash`, canonicalize the remaining object,
   compute SHA-256, and compare to `evidence_bundle_hash`.

Malformed or noncanonical `bundle.json` exits 2. Canonical `bundle.json` with the
wrong evidence-bundle root exits 4.

`bundle.json` contains exactly these required fields and no others:

| Field | Contract |
|---|---|
| `version` | `spendguard-agentrust-evidence/v1`. |
| `bundle_id` | `^[a-z0-9][a-z0-9._-]{0,63}$`; also the output path segment. |
| `created_at` | RFC 3339 UTC `Z` timestamp with exactly three fractional digits. |
| `tenant_id` | UUIDv4. |
| `run_id` | `identity_string` from the schema source. |
| `decision_id` | UUIDv7. |
| `decision_path` | `allow` or `deny`; declaration only, verified later from signed evidence. |
| `evidence_bundle_hash` | `sha256:` plus the bundle-root SHA-256. |
| `files` | Array of closed `file_entry` objects. |

Each `file_entry` contains exactly `path`, `media_type`, and `sha256`. `path` is
one of `decision.json`, `outcome.json`, `policy-bundle.json`,
`tool-catalog.json`, or `build-provenance.json`. `sha256` is 64 lowercase hex
over raw file bytes. `media_type` must match the path-specific enum in the schema
source. Duplicate `path` values, missing required base files, extra listed paths,
unlisted physical files, and listed files that are absent on disk exit 2.
The phase-3 file-set algorithm computes expected physical files as exactly
`{"bundle.json"} union {entry.path for entry in bundle.files}`. It then compares
that set to the actual direct child regular files in the bundle root after
rejecting directories, symlinks, hidden files, and `conversion-record.json`.
`bundle.json` is required physically but is never listed in `bundle.json.files`.

`outcome.json` may be physically present only when it is listed in
`bundle.json.files`; an unlisted `outcome.json` exits 2. For public CLI bundle
roots, the selected root profile owns the file-set rule: `examples/allow`
requires a listed physical `outcome.json`, and `examples/deny` forbids physical
or listed `outcome.json`. The lifecycle truth table owns declared allow/deny
compatibility only after a bundle reaches signed lifecycle validation. Internal
make mutation roots may deliberately reach those lifecycle rows; public fixture
roots do not bypass the root-profile file-set rule.

After the external AgenTrust package prerequisite phase, after
`bundle.json.files` has been accepted, every listed non-`bundle.json` file is
hashed over raw bytes before privacy scanning or full fixture validation parses
that file. A listed hash mismatch exits 4 and
suppresses later JSON syntax, duplicate-key, canonical-byte, closed-schema, and
media-type/profile diagnostics for that file. If the listed hash matches, those
checks exit 2 on failure.

## Schema Source

`docs/specs/agentrust-schema-source-v1alpha1.json` is the normative compact
source for every SpendGuard-owned local closed object. The committed artifact is
the generator input and machine-readable form of this section. It must match this
prose; any drift is a spec/release bug, not an override rule. Implementation must verify the artifact SHA-256 from
Package Pins And Artifacts before schema generation. Missing artifact, SHA drift,
JSON parse failure, unknown source token, or generation failure exits 6 with
`schema_source_mismatch`.

The artifact's `closed_objects` inventory is fixed:

```text
agent_manifest_report, audit_signature, build_provenance, bundle,
decision_event_file, decision_payload, digest_report, error_object,
failure_report, file_entry, filesystem_report, fixture_audit_key,
fixture_trust_store, make_verification_report, model_identity_payload,
negative_test_result, outcome_event_file, outcome_payload, outputs_report,
package_versions_report, payload_build_bindings, payload_spendguard_bindings,
policy_bundle, replay_cache_report, spendguard_artifacts,
spendguard_artifacts_row, spendguard_cloudevent, spendguard_verification_report,
tool_catalog, trace_exporter_report, verification_report
```

Schema generation is deterministic:

- Dialect is JSON Schema 2020-12.
- Token definitions and object field mappings are read from the artifact's
  `token_definitions` and `closed_objects` objects. Array/map helper tokens such
  as `array:file_entry`, `array:fixture_audit_key`,
  `array:error_object<=128`, and `object:mutated_signature,wrong_key,mutated_policy_hash,missing_trace_signature`
  are valid only if present in or derivable by the generator from the artifact;
  unknown source tokens fail generation.
- For each `closed_objects` key, generate `<object>.schema.json` with
  `$schema = "https://json-schema.org/draft/2020-12/schema"` and `$id =
  "https://agenticspendguard.dev/schemas/agentrust/v1alpha1/<object>.schema.json"`.
- Generated object schemas have `type: "object"`, exact `required` keys,
  `additionalProperties: false`, and per-field constraints expanded from source
  tokens.
- Source tokens are implemented by local validators for UUIDv4, UUIDv7, RFC 3339
  UTC timestamps, SHA-256 digests, base64url without padding, standard base64
  with padding, POSIX relative paths, enum/const/null unions, bounded strings,
  and bounded arrays.
- CI checks that regeneration from the committed source is byte-for-byte stable
  and that fixtures and success outputs validate against generated schemas.

Procedural validators in this spec own behavior that is not expressible in the
compact source: CLI parsing, path safety, file-set validation, canonical byte
checks, fixture privacy scanning, SpendGuard CloudEvent signature verification,
lifecycle derivation, package probe drift, output atomicity, and negative-test
mutations. If a generated local schema and prose disagree for a named local field,
the implementation fails conformance until this spec and the schema artifact are
updated together. The review bundle contains the generated token/object summary
for human audit.

External TRACE and Agent Manifest package schemas are not generated from this
source; they are validated by pinned AgenTrust packages.

## Fixture Trust Store

The trust store is fixture-only compatibility evidence, not a production trust
anchor. It contains only test SpendGuard audit Ed25519 public keys with
`environment = "fixture"`.

`trust-store.json` is a SpendGuard-owned local JSON file outside the bundle. It
must be UTF-8 RFC 8785 canonical JSON with no trailing newline, reject duplicate
keys, validate against generated `fixture_trust_store` schema, reject unknown
fields, and pass fixture privacy scanning. Trust-store JSON syntax,
duplicate-key, privacy, schema, canonicalization, unknown-field, key-field, and
duplicate-`key_id` failures are phase 7 failures and exit 2 after package
prerequisites have passed.

Each key has `key_id`, `algorithm = "Ed25519"`, `public_key_base64url`,
`allowed_tenant_ids`, `allowed_producer_ids`, `allowed_subject_prefixes`,
`environment = "fixture"`, `not_before`, `not_after`, and `revoked_at = null`.
Duplicate `key_id`, empty authorization arrays, non-Ed25519 algorithms,
non-fixture environments, revoked keys, or public keys that do not decode to 32
bytes fail exit 2.

Key selection is by signed event `signature.key_id`; exactly one key must match.
The selected key authorizes an event only when:

- the signed envelope tenant is in `allowed_tenant_ids`;
- the signed producer is in `allowed_producer_ids`;
- the derived subject starts with an `allowed_subject_prefixes` entry;
- that event's own signed time is within `[not_before, not_after]`.

No wall-clock time is used for fixture key expiry. Decision and outcome events
are each checked against their own signed event time. Failure to select exactly
one key, tenant/producer/subject-prefix authorization failure, or Ed25519
signature mismatch exits 3. Event time outside `[not_before, not_after]` after a
valid key is selected is a temporal evidence mismatch and exits 4 with
`trust_key_time_mismatch`. These are verification failures, not malformed
trust-store schema errors.

Trust diagnostics are exact:

| Condition | Exit | Diagnostic code |
|---|---:|---|
| trust-store JSON syntax, duplicate keys, or noncanonical bytes | 2 | `canonical_json_invalid` |
| trust-store schema violation, unknown field, duplicate `key_id`, unsupported algorithm, non-fixture environment, non-null `revoked_at`, malformed timestamps, or public key decode failure | 2 | `trust_store_invalid` |
| no key matching `signature.key_id` | 3 | `trust_key_not_found` |
| signed tenant, producer, or subject prefix is not authorized by the selected key | 3 | `trust_authorization_failed` |
| signed event time is outside the selected key window | 4 | `trust_key_time_mismatch` |
| detached Ed25519 verification fails after a valid authorized key is selected | 3 | `spendguard_signature_invalid` |

## Signed SpendGuard Events

`decision.json` and optional `outcome.json` are fixture wrappers, not the signed
bytes themselves. When `outcome.json` is absent, no outcome signature check is
performed. When `outcome.json` is present, it must pass the same wrapper, trust,
signature, and `data_b64` checks as `decision.json` before lifecycle derivation.

The wrapper exposes a SpendGuard CloudEvent object, `canonical_bytes_sha256`, and
`signature` metadata. The CloudEvent object contains `data_b64` plus a
`data_json` convenience object at `$.cloudevent.data_json`. `signature` contains
`key_id`, `algorithm = "Ed25519"`, and `value_base64url` using RFC 4648 base64url
without padding. The signature bytes are the CloudEvent `producer_signature`.
`canonical_bytes_sha256` is the SHA-256 of the reconstructed verification bytes
defined below; it is never a hash of the textual `decision.json`/`outcome.json`
wrapper, never a hash of `data_b64`, and never supplied as an independent trust
input.

CloudEvent `data_b64` uses RFC 4648 standard base64 with padding. It is never
base64url. Decoded `data_b64` bytes must be UTF-8 JSON without duplicate keys and
must exactly equal RFC 8785 canonical JSON serialization of
`$.cloudevent.data_json` with no trailing newline. This is a byte comparison, not
semantic JSON
equivalence. A mismatch exits 2 with `data_b64_mismatch`.

Signed `data_json` is closed, not an open extension object. The schema source
uses `decision_payload_by_event_type`: when `$.cloudevent.type` is
`spendguard.audit.decision`, `data_json` must validate as `decision_payload`; when
the type is `spendguard.audit.outcome`, `data_json` must validate as
`outcome_payload`. A decision payload has exactly `reason_codes`, `matched_rules`,
`snapshot_hash`, `spendguard`, `model`, and `data_class`. Its nested `spendguard`
object has exactly `policy_bundle_hash`, `tool_catalog_hash`, and `build`; nested
`build` has exactly `container_image_digest`, `release_bundle_digest`,
`sbom_digest`, and `source_commit_digest`, each a `sha256:` digest or `null`.
Nested `model` has exactly `provider`, `model_id`, and `version`. An outcome
payload has exactly `kind`, `decision_id`, `reservation_id`, and
`estimated_amount_atomic`. Unknown signed payload fields exit 2 before lifecycle
or artifact-binding checks.

SpendGuard audit signature verification is normative in this section.
Conformance is determined by the canonicalization rules below, the committed
`proto/spendguard/common/v1/common.proto` artifact and SHA-256 in Package Pins
And Artifacts, and
`docs/specs/agentrust-cloudevent-signature-vector-2026-07-01.json`.

- If `producer_id` does not start with `ledger:`, decode wrapper
  `signature.value_base64url` as the detached signature bytes. Decode the fixture
  CloudEvent fields to `spendguard.common.v1.CloudEvent` from the committed
  proto artifact, populate all signed fields, set `producer_signature` to empty bytes for
  canonicalization, encode with protobuf deterministic field semantics equivalent
  to `prost::Message::encode_to_vec`, and verify the detached signature bytes
  against those encoded bytes. Wrapper `tenant_id`, `run_id`, `decision_id`,
  `producer_id`, and `schema_bundle_id` map to same-named message fields; wrapper
  `signature.key_id` maps to proto/ledger `signing_key_id`. Only the ledger JSON
  canonical path below uses the compact names `tenantid`, `runid`, and
  `decisionid`.
- If `producer_id` starts with `ledger:`, reconstruct the ledger JSON canonical
  byte string from the normalized fixture fields. The reconstructed object
  contains exactly `specversion`, `type`, `source`, `id`,
  `time_seconds`, `time_nanos`, `datacontenttype`, `data_b64`, `tenantid`,
  `runid`, `decisionid`, `schema_bundle_id`, `producer_id`,
  `producer_sequence`, and `signing_key_id`, in the deterministic order used by
  the CloudEvent signature vector, and verifies wrapper
  `signature.value_base64url` as the detached signature bytes. The ledger JSON
  byte string is the exact compact JSON object in that key order; it is not RFC
  8785 sorted-key JSON and does not rely on serializer insertion-order defaults.

Ledger JSON canonicalization is UTF-8 with no trailing newline, no insignificant
whitespace, object keys exactly in the order listed above, JSON strings escaped
per RFC 8259, and integer fields serialized as base-10 JSON numbers with no
leading zeroes, signs, decimals, or exponent notation. The only numeric fields
are `time_seconds`, `time_nanos`, and `producer_sequence`. String values use the
already-normalized fixture strings; `data_b64` is standard padded base64.
For v1alpha1 ledger fixtures, every string value in the reconstructed ledger JSON
object (`specversion`, `type`, `source`, `id`, `datacontenttype`, `data_b64`,
`tenantid`, `runid`, `decisionid`, `schema_bundle_id`, `producer_id`,
`signing_key_id`) must be printable ASCII and must not contain `"`, `\`, or
control characters. A value that would require JSON escaping exits 2 with
`canonical_json_invalid` during phase 8 canonical reconstruction.

The non-ledger protobuf branch uses this exact field mapping from the committed
`CloudEvent` descriptor:

| Fixture path | Proto field | Field number | Type |
|---|---|---:|---|
| `$.cloudevent.specversion` | `specversion` | 1 | `string` |
| `$.cloudevent.type` | `type` | 2 | `string` |
| `$.cloudevent.source` | `source` | 3 | `string` |
| `$.cloudevent.id` | `id` | 4 | `string` |
| `$.cloudevent.time` | `time` | 5 | `google.protobuf.Timestamp` |
| `$.cloudevent.datacontenttype` | `datacontenttype` | 6 | `string` |
| decoded `$.cloudevent.data_b64` bytes | `data` | 7 | `bytes` |
| `$.cloudevent.tenant_id` | `tenant_id` | 100 | `string` |
| `$.cloudevent.run_id` | `run_id` | 101 | `string` |
| `$.cloudevent.decision_id` | `decision_id` | 102 | `string` |
| `$.cloudevent.schema_bundle_id` | `schema_bundle_id` | 103 | `string` |
| `$.cloudevent.producer_id` | `producer_id` | 200 | `string` |
| `$.cloudevent.producer_sequence` | `producer_sequence` | 201 | `uint64` |
| empty bytes during verification | `producer_signature` | 202 | `bytes` |
| `$.signature.key_id` | `signing_key_id` | 203 | `string` |

The public Python CLI may satisfy the non-ledger protobuf branch either by
calling a checked Rust helper or by using a generated Python binding. In both
cases, the binding/helper must be generated from or demonstrably equivalent to
the committed proto artifact, and conformance is judged by the descriptor mapping
above plus the CloudEvent signature vector's canonical byte strings, field set,
and signatures.
The verifier's phase-4 prerequisite checks must execute that helper or binding
against the committed CloudEvent signature vector before verifying fixtures. If
it cannot reproduce the vector's non-ledger protobuf canonical bytes exactly from
the pinned `common.proto` artifact, the run exits 6 with `package_probe_drift`.
This vector check is required in the verifier path, not only in external CI.

All fields used for canonicalization are required by `spendguard_cloudevent`;
missing or malformed wrapper fields exit 2 during phase-7 local schema validation
before canonical bytes are computed. Fixture
`$.cloudevent.time` is RFC 3339 UTC milliseconds and converts to protobuf
`Timestamp { seconds, nanos }` with `nanos` equal to milliseconds times
1,000,000. `producer_sequence` must be a non-negative integer that fits in signed
64-bit range (`0..9223372036854775807`) even though the proto field type is
`uint64`; the JSON schema token bound is normative. Values above that bound exit
2 with `schema_invalid`. `data_b64` decodes to the proto `data` bytes. Fields not
listed in `spendguard_cloudevent` are rejected by schema; proto fields not
represented by the fixture use protobuf default values. Proto3 default-valued
scalar and bytes fields are omitted from the encoded canonical bytes.
`producer_signature` is set to the empty byte string for verification and is
therefore omitted from the proto3 encoding. No unknown-field preservation is part
of v1alpha1 canonicalization.

For both profiles, `canonical_bytes` means exactly the byte string passed to
Ed25519 verification: protobuf `encode_to_vec` bytes for non-ledger producers, or
the deterministic reconstructed ledger JSON byte string for ledger producers.
`sha256(canonical_bytes)` must equal wrapper `canonical_bytes_sha256`. Malformed
profile inputs or inability to reconstruct canonical bytes exit 2; a successfully
reconstructed byte string whose hash differs from `canonical_bytes_sha256` exits
4 with `cloudevent_canonical_hash_mismatch`; Ed25519 verification failure exits
3 with `spendguard_signature_invalid`.

The CloudEvent vector artifact contains both canonical branches: non-`ledger:`
protobuf bytes with `producer_signature` empty, and `ledger:` JSON canonical
bytes with the exact key order listed above. The vector
includes canonical bytes, SHA-256, Ed25519 public key, detached signature, and
negative controls proving that a proto-signed event does not verify under the
ledger JSON canonical form and vice versa.

The derived TRACE subject is computed only after SpendGuard signature and
trust-store authorization succeed:

```text
spiffe://spendguard.local/tenant/<event.tenant_id>/run/<event.run_id>
```

For this AgenTrust subject derivation, signed `run_id` must not contain `/` even
though the shared `identity_string` token permits it for other SpendGuard uses.
`identity_string` already excludes `..`, `%`, `?`, `#`, spaces, and controls;
with the slash ban, remaining `:`, `@`, `.`, `_`, and `-` bytes are URI
path-segment safe.
Slash-containing `run_id` exits 2 with `schema_invalid` before subject-prefix
authorization.

The verifier must not use bundle metadata, CLI arguments, HTTP headers, workload
identity, unsigned debug files, or conversion records as subject or trust input.

## Signed Field Paths

All lifecycle and binding values are read from the signed CloudEvent envelope and
from decoded `data_b64` JSON. The fixture wrapper may expose convenience
`data_json`, but `data_json` is trusted only after the byte comparison above.

Envelope paths use the wrapper `cloudevent` object:

| Meaning | Canonical fixture path |
|---|---|
| tenant | `$.cloudevent.tenant_id` |
| run | `$.cloudevent.run_id` |
| decision UUID | `$.cloudevent.decision_id` |
| producer | `$.cloudevent.producer_id` |
| event type | `$.cloudevent.type` |
| event time | `$.cloudevent.time`, parsed from RFC 3339 UTC milliseconds to proto `(seconds,nanos)` when needed |
| payload | `$.cloudevent.data_b64` |
| signing key | `$.signature.key_id` |

For the ledger JSON canonical path only, those same normalized envelope values
are serialized as `tenantid`, `runid`, and `decisionid` because that is the
ledger JSON vector contract.

Decoded `data_b64` paths:

| Meaning | Decoded JSON path |
|---|---|
| deny/allow reason list | `$.reason_codes` |
| deny matched rules | `$.matched_rules` |
| decision snapshot hash | `$.snapshot_hash` |
| outcome kind | `$.kind` |
| outcome decision UUID | `$.decision_id` |
| reservation UUID | `$.reservation_id` |
| estimated amount | `$.estimated_amount_atomic` |
| policy binding hash | `$.spendguard.policy_bundle_hash` |
| tool catalog hash | `$.spendguard.tool_catalog_hash` |
| model provider | `$.model.provider` |
| model ID | `$.model.model_id` |
| model version | `$.model.version` |
| data class | `$.data_class` |
| container image digest | `$.spendguard.build.container_image_digest` |
| release bundle digest | `$.spendguard.build.release_bundle_digest` |
| SBOM digest | `$.spendguard.build.sbom_digest` |
| source commit digest | `$.spendguard.build.source_commit_digest` |

No other field path may influence lifecycle, subject derivation, policy binding,
tool binding, model identity, data class, or build digest selection in v1alpha1.

## Lifecycle

Lifecycle derivation reads only signed decoded `data_b64` JSON after signature
and trust-store authorization succeed. Diagnostics may reference `data_json`
pointers only after `data_b64` byte comparison passes.

Decision lifecycle inputs are decoded paths `$.reason_codes`,
`$.matched_rules`, and `$.snapshot_hash`. Outcome lifecycle inputs are decoded
paths `$.kind`, `$.decision_id`, `$.reservation_id`, and
`$.estimated_amount_atomic`.

After decision signature and trust verification succeed, unsigned bundle metadata
must match the signed decision envelope: `bundle.tenant_id`,
`bundle.run_id`, and `bundle.decision_id` equal the signed
`tenant_id`, `run_id`, and `decision_id`; `bundle.created_at` equals the signed
decision CloudEvent time after both are parsed and normalized to RFC 3339 UTC
milliseconds. Mismatch exits 4 with `lifecycle_mismatch`. Generated TRACE and
Agent Manifest timestamps are derived only after this equality check passes.

Fixture truth table:

| `bundle.decision_path` | Physical/listed `outcome.json` | Decision `reason_codes` | Outcome `kind` | Result |
|---|---|---|---|---|
| `deny` | absent and unlisted | non-empty string array | n/a | derive deny; require `matched_rules` and `snapshot_hash`. |
| `deny` | present and listed | any well-formed value | any well-formed value | public CLI and required make mutations exit 2 during file-set validation before signed lifecycle; a verifier-core unit test that reaches signed lifecycle exits 4. |
| `deny` | present but unlisted | any | any | exit 2 during file-set validation. |
| `allow` | present and listed | empty array | `reservation_created` | derive allow. |
| `allow` | absent or unlisted | empty array | n/a | public CLI and required make mutations exit 2 during file-set validation before signed lifecycle; a verifier-core unit test that reaches signed lifecycle exits 2 after signature/trust because allow requires a valid outcome. |
| `allow` | present and listed | non-empty string array | any well-formed value | exit 4. |
| any | any | missing, null, non-array, or non-string entry | any | exit 2 during phase-7 payload schema validation before signed lifecycle. |
| any | present and listed | empty array | missing, null, non-string, or empty | exit 2 during phase-7 payload schema validation before signed lifecycle. |
| any | present and listed | empty array | string other than `reservation_created` | exit 4. |

Derived signed-state summary; the fixture truth table above is normative and
wins if wording drifts:

| Signed state | Result |
|---|---|
| `reason_codes` missing, null, non-array, or contains a non-string entry | exit 2 during phase-7 payload schema validation |
| decision `reason_codes` non-empty and no `outcome.json` | derive deny; require non-empty string-array `matched_rules` and `snapshot_hash` |
| decision `reason_codes` non-empty and outcome event reaches signed lifecycle | exit 4; public/root-profile deny with listed or physical `outcome.json` exits 2 during phase-3 file-set validation before this row is reachable |
| decision `reason_codes` empty and no `outcome.json` | exit 2 |
| decision `reason_codes` empty and outcome `kind = "reservation_created"` | derive allow |
| outcome `kind` missing, null, non-string, or empty | exit 2 during phase-7 payload schema validation |
| outcome `kind` is a string other than `reservation_created` | exit 4 |
| derived path differs from `bundle.json.decision_path` | exit 4 |

The distinction is: malformed fixture structure exits 2; signed evidence that is
well-formed but contradicts other verified evidence exits 4.
The schema source allows `decision_payload.matched_rules` to be an empty array so
allow decisions can use the same closed payload shape. Deny lifecycle adds a
procedural requirement: when `reason_codes` is non-empty,
`matched_rules` must contain at least one non-empty string and `snapshot_hash`
must be a `sha256:` digest. If phase-7 payload schema has passed but this deny
procedural requirement fails, the verifier exits 4 with `lifecycle_mismatch` and
uses JSON pointer `/cloudevent/data_json/matched_rules` for an empty
`matched_rules` array.

For allow, outcome type is `spendguard.audit.outcome`; tenant, run, decision, and
derived subject linkage must match; outcome time is greater than or equal to
decision time; decoded `$.decision_id` equals the signed decision UUIDv7;
`reservation_id` is UUIDv4; `estimated_amount_atomic` matches
`^[1-9][0-9]{0,38}$`. Signs, decimal points, exponent notation, leading zeroes,
`NaN`, and infinity are invalid. Outcome wrapper/payload lifecycle mismatches
exit 4 with `lifecycle_mismatch` and point to `/cloudevent/tenant_id`,
`/cloudevent/run_id`, `/cloudevent/decision_id`, `/signature/key_id`,
`/cloudevent/data_json/decision_id`, or `/cloudevent/data_json/kind`.

`bundle.json.created_at` is RFC 3339 UTC with exactly three fractional second
digits. CloudEvent times are compared as `(seconds,nanos)` values. Generated
Agent Manifest `issued_at`, `expires_at`, and artifact `bound_at` values are
whole-second RFC 3339 UTC with `Z`; millisecond input is truncated toward the
past by flooring fractional seconds.

Malformed, missing, offset-based, or non-UTC timestamp fields in `bundle.json`,
trust-store keys, CloudEvents, or generated reports exit 2. Temporal
contradictions after valid parsing, such as outcome time before decision time,
exit 4. Event time outside the selected fixture key validity window exits 4 with
`trust_key_time_mismatch`; it is a temporal evidence mismatch after key
selection, not a trust-store schema or authorization failure.

## Policy, Tool, And Build Bindings

`policy-bundle.json`, `tool-catalog.json`, and `build-provenance.json` are
closed fixture files. Their wrapper file hashes prove fixture file integrity;
their internal digest fields bind source artifacts.

Policy binding is evidentiary only when `policy_bundle_hash` equals signed
decoded `$.spendguard.policy_bundle_hash`; mismatch exits 4. Tool binding is
evidentiary only when `catalog_hash` equals signed decoded
`$.spendguard.tool_catalog_hash`; mismatch exits 4.

Build provenance digest selection uses this fixed priority:
`container_image_digest`, else `release_bundle_digest`, else `sbom_digest`, else
`source_commit_digest`. Every present signed build digest must match the wrapper
same-named digest or exit 4. At least one signed build digest must be present in
decoded `$.spendguard.build`. Wrapper-only build provenance is invalid in
v1alpha1 and exits 4.
Any non-null wrapper build digest whose same-named signed decoded digest is
absent is wrapper-only and exits 4. The selected build digest is the first
signed-and-wrapper-matched digest in the priority order above.
Wrapper fields whose value is `null` are ignored for priority only when the
same-named signed decoded digest is also absent or `null`. A signed non-null
digest paired with a wrapper `null` is a build binding mismatch. A lower-priority
signed-and-wrapper-matched digest may be selected when every higher-priority
wrapper field is `null` and every higher-priority signed decoded digest is absent
or `null`.

## TRACE Mapping

TRACE output is signed with the committed test-only TRACE exporter key and
validated with `agentrust-trace==0.2.0`.

Required package APIs are `TrustRecord`, `load_signing_key`, `sign_record`, and
`validate_json` from `agentrust_trace==0.2.0`. Missing packages, import failures,
or pinned-version mismatch exit 6 before
outputs are generated. Package drift exits 6 if the committed package probe's
accepted TRACE shape no longer passes `TrustRecord.model_validate(...)` and
`validate_json(...)` under the installed package set. A generated SpendGuard TRACE
object that fails those validations exits 5.

The pre-sign TRACE dict contains exactly `eat_profile`, `iat`, `subject`,
`model`, `runtime`, `policy`, `data_class`, `tool_transcript`,
`build_provenance`, `appraisal`, and `transparency`; it must not contain `cnf` or
`signature`. `sign_record(pre_sign, load_signing_key(path))` is the only TRACE
signing API. The returned dict must add `cnf.jwk` and `signature`.

The generated TRACE `build_provenance.slsa_level` is integer `1` because the
accepted `agentrust-trace==0.2.0` schema rejects `0`; this is not a field in the
local `build-provenance.json` schema. Reports and docs must not describe that
generated TRACE field as SLSA level 0. TRACE Level 0 is represented by the
software-only runtime/appraisal semantics, not by
`build_provenance.slsa_level = 0`. Allowed wording:
`generated TRACE build_provenance.slsa_level=1 is package-required metadata, not
a SLSA Level 1 claim`. Forbidden wording includes `SLSA Level 1`,
`SLSA-compliant`, `SLSA attested`, or any text that treats the package-required
integer as provenance assurance.

`trace-level0.json` is exempt from SpendGuard RFC 8785 output canonicalization
because it is an AgenTrust package object. The writer uses
`json.dumps(record, sort_keys=True, separators=(",", ":"), ensure_ascii=True)`
with no trailing newline, then validates the in-memory dict with the package
APIs above. No semantic post-processing is allowed after `sign_record(...)`.

TRACE fields:

| TRACE field | Exact v1alpha1 value |
|---|---|
| `eat_profile` | `tag:agentrust.io,2026:trace-v0.1`. |
| `iat` | Unix seconds parsed from `bundle.json.created_at`. |
| `subject` | Derived signed-envelope subject. |
| `model.provider` | Decision decoded path `$.model.provider`. |
| `model.model_id` | Decision decoded path `$.model.model_id`. |
| `model.version` | Decision decoded path `$.model.version`. |
| `runtime.platform` | `software-only`. |
| `runtime.measurement` | Full `bundle.json.evidence_bundle_hash`. |
| `policy.bundle_hash` | `policy-bundle.json.policy_bundle_hash`; mismatch with signed decoded `$.spendguard.policy_bundle_hash` exits 4. |
| `policy.enforcement_mode` | `policy-bundle.json.enforcement_mode`. |
| `policy.version` | `spendguard-agentrust-v1`. |
| `data_class` | Decision decoded path `$.data_class`. |
| `tool_transcript.hash` | `tool-catalog.json.catalog_hash`; mismatch with signed decoded `$.spendguard.tool_catalog_hash` exits 4. |
| `tool_transcript.call_count` | `0` for deny, `1` for allow. |
| `tool_transcript.transcript_uri` | `urn:spendguard:evidence:<bundle_id>`. |
| `build_provenance.builder` | `build-provenance.json.builder`. |
| `build_provenance.digest` | Fixed decoded-path priority: `$.spendguard.build.container_image_digest`, else `$.spendguard.build.release_bundle_digest`, else `$.spendguard.build.sbom_digest`, else `$.spendguard.build.source_commit_digest`; each value must match the same-named field in `build-provenance.json`. |
| `build_provenance.provenance_uri` | `urn:spendguard:build-provenance:<bundle_id>`. |
| `build_provenance.slsa_level` | Generated TRACE field fixed to `1`; not present in local `build-provenance.json`. |
| `appraisal.status` | `none`. |
| `appraisal.policy_ref` | `spendguard-agentrust-v1`. |
| `appraisal.verifier` | `agentic-spendguard-agentrust-exporter`. |
| `appraisal.timestamp` | Same value as `iat`. |
| `transparency` | `pending`; no transparency log anchoring is claimed. |
| `cnf`, `signature` | Generated by `agentrust-trace`. |

The committed TRACE exporter private key is test-only compatibility material. It
may exist only under
`integrations/agentrust/test-private-keys/*.test.ed25519.pem`, must not contain
comments or labels inside the PEM bytes, and must be excluded from published
packages, declared container context tars, and production install artifacts.
v1alpha1 does not claim full container-image scanning. Generated TRACE
records must use fixture issuer/key metadata and documentation warnings so they
cannot be mistaken for production trust evidence.
If a committed test key exists, the exact source file must be owned by the
current effective UID. `make verify-agentrust` copies it to `<trace-key-copy>`
with mode `0600` and passes the copy to internal verifier invocations; copy,
chmod, other owner, or remaining group/world bits on the copy exit 6 with
`trace_key_invalid`. After the final verifier invocation, make must unlink
`<trace-key-copy>`, remove the empty `.trace-key` directory, and fail final
packaging guard with `packaging_guard_failed` if `.trace-key` remains.
Secret-scanner allowlists for this integration may name only the exact source path
plus the exact SHA-256 of the committed fixture key bytes. Wildcard PEM allowlists,
directory-wide allowlists, or allowlists that match private-key bytes without a
path-and-hash pair are non-conformant and the packaging/container guard must fail
closed. Reports, diagnostics, help text, and aggregate make output must not copy
the concrete private-key path or private-key bytes; they may use only the
allowlist glob and the committed-test-key warning text below.

Ed25519 key encoding is fixed. Trust-store public keys are raw 32-byte Ed25519
public keys encoded as base64url without padding; any other decoded length exits
2 with `trust_store_invalid`. The TRACE exporter private key is an unencrypted
PKCS#8 PEM Ed25519 private key with exactly one `-----BEGIN PRIVATE KEY-----` /
`-----END PRIVATE KEY-----` block, no PEM encryption headers, no OpenSSH format,
no additional PEM blocks, no leading or trailing non-whitespace text, and no
comments or labels. Encrypted PEMs, OpenSSH private keys, SEC1-style keys,
non-Ed25519 OIDs, extra blocks, or parse failures exit 6 with
`trace_key_invalid` before output generation. Implementations may use
`cryptography` or another library only if it enforces this same accepted byte
format and Ed25519 key type.

The verification report must include `trace_exporter.signature_status =
"trace_package_signed"` and `trace_exporter.key_scope = "committed_test_key"`.
`trace_exporter.key_path` records the literal allowlist glob
`integrations/agentrust/test-private-keys/*.test.ed25519.pem`, not the selected
concrete private-key path. The concrete `--trace-signing-key` value is validated
by path safety and key loading but is not copied into reports.
Generated docs and reports must include the literal warning
`TRACE signed by committed test fixture key; not production trust evidence`.
`make verify-agentrust` fails if packaging/container guard output cannot prove
the test key is excluded from distributable artifacts.

The packaging/container guard is offline and mechanical:

| Surface | Contract |
|---|---|
| Source key allowlist | Only manifest `trace_test_key` and `wrong_key_negative` paths under `integrations/agentrust/test-private-keys/*.test.ed25519.pem`; exact path plus SHA-256 allowlists only. |
| Python packages | Build with `python -m build --no-isolation`; scan current-run `dist/*.tar.gz` and `dist/*.whl` member names/bytes. Any package member under `integrations/agentrust/out/` fails. |
| Repository/output scan | Scan `git ls-files -z --cached --others --exclude-standard` plus public `integrations/agentrust/out/*/**`, excluding `.verify-agentrust/**` make scratch; repository symlink targets are not followed. |
| Container contexts | If `integrations/agentrust/container-contexts.txt` exists, each non-comment line is a repo-relative context. Leading/trailing whitespace, absolute paths, `..`, hidden segments, duplicates, outside-repo paths, unreadable files, or symlinks inside contexts fail closed. Absence means no container packaging claim. |
| `.dockerignore` subset | LF/CRLF lines, blank/comment lines, `!` negation, slash-rooting, `*` within one segment, bytewise case-sensitive last-match-wins over regular files. Unsupported: directory-only `/`, `**`, character classes, escaped comments, significant trailing spaces, backslashes, Dockerfile-specific ignore files, BuildKit/named/remote/bind/generated contexts. |
| Resource limits | Any scanned file/member over 64 MiB, archive over 10000 entries, nested archive, unsafe member path, duplicate normalized member name, unstreamable compressed member, or unsafe extraction fails closed. |
| Leak checks | Non-allowlisted concrete `*.test.ed25519.pem` names, exact allowlisted source PEM bytes outside allowlisted paths, or a complete private-key PEM block with BEGIN line, base64 body, and matching END line fail. Standalone sentinel strings in normative specs/tests are not secret bytes. |
| Internal exposure | Generated wheel/sdist entry points, scripts, metadata, docs, public help, and declared container contexts must not expose `_verify_internal`, `--mutation`, `SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT`, or an internal command name. Module member names alone are not exposure. |

Scan order is deterministic: context syntax, build, internal-entry-point scan,
repository scan, generated package scan, declared context tar scan, then output
scan. Within a scan item, internal entry points beat key path names, and key path
names beat private-key bytes. All failures are exit 6
`packaging_guard_failed` in make setup context and are never warnings.

## Agent Manifest Mapping

Agent Manifest output is unsigned and validated with `agent-manifest==0.1.1`.

Required package API is `Manifest` from `agent_manifest==0.1.1`. Missing
packages, import failures, or pinned-version mismatch exit 6 before
outputs are generated. Package drift exits 6 if the committed package probe's
accepted unsigned Manifest shape no longer passes
`Manifest.model_validate(...)`. A generated SpendGuard Manifest that fails
`Manifest.model_validate(manifest_dict)` exits 5.

The artifact object shapes below are not free-form SpendGuard extensions; they
are the exact unsigned Manifest shape proven by
`docs/specs/agentrust-package-probe-2026-07-01.json` under
`agent-manifest==0.1.1`. Fields such as `agt_version`,
`audit_chain_root`, and `audit_key_sealed` are allowed only because the probe's
accepted shape validates. If a future package rejects that probe, the verifier
exits 6 at the package prerequisite phase, after bundle index, file-set, and
privacy intake checks, but before full fixture schema, canonicalization, hash,
trust, lifecycle, package-output validation, or output generation. If generated
output diverges from the probed shape and is rejected, the run exits 5. The phrase
"evidence-bundle root" is a documentation
and report label; it is not added as an extra Agent Manifest field unless present
in the probed package-accepted shape.

The unsigned Manifest dict contains exactly these top-level keys: `@context`,
`@type`, `version`, `manifest_id`, `agent_id`, `issuer`, `issued_at`,
`expires_at`, `crypto_profile`, and `artifacts`. v1alpha1 must not emit
`signature`, `signed_at`, `verification`, trust-context, or transparency-log
fields. The verification report sets `signature_status =
"unsigned_not_verified"`.

`@context` is `https://agentmanifest.agentrust.io/v0.1/context.json`; `@type` is
`AgentManifest`; `version` is `0.1`; `crypto_profile` is `standard`.

`artifacts` is an object with exactly `system_prompt`, `policy_bundle`,
`model_identity`, `tool_manifest`, and `decision_trace`, plus `supply_chain` only
when `container_image_digest` is present. Artifact objects do not contain
per-artifact `@type`, `name`, `id`, `signature`, or `verification` fields in
v1alpha1. Every field listed below is required when its artifact object is
present. Optional artifact objects are omitted entirely; v1alpha1 does not use
`null` fields in Agent Manifest output. The writer validates the in-memory
`manifest_dict` with `Manifest.model_validate(manifest_dict)`, discards any
package-normalized model object returned by validation, and serializes the
original validated dict with
`json.dumps(manifest_dict, sort_keys=True, separators=(",", ":"), ensure_ascii=True)`
with no trailing newline.

Artifact objects are closed:

- `system_prompt`: `bound_at`, `hash`, `hash_algorithm`, `classification`, `version`
- `policy_bundle`: `agt_version`, `bound_at`, `enforcement_mode`, `hash`, `policy_language`, `scope`, `version`
- `model_identity`: `bound_at`, `deployment_type`, `model_attestation_type`, `model_id`, `provider`, `version`
- `tool_manifest`: `allow_dynamic_registration`, `bound_at`, `catalog_hash`, `rug_pull_policy`, `tools`
- `decision_trace`: `audit_chain_root`, `audit_chain_uri`, `audit_key_sealed`, `bound_at`, `entry_count`, `first_entry_at`, `last_entry_at`, `signing_key_id`, `trace_type`
- `supply_chain`: `bound_at`, `container_image_digest`

Any missing or extra top-level key, artifact key, or nested artifact field is an
Agent Manifest layout error and exits 5 before writing outputs, even if the
package would otherwise accept the object.

`issuer` appears exactly once, as the top-level Manifest `issuer`. Its only
source is the CLI default or `--manifest-issuer`. Bundle files, trust-store files,
TRACE records, package-probe examples, and artifact bindings must not provide or
override Manifest issuer. Any `issuer` key inside `artifacts` or nested artifact
objects is an Agent Manifest layout error and exits 5.

Generated Agent Manifest identity:

- `agent_id` equals the derived TRACE subject.
- `issued_at` equals `bundle.json.created_at` truncated to whole seconds.
- `expires_at` equals `issued_at + 90 days`, computed as exactly 7,776,000 UTC
  seconds after `issued_at` and formatted as whole-second RFC 3339 UTC with `Z`.
- fixture `issuer` defaults to `spiffe://spendguard.local/agentrust-exporter`.
- `--manifest-issuer`, when present, must satisfy the same CLI rule: an absolute
  `spiffe://` URI with no query, no fragment, no ASCII control characters, and at
  most 256 bytes. Violation exits 2 during CLI validation. It replaces only
  `issuer`; it does not affect `manifest_id`, TRACE output, evidence-bundle root,
  or package validation semantics.

`manifest_id` is deterministic UUIDv7. Timestamp bits are `issued_at` Unix
milliseconds. Random bits are derived from:

```text
u32be(len(evidence_bundle_hash)) || evidence_bundle_hash ||
u32be(len(agent_id)) || agent_id ||
u32be(len("agent-manifest")) || "agent-manifest"
```

Take SHA-256 of that preimage and interpret the digest as a big-endian integer.
Let `rand_a` be the top 12 digest bits and `rand_b` be the next 62 digest bits.
The UUID integer is:

```text
(unix_ms << 80) | (0x7 << 76) | (rand_a << 64) | ((0b10 << 62) | rand_b)
```

`unix_ms` is `floor(bundle.created_at to whole seconds) * 1000`. It must fit in
the 48-bit UUIDv7 timestamp field (`0 <= unix_ms <= 281474976710655`); timestamps
outside that range exit 2 as timestamp/schema invalid before Manifest generation.
The RFC 3339 parser rejects leap seconds (`:60`), offsets other than `Z`, and
timezone names, so leap-second arithmetic is not part of v1alpha1. The final string
is the lowercase canonical UUID representation. This is checked by
`docs/specs/agentrust-manifest-uuidv7-vector-2026-07-01.json` and
`docs/specs/check-agentrust-vectors.py`. No other Agent Manifest artifact binding
UUIDs are generated in v1alpha1; SpendGuard extension artifact row IDs are fixed
strings.

Required artifact bindings are `system_prompt`, `policy_bundle`,
`model_identity`, `tool_manifest`, and `decision_trace`; `supply_chain` is emitted
only when `container_image_digest` exists. `decision_trace.audit_chain_root`
contains the evidence-bundle root and must be labeled as evidence-bundle root,
not ledger audit-chain proof.

Manifest artifact fields are fixed:

| Manifest artifact field | Exact v1alpha1 value |
|---|---|
| `system_prompt.hash` | `policy-bundle.json.system_prompt_hash` when present, otherwise `sha256:` plus 64 zeroes. |
| `system_prompt.hash_algorithm` | `SHA-256`. |
| `system_prompt.classification` | Same selected value as TRACE `data_class`. |
| `system_prompt.version` | `spendguard-agentrust-v1`. |
| `policy_bundle.hash` | `policy-bundle.json.policy_bundle_hash`. |
| `policy_bundle.enforcement_mode` | `policy-bundle.json.enforcement_mode`. |
| `policy_bundle.policy_language` | `composite`. |
| `policy_bundle.scope` | `["spendguard:budget"]`. |
| `policy_bundle.version` / `agt_version` | `spendguard-agentrust-v1`. |
| `model_identity.provider` / `model_id` / `version` | Same selected values as TRACE `model`. |
| `model_identity.deployment_type` | `api`. |
| `model_identity.model_attestation_type` | `provider-asserted`. |
| `tool_manifest.catalog_hash` | `tool-catalog.json.catalog_hash`. |
| `tool_manifest.tools` | Empty array for v1alpha1 fixtures. |
| `tool_manifest.allow_dynamic_registration` | `false`. |
| `tool_manifest.rug_pull_policy` | `deny-and-alert`. |
| `decision_trace.audit_chain_root` | `bundle.json.evidence_bundle_hash`; label as evidence-bundle root, not ledger audit-chain root. |
| `decision_trace.audit_chain_uri` | `urn:spendguard:evidence:<bundle_id>`. |
| `decision_trace.trace_type` | `hash-chained`. |
| `decision_trace.entry_count` | `1` for deny, `2` for allow. |
| `decision_trace.first_entry_at` | Decision CloudEvent `time`, whole-second UTC. |
| `decision_trace.last_entry_at` | Outcome CloudEvent `time` for allow, otherwise decision CloudEvent `time`, whole-second UTC. |
| `decision_trace.signing_key_id` | Selected trust-store `key_id`. |
| `decision_trace.audit_key_sealed` | `false`. |
| `supply_chain.container_image_digest` | Signed-and-wrapper-matched `container_image_digest`; omit `supply_chain` if no such container digest exists. |

Every emitted artifact object has `bound_at` equal to Manifest `issued_at`.
When the selected build digest is `release_bundle_digest`, `sbom_digest`, or
`source_commit_digest`, it is represented in TRACE and `spendguard-artifacts.json`
only; v1alpha1 does not project those digests into Agent Manifest `supply_chain`.

Manifest signing, `Ed25519Signer`, trust contexts, and `verify_manifest(...)` are
out of scope and related CLI flags exit 2.

## SpendGuard Artifacts

`spendguard-artifacts.json` is a SpendGuard extension artifact, not inserted into
Agent Manifest. It validates against the closed `spendguard_artifacts` schema
source object and is an object with exactly `schema =
"spendguard-agentrust-artifacts/v1"` and `artifacts`. `artifacts` contains
exactly four `spendguard_artifacts_row` rows in this order:

1. `spendguard.policy_bundle`
2. `spendguard.tool_catalog`
3. `spendguard.audit_evidence`
4. `spendguard.build_provenance`

Each row records `id`, `media_type`, `binding_digest`, `source_file_digest`,
`evidence_level`, `signed_source`, and `source_file`. Evidence levels are
`signed_event_field`, `verified_bundle_root`, or `operator_supplied_context`.
Operator-supplied context must not be called SpendGuard-signed evidence.

Exact row mapping:

| Row id | `media_type` | `binding_digest` | `source_file_digest` | `evidence_level` | `signed_source` | `source_file` |
|---|---|---|---|---|---|---|
| `spendguard.policy_bundle` | `application/vnd.spendguard.policy-bundle+json` | `policy-bundle.json.policy_bundle_hash` | Bundle file hash for `policy-bundle.json`. | `signed_event_field` | `$.spendguard.policy_bundle_hash` | `policy-bundle.json` |
| `spendguard.tool_catalog` | `application/vnd.spendguard.tool-catalog+json` | `tool-catalog.json.catalog_hash` | Bundle file hash for `tool-catalog.json`. | `signed_event_field` | `$.spendguard.tool_catalog_hash` | `tool-catalog.json` |
| `spendguard.audit_evidence` | `application/vnd.spendguard.agentrust.evidence-bundle+json` | `bundle.json.evidence_bundle_hash` | Evidence-bundle root. | `verified_bundle_root` | `null` | `bundle.json` |
| `spendguard.build_provenance` | `application/vnd.spendguard.build-provenance+json` | Selected signed build digest. | Bundle file hash for `build-provenance.json`. | `signed_event_field` | Matching signed build digest pointer. | `build-provenance.json` |

## Output Semantics

Successful `--out-dir/<bundle_id>/` contains exactly `trace-level0.json`,
`agent-manifest.json`, `spendguard-artifacts.json`, and
`verification-report.json`. The final target must not already exist. Phase 1
validates only the allowed `--out-dir`; immediately after phase 2 accepts
`bundle_id`, the verifier rejects a preexisting final target and cleans stale
`.tmp-<bundle_id>-<nonce>` directories for that bundle.

All four output objects are generated, package/local-schema validated, and
serialized in memory before any temp output directory is created. Any TRACE,
Manifest, SpendGuard-artifacts, verification-report, or serialization failure
creates no temp directory and no final target. After validation, the exporter
writes prevalidated bytes to `--out-dir/.tmp-<bundle_id>-<nonce>/`, reads them
back byte-for-byte, then promotes the sibling directory to
`--out-dir/<bundle_id>/` with an exclusive no-replace directory rename.

Supported promotion platforms are Linux with `renameat2(..., RENAME_NOREPLACE)`
or equivalent, and macOS with `renamex_np(..., RENAME_EXCL)` or equivalent.
Other POSIX platforms are unsupported for v1alpha1 conformance and fail exit 2
with `output_target_invalid`. Python code must call the native primitive through
`ctypes`, a checked helper, or a standard-library wrapper with the same
exclusive guarantee. `os.rename`, `os.replace`, copy-into-final, and
create-final-then-fill are non-conformant. The guarantee is visibility atomicity,
not fsync/power-loss durability; crashes may leave stale temp directories but
never valid AgenTrust outputs.
`make verify-agentrust` conformance requires a CPython environment with packages
installed from `integrations/agentrust/requirements-v1alpha1.lock`; the lock
pins package versions and hashes, not the interpreter path, Python minor version,
or CPU architecture. Platform support is claimed only when the environment has a
native non-replacing directory promotion primitive. Linux with
`renameat2(RENAME_NOREPLACE)` and macOS with `renamex_np(RENAME_EXCL)` are the
supported primitive families for v1alpha1. If the syscall exists but the mounted
filesystem returns unsupported/invalid-operation for exclusive directory rename,
including network or legacy filesystems, the verifier exits 2 with
`output_target_invalid`.

Concurrent same-bundle runs are unsupported. At most one may promote the final
target; others fail with `output_target_invalid`. Concurrent different-bundle
runs are allowed because cleanup is scoped to `.tmp-<bundle_id>-<nonce>`. The
default CLI writes no `failure-report.json`; the `failure_report` schema object is
reserved for implementation tests and future diagnostics.

The make harness may leave only per-fixture success directories under
`<fresh-out-dir>/<bundle_id>/`, `<fresh-out-dir>/verify-agentrust-report.json`,
and diagnostic JSON under `<fresh-out-dir>/.diagnostics/`; mutation working
directories are deleted best-effort before make returns. v1alpha1 never cleans
previous run directories under `integrations/agentrust/out/.verify-agentrust/`.
`.verify-agentrust/**` is make scratch and is excluded from repository/package
leak scans and source distribution inputs; public `integrations/agentrust/out/*`
outputs remain scanned. Diagnostic and mutation directories are never valid
AgenTrust outputs. Internal diagnostic files, when written, contain exactly the
single stderr diagnostic JSON from that invocation with one trailing newline;
stderr remains the source of truth for aggregate primary diagnostic codes.

`verification-report.json` is RFC 8785 canonical JSON with `status = "pass"` and
must validate against the `verification_report` schema source object.
`replay_cache` is the closed `replay_cache_report` object:
`status = "not_applicable_fixture"` and `path`, `operation_id`, and `replay_at`
are `null`. `outputs.selected_outputs` is exactly
`["trace","agent_manifest","spendguard_artifacts"]`; the report is not a selected
AgenTrust output. `spendguard_artifacts` is intentionally listed as a selected
output key even though `spendguard-artifacts.json` is a SpendGuard extension
artifact rather than an Agent Manifest artifact.

Human-readable report text, CLI summaries, generated README text, and diagnostic
summaries must use this exact status sentence:

```text
AgenTrust Community fixture only: TRACE Level 0 software-only; TRACE signed with a committed test fixture key; generated TRACE build_provenance.slsa_level=1 is package-required metadata, not a SLSA Level 1 claim; Agent Manifest is unsigned_not_verified; not AgenTrust Verified; not hardware attested; not production trust evidence.
```

The closed JSON `verification-report.json` must not add `verified`, `trace_l1`,
`slsa_claim`, or equivalent claim fields in v1alpha1. It conveys status only
through existing schema-source fields such as `mode`,
`trace_exporter.signature_status`, `trace_exporter.key_scope`,
`trace_exporter.warning`, and `agent_manifest.signature_status`.

Single-bundle CLI runs set `negative_tests_scope = "single_bundle_not_run"` in
each per-bundle `verification-report.json`. The required `negative_tests` object
is present with exactly four placeholder entries: `mutated_signature`,
`wrong_key`, `mutated_policy_hash`, and `missing_trace_signature`. Each placeholder
uses the table's `expected_exit_code`, `status = "not_run"`,
`observed_exit_code = null`, `diagnostic_code = null`, and
`outputs_absent = false`.

`make verify-agentrust` writes a separate make-level aggregate report at
`<fresh-out-dir>/verify-agentrust-report.json` that validates against
`make_verification_report`. Actual success fixture pass/fail/not_run entries live
in `success_fixtures`, keyed exactly by `allow` and `deny`; each value records
`status`, `bundle_id`, `outputs_match`, and `diagnostic_code`. A per-fixture
golden mismatch records `status = "fail"`, `outputs_match = false`, and
`diagnostic_code = "generated_output_schema_invalid"`, then make exits 1 after
remaining runnable checks finish. `conformance_checks` is keyed exactly by
`artifact_preflight_envelope`, `event_type_binding`, `fixture_identity`,
`internal_guard`, `manifest_issuer_variant`, `package_guard_surfaces`,
`private_parser_matrix`, and `public_parser_matrix`;
each value records `status` and `diagnostic_code`, and `status` is `pass` only
after the required stdout/stderr/exit or surface checks pass. Actual negative-test
pass/fail/not_run entries live in that aggregate report, not in per-bundle success
reports. The aggregate
`negative_tests` object has exactly the same four keys. Each value records
`status`, `expected_exit_code`, `observed_exit_code`, `diagnostic_code`, and
`outputs_absent`. Negative `status` is `pass` only when the exit code matches and
no output target or partial success file exists.

Allowed make-level `diagnostic_code` values:

| Negative key | Expected diagnostic code |
|---|---|
| `mutated_signature` | `spendguard_signature_invalid` |
| `wrong_key` | `spendguard_signature_invalid` |
| `mutated_policy_hash` | `policy_binding_mismatch` |
| `missing_trace_signature` | `trace_package_validation_failed` |

Any other diagnostic code in the aggregate report fails `make verify-agentrust`.
`wrong_key` keeps the same `key_id` and authorization metadata but changes only
the Ed25519 public key bytes; key selection and authorization must succeed, and
any earlier trust-store validation failure makes that aggregate entry `fail`
because the mutation did not reach the intended phase-9 cryptographic verifier.

## Package Pins And Artifacts

Accepted v1alpha1 package versions:

| Package | Accepted version |
|---|---:|
| `agentrust-trace` | `==0.2.0` |
| `agentrust-trace-tests` | `==0.1.0` |
| `agent-manifest` | `==0.1.1` |

Package version authority is `importlib.metadata.version(<distribution-name>)`
for installed Python distributions in the active interpreter. Editable installs
are allowed only when they expose normal distribution metadata with the exact
accepted version. Vendored modules, path-shadowed modules without distribution
metadata, dependency metadata absence, or importing a module whose distribution
version cannot be resolved are `package_missing` or `package_version_mismatch`
according to whether metadata is absent or present-but-wrong. Package validators
must be treated as pass/fail predicates: the verifier passes deep copies to
package validation APIs, discards returned model objects, and serializes only the
original generated dicts after validation passes.

The spec intentionally does not embed generated JSON Schema bodies, fixture
payloads, full package probes, full vector JSON, or full generated output JSON.
Reviewers must use these
committed artifacts by path and SHA-256:

| Artifact | Role | SHA-256 |
|---|---|---|
| `docs/specs/agentrust-schema-source-v1alpha1.json` | Compact schema source for closed local objects. | `04b19e11ad39a0717774a2725dba9c3eab8ce4a8f72a7a2f0a8f93426d7ed54f` |
| `docs/specs/agentrust-package-probe-2026-07-01.json` | Package/API compatibility probe for accepted AgenTrust packages. | `6eb4331cd54f33e83e0e32f023332f3fd52ef0d9782187eba2a0d8729b52c68b` |
| `docs/specs/agentrust-bundle-root-vector-2026-07-01.json` | Evidence-bundle root canonicalization vector. | `d52305368187a3b1e1a0c8afc12ef45103a1e63bdfb01eff0d3f858df38dacce` |
| `docs/specs/agentrust-manifest-uuidv7-vector-2026-07-01.json` | Deterministic Manifest UUIDv7 vectors. | `800317f1653d88a60ec57c91b3751dec835d6a43bf99a133dab30dc99ea09d8d` |
| `docs/specs/agentrust-cloudevent-signature-vector-2026-07-01.json` | SpendGuard CloudEvent protobuf and ledger JSON canonicalization/signature vector. | `1e499623035dad7dcdb4d20c6bcc7167140b8eb9282b4310503cfe35395bc902` |
| `proto/spendguard/common/v1/common.proto` | Normative `spendguard.common.v1.CloudEvent` descriptor for non-ledger SpendGuard signature verification. | `5326c86ac4f03dd8790af751c80b4358e332fe7904d493ba0c1ef18faefede4a` |
| `integrations/agentrust/examples/fixture-sha256-manifest.v1alpha1.json` | SHA manifest for committed public fixtures and test keys. | `78ad0d7be7fb2c6eb91105d5fee5964789ce4b8fa161d7087e084ede741e7102` |
| `integrations/agentrust/examples/generated-output-sha256-manifest.v1alpha1.json` | SHA manifest for committed expected generated outputs. | `def85d6499d48398614736386cdbbd99a03cb44a29378da9aab06f75d42b6058` |
| `docs/specs/check-agentrust-vectors.py` | Executable vector/probe consistency check. | `eb723e1073576a87e8976b30e7f4eb33840b3237f4b003478c166fa7e0d38366` |
| `integrations/agentrust/requirements-v1alpha1.lock` | Hash-locked AgenTrust direct and transitive Python package dependencies for conformance. | `e3829562a20a4e5d646c1db0115089e373d71f20db4b1990b63e9de09618cc26` |

All rows are already committed immutable v1alpha1 inputs; implementation reads
them by path and verifies SHA-256 before use.

`docs/specs/agentrust-schema-source-v1alpha1.json` is the docs/spec review
source. Slice 1 copies it to
`integrations/agentrust/schema-source/v1alpha1.schema-source.json` and generates
local JSON Schemas under `integrations/agentrust/schemas/*.schema.json`.

`make verify-agentrust` is conformant only in a CPython environment where the
AgenTrust Python dependencies are installed from
`integrations/agentrust/requirements-v1alpha1.lock` using hash verification and
an offline wheel cache. The lock file pins package versions and hashes; it does
not make the package probe's local interpreter path, Python minor version,
architecture, or `module_file` paths normative. The command must not resolve
dependencies or access the network during make conformance. Runtime package
checks still use `importlib.metadata.version(...)`, package probe validation,
and the vector checks above, so the lock file is an environment reproducibility
artifact, not a replacement for package API validation. The `build` frontend and
any build backend dependency used by `python -m build --no-isolation` must also
come from this lock/cache; otherwise packaging guard setup fails.

Package probe drift checks are exact:

1. Load `docs/specs/agentrust-package-probe-2026-07-01.json` and verify its
   SHA-256 equals the table above.
2. Verify `python_modules.agentrust_trace.version == "0.2.0"`,
   `python_modules.agentrust_trace.sign_record_contract.adds_cnf == true`,
   `python_modules.agentrust_trace.sign_record_contract.adds_signature == true`,
   and `python_modules.agentrust_trace.has_verify_record == false`.
3. Verify `python_modules.agent_manifest.version == "0.1.1"` and
   `python_modules.agentrust_trace_tests.version == "0.1.0"`.
4. Execute exactly these validation calls against the probe's accepted shapes:

```python
trace = probe["accepted_json_shapes"]["trace_signed_level0"]
TrustRecord.model_validate(trace)
validate_json(trace)

manifest = probe["accepted_json_shapes"]["agent_manifest_unsigned"]
Manifest.model_validate(manifest)
```

Probe file missing, SHA mismatch, JSON parse failure, missing expected probe
paths, import/version mismatch, or any failed validation call above exits 6
because the local package environment drifted before SpendGuard output
generation. The same validation calls against newly generated SpendGuard TRACE
or Manifest objects exit 5 on failure because package prerequisites already
passed and the generated output is invalid. Package APIs are validation
boundaries only: the implementation must ignore package-normalized serialization,
package exception text, and returned model field ordering. TRACE/Manifest byte
stability is owned by SpendGuard writers defined in their mapping sections.

## `make verify-agentrust`

`make verify-agentrust` is the Community conformance command. It must run without
network access after dependencies are installed, run
`docs/specs/check-agentrust-vectors.py`, validate package versions, verify allow
and deny fixtures, generate the four selected success output files for each
public fixture in a fresh output root, and run the four negative mutations below.
Ordering is success `allow`, success `deny`, then each mutation exactly once in
table order. Exit 0 requires all setup checks, success fixtures, output-absence
checks, and negative entries to pass. If an aggregate report can be written, any
`fail` or required `not_run` entry makes make exit 1 after all runnable mutations
complete; make must not return expected verifier exits such as 3, 4, or 5.
Setup failure before aggregate report creation returns the setup failure code.

The public CLI accepts only the exact allow/deny bundle paths listed in Path
Safety. For negative tests, the make harness creates mutated fixture copies under
`<fresh-out-dir>/.mutations/<mutation>/<fixture>/bundle` and invokes the verifier
core through a private internal test entry point that applies the same schema,
canonical, trust, lifecycle, package, and output checks while allowing that
mutation root and `<mutation-out-dir>`. The public verifier must reject those
hidden paths. Mutations that need valid wrapper hashes or evidence roots
recompute only the hashes named in the table; they never rewrite committed
fixtures.

Internal mutation fixture identity is the `<fixture>` path segment, either
`allow` or `deny`, and must match the copied committed fixture. Public root-profile
file-set rules still apply: `allow` requires listed physical `outcome.json`;
`deny` forbids physical/listed `outcome.json`. `<fresh-out-dir>` is
`integrations/agentrust/out/.verify-agentrust/<run-id>`, created by make before
internal checks. `<mutation-out-dir>` is created for one negative invocation and
must not contain `<bundle_id>/` before that invocation. The public CLI never
accepts `<fresh-out-dir>` or mutation paths.

The internal test entry point is not a public CLI flag. It is enabled only when
invoked by `make verify-agentrust` with
`SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT=1`; production/package entry points and
installed console scripts must not expose it. The private invocation grammar is:

```text
python -m spendguard_agentrust._verify_internal \
  --mode fixture \
  --fixture-profile upstream \
  --bundle <public-bundle-root-or-mutation-bundle-root> \
  --trust-store <public-trust-store-or-mutation-trust-store> \
  --trace-signing-key <trace-key-copy> \
  --out-dir <fresh-out-dir-or-mutation-out-dir> \
  [--manifest-issuer <spiffe-uri>] \
  --mutation <none|mutated_signature|wrong_key|mutated_policy_hash|missing_trace_signature>
```

`--manifest-issuer` is accepted only for private success invocations; private
negative invocations reject it.

`_verify_internal` is private, not a console script and not a documented public
module. Make invokes it from the source checkout with that checkout on
`PYTHONPATH`. It aborts exit 2 `cli_parse_error` unless
`SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT=1`, the current repo root is the source
checkout, loaded package/module realpaths are inside that checkout and not an
installed artifact, `--out-dir` matches `<fresh-out-dir>` for success or
`<mutation-out-dir>` for a negative invocation, and hidden segments match Path
Safety's internal grammar. Normative spec files may mention the internal entry
point only if excluded from wheel/sdist/container contexts. Generated scripts,
declared context tars, README/docs inside built artifacts, public CLI help, and
production entry points must not expose `_verify_internal`, `--mutation`, or
`SPENDGUARD_AGENTRUST_INTERNAL_TEST_ROOT`; any public exposure is
`packaging_guard_failed`. After the gate passes, `_verify_internal` uses public
token precedence/diagnostics plus the internal `--mutation` enum. Every spawned
verifier still parses arguments before package imports and runs package
prerequisites after phase 3.

The internal entry point may mutate generated in-memory objects only after all
input, trust, lifecycle, and artifact-binding checks pass and immediately before
final package validation/output commit. For success fixtures it must not skip
signing, local schema validation, package validation, or output atomicity. For a
negative mutation, validation may stop at the expected failing phase, but earlier
phases must still run and later phases must not create outputs after the expected
failure.

`missing_trace_signature` is the only output-pipeline mutation. It is a make-only
test hook applied to the in-memory TRACE dict immediately after
`sign_record(...)` returns and before TRACE package validation and before any
temporary output directory is created. The public CLI exposes no option to invoke
this hook, and normal generation still uses `sign_record(...)` as the only TRACE
signing API.
For this mutation, candidate objects may exist in memory, but nothing may be
serialized or written. TRACE package validation runs next and must fail with
`trace_package_validation_failed`; later output validation and commit are skipped.

| Mutation | Fixture | Required setup | Expected exit | Output expectation |
|---|---|---|---:|---|
| `mutated_signature` | `allow` | In the mutation bundle copy, flip one base64url character in `decision.json.signature.value_base64url` without changing signed fields, then recompute only the `decision.json` raw file hash in `bundle.json.files` and recompute `bundle.json.evidence_bundle_hash`. | 3 | No target directory or partial success file. |
| `wrong_key` | `deny` | Use `<mutation-trust-store>` whose key ID and authorization metadata match the fixture trust store but whose Ed25519 public key differs; rewrite that trust store as canonical JSON and require it to pass phase-3 scanner checks and full phase-7 trust-store validation. The bundle copy is byte-identical to the source fixture. | 3 | No target directory or partial success file. |
| `mutated_policy_hash` | `allow` | In the mutation bundle copy, change `policy-bundle.json.policy_bundle_hash`, keep the signed decoded `$.spendguard.policy_bundle_hash` unchanged, then recompute the `policy-bundle.json` raw file hash in `bundle.json.files` and recompute `bundle.json.evidence_bundle_hash`. | 4 | No target directory or partial success file. |
| `missing_trace_signature` | `deny` | Remove `signature` from the generated TRACE dict before final TRACE package validation. | 5 | No final target directory, no partial success file, and no temporary output directory is created. |

## Exit Codes

| Code | Meaning |
|---:|---|
| 0 | v1alpha1 fixture verification passed. |
| 2 | CLI, path, schema, canonical JSON, required field, fixture privacy, output target, or unsupported flag error. |
| 3 | SpendGuard signature or fixture trust-store verification failed. |
| 4 | Bundle hash, canonical-byte hash, trust-key time-window, lifecycle, or artifact binding mismatch. |
| 5 | Generated TRACE, Agent Manifest, or SpendGuard-owned output validation failed. |
| 6 | Missing or incompatible pinned package/helper prerequisite. |

On failure, the public CLI writes exactly one compact JSON diagnostic object to
stderr followed by `\n`; stdout is empty. The object has exactly these keys in
this insertion order: `schema`, `exit_code`, `phase`, `code`, `message`,
`json_pointer`. `schema` is `spendguard-agentrust-diagnostic/v1`; `exit_code` is
the process exit code; `phase` is the diagnostic family name from the table below;
`code` is the primary diagnostic code; `message` is a non-empty ASCII string for
humans; `json_pointer` is an RFC 6901 pointer or `null`. The diagnostic object is
not a selected output file and is not written to `--out-dir`.

`make verify-agentrust` obtains a negative test's primary diagnostic by parsing
that stderr JSON from the internal verifier invocation. If the invocation exits
with the expected code but stderr is empty, malformed, has multiple JSON objects,
or uses a code outside the allowed table, the aggregate negative-test entry is
`fail`.

Diagnostic code registry:

| Code | Phase | Exit |
|---|---|---:|
| `cli_parse_error` | CLI/path/output | 2 |
| `cli_missing_value` | CLI/path/output | 2 |
| `cli_missing_required_flag` | CLI/path/output | 2 |
| `cli_value_invalid` | CLI/path/output | 2 |
| `cli_equals_form_unsupported` | CLI/path/output | 2 |
| `unsupported_capability` | CLI/path/output | 2 |
| `path_safety_error` | CLI/path/output | 2 |
| `output_target_invalid` | CLI/path/output | 2 |
| `bundle_index_invalid` | Bundle index/file set/privacy | 2 |
| `file_set_invalid` | Bundle index/file set/privacy | 2 |
| `fixture_privacy_violation` | Bundle index/file set/privacy | 2 |
| `package_missing` | Package prerequisites | 6 |
| `package_version_mismatch` | Package prerequisites | 6 |
| `package_probe_drift` | Package prerequisites | 6 |
| `schema_source_mismatch` | Package prerequisites | 6 |
| `trace_key_invalid` | Package prerequisites | 6 |
| `packaging_guard_failed` | Package prerequisites | 6 |
| `schema_invalid` | Local schema/canonical/hash | 2 |
| `canonical_json_invalid` | Local schema/canonical/hash | 2 |
| `media_type_invalid` | Local schema/canonical/hash | 2 |
| `file_hash_mismatch` | Local schema/canonical/hash | 4 |
| `evidence_bundle_hash_mismatch` | Local schema/canonical/hash | 4 |
| `data_b64_mismatch` | Local schema/canonical/hash | 2 |
| `cloudevent_canonical_hash_mismatch` | Local schema/canonical/hash | 4 |
| `trust_store_invalid` | Local schema/canonical/hash | 2 |
| `trust_key_not_found` | Trust/signature | 3 |
| `trust_authorization_failed` | Trust/signature | 3 |
| `trust_key_time_mismatch` | Trust/signature | 4 |
| `spendguard_signature_invalid` | Trust/signature | 3 |
| `lifecycle_mismatch` | Lifecycle/bindings | 4 |
| `policy_binding_mismatch` | Lifecycle/bindings | 4 |
| `tool_binding_mismatch` | Lifecycle/bindings | 4 |
| `build_binding_mismatch` | Lifecycle/bindings | 4 |
| `trace_package_validation_failed` | Package output validation | 5 |
| `manifest_package_validation_failed` | Package output validation | 5 |
| `manifest_layout_invalid` | Package output validation | 5 |
| `generated_output_schema_invalid` | Package output validation | 5 |

This registry is exhaustive. No diagnostic code outside this table may be
emitted by the public CLI, private make verifier, make aggregate report, or
diagnostic JSON files. The edge-case mapping table below is explanatory and
cannot introduce additional codes.

Common mappings not obvious from the registry: malformed `--manifest-issuer`
uses `cli_value_invalid`; malformed bundle index or `files` entries use
`bundle_index_invalid`; schema-source SHA/generation failure uses
`schema_source_mismatch`; package import absence uses `package_missing`; package
version mismatch uses `package_version_mismatch`; TRACE key format failure uses
`trace_key_invalid`; package probe drift uses `package_probe_drift`; unsupported
`.dockerignore`, package/container key leakage, generated `integrations/agentrust/out/`
package members, or public exposure of `_verify_internal` use
`packaging_guard_failed`; generated Manifest layout drift uses
`manifest_layout_invalid`.

When multiple failures are present, the verifier reports the earliest phase in
this precedence order and suppresses later diagnostics:

1. CLI parse, unsupported or unknown flags, path safety, and output target
   checks that do not require parsed bundle contents: exit 2.
2. `bundle.json` index parse for ordering only, including UTF-8 JSON syntax,
   duplicate-key rejection, top-level object requirement, `bundle_id` token
   validation, and `files` entries that are objects exposing string `path`,
   `media_type`, and `sha256`: exit 2. Output target checks that require
   `bundle_id`, including stale temp cleanup for that bundle, run immediately
   after this index parse and before package imports; failures exit 2 with
   `output_target_invalid`.
3. File-set and forbidden-file checks comparing actual bundle entries to
   `bundle.json.files`: exit 2.
4. Schema-source SHA/generation checks, required AgenTrust package import,
   pinned-version checks, package probe drift checks, and TRACE key format
   checks: exit 6. The package/container guard is make-setup-only: `make
   verify-agentrust` runs it before any verifier invocation and reports exit 6
   `packaging_guard_failed` on setup failure.
5. Full `bundle.json` schema/canonical checks: exit 2; `evidence_bundle_hash`
   mismatch: exit 4.
6. Raw file hash mismatches for listed non-`bundle.json` files: exit 4. Hash
   mismatch suppresses JSON, privacy, and schema diagnostics for that file.
7. Privacy scanning, non-`bundle.json` JSON syntax, duplicate-key rejection,
   canonical JSON, local closed-schema validation, media-type/profile checks, and
   full `trust-store.json` validation: exit 2. For `decision.json` and
   `outcome.json`, wrapper object schema, `spendguard_cloudevent`, and selected
   `data_json` payload schema fail here.
8. Procedural signed-event checks after wrapper schema passes:
   `data_b64` byte comparison, canonical signed-byte reconstruction, and
   `canonical_bytes_sha256` comparison. Malformed canonical reconstruction exits
   2 with `canonical_json_invalid`; decoded `data_b64` bytes that do not exactly
   equal canonical `data_json` bytes exit 2 with `data_b64_mismatch`;
   reconstructed signed bytes whose hash differs from wrapper
   `canonical_bytes_sha256` exit 4 with `cloudevent_canonical_hash_mismatch`.
9. Trust-store key selection, authorization, selected-key time-window check, and
   SpendGuard Ed25519 signature verification: exit 3 for key-not-found,
   authorization, or signature failure; exit 4 with `trust_key_time_mismatch` for
   selected-key time-window mismatch.
10. Lifecycle derivation and signed artifact-binding mismatches: exit 4.
11. TRACE, Agent Manifest, and SpendGuard-owned generated output validation:
   exit 5.

Within phases 8 and 9, event processing order is always `decision.json` first,
then `outcome.json` if listed. If decision procedural canonicalization, trust, or
signature verification fails, outcome verification is not used for the primary
diagnostic. If decision passes and outcome fails, the outcome failure is primary.
Phase 10 runs only after all required event phase-8 and phase-9 checks pass.
Within phase 10, lifecycle/linkage checks run before artifact bindings; artifact
bindings run in this order: policy, tool catalog, then build provenance. Thus an
outcome trust failure beats any policy/tool/build mismatch, and a policy binding
mismatch beats a simultaneous tool or build mismatch.

Phase 7 uses the file-local sub-order described in Fixture Privacy before the
generic same-phase sorter. Otherwise, if two errors occur in the same phase,
diagnostics are sorted by bundle file order (`bundle.json`, `decision.json`,
`outcome.json`, `policy-bundle.json`, `tool-catalog.json`,
`build-provenance.json`, `trust-store.json`) and then by JSON pointer. A concrete
JSON pointer sorts before `null`; two non-pointer diagnostics sort by the
diagnostic code order shown in the table above.
