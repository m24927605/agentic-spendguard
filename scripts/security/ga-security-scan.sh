#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

output_dir="docs/reviews/ga-readiness/GA_09_security_signoff_supply_chain"
require_external_tools=false

usage() {
  cat <<'USAGE'
Usage:
  scripts/security/ga-security-scan.sh [--output-dir DIR] [--require-external-tools]

Runs the GA_09 security and supply-chain gate:
  - Helm demo + production renders
  - production Helm values validator
  - container, SVID, RLS, replay, PII, and workflow invariant checks
  - deterministic Cargo dependency SBOM evidence

By default the gate is fully local and records missing optional scanners.
Use --require-external-tools for release signoff; that mode fails closed unless
syft, trivy, cosign, and cargo-audit are installed.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output-dir)
      output_dir="${2:-}"
      shift 2
      ;;
    --require-external-tools)
      require_external_tools=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

mkdir -p "$output_dir"
output_dir="$(cd "$output_dir" && pwd -P)"

commit_sha="$(git rev-parse HEAD)"
branch_name="$(git rev-parse --abbrev-ref HEAD)"
scan_started_utc="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
worktree_status="$(git status --porcelain)"
worktree_dirty_at_start=false
if [[ -n "$worktree_status" ]]; then
  worktree_dirty_at_start=true
fi

if [[ "$require_external_tools" == "true" && "$worktree_dirty_at_start" == "true" ]]; then
  echo "release-mode security scan requires a clean git worktree" >&2
  git status --short >&2
  exit 1
fi

run_and_capture() {
  local name="$1"
  shift
  local log_file="$output_dir/${name}.txt"
  {
    printf '$'
    printf ' %q' "$@"
    printf '\n\n'
    "$@"
  } >"$log_file" 2>&1
}

tool_version() {
  local tool="$1"
  shift
  if command -v "$tool" >/dev/null 2>&1; then
    printf '%s: ' "$tool"
    "$@" 2>&1 | head -n 1
  else
    printf '%s: MISSING\n' "$tool"
  fi
}

{
  printf 'commit=%s\n' "$commit_sha"
  printf 'branch=%s\n' "$branch_name"
  printf 'scan_started_utc=%s\n' "$scan_started_utc"
  tool_version helm helm version --short
  tool_version python3 python3 --version
  tool_version cargo cargo --version
  tool_version syft syft version
  tool_version trivy trivy --version
  if command -v cosign >/dev/null 2>&1; then
    printf 'cosign: '
    cosign version --json 2>/dev/null | python3 -c 'import json, sys; print(json.load(sys.stdin)["gitVersion"])'
  else
    printf 'cosign: MISSING\n'
  fi
  tool_version cargo-audit cargo audit --version
} >"$output_dir/tool-versions.txt"

missing_external=()
for tool in syft trivy cosign cargo-audit; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    missing_external+=("$tool")
  fi
done

if [[ "$require_external_tools" == "true" && ${#missing_external[@]} -gt 0 ]]; then
  printf 'missing required release security tools: %s\n' "${missing_external[*]}" >&2
  printf 'Install path:\n' >&2
  printf '  brew install syft trivy cosign cargo-audit\n' >&2
  exit 1
fi

run_and_capture helm-demo helm template spendguard charts/spendguard --set chart.profile=demo
helm template spendguard charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production >"$output_dir/helm-production.yaml"
run_and_capture production-helm-validator scripts/release/validate-production-helm-values.sh charts/spendguard/values-production.example.yaml --rendered-manifest "$output_dir/helm-production.yaml"

metadata_raw="$(mktemp)"
cargo metadata --format-version 1 --locked >"$metadata_raw"
python3 - "$metadata_raw" "$output_dir/cargo-metadata.txt" "$output_dir/cargo-sbom.json" <<'PY'
import json
import sys
from pathlib import Path

raw_path = Path(sys.argv[1])
metadata_out = Path(sys.argv[2])
sbom_out = Path(sys.argv[3])
root = Path.cwd().resolve()
home = Path.home().resolve()
cargo_registry_src = home / ".cargo" / "registry" / "src"


def sanitize_string(value: str) -> str:
    value = value.replace(str(root), "$REPO")
    value = value.replace(str(cargo_registry_src), "$CARGO_REGISTRY_SRC")
    value = value.replace(str(home), "$HOME")
    return value


def sanitize(value):
    if isinstance(value, dict):
        return {key: sanitize(item) for key, item in value.items()}
    if isinstance(value, list):
        return [sanitize(item) for item in value]
    if isinstance(value, str):
        return sanitize_string(value)
    return value


def stable_manifest_path(value):
    if not value:
        return None
    path = Path(value)
    try:
        return path.resolve().relative_to(root).as_posix()
    except Exception:
        if str(cargo_registry_src) in value or "/.cargo/registry/" in value:
            return None
        return sanitize_string(value)


metadata = json.loads(raw_path.read_text(encoding="utf-8"))
sanitized_metadata = sanitize(metadata)
metadata_out.write_text(
    "$ cargo metadata --format-version 1 --locked\n\n"
    + json.dumps(sanitized_metadata, indent=2, sort_keys=True)
    + "\n",
    encoding="utf-8",
)

packages = []
for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
    packages.append(
        {
            "name": package["name"],
            "version": package["version"],
            "id": sanitize_string(package["id"]),
            "source": sanitize_string(package["source"]) if package.get("source") else None,
            "license": package.get("license"),
            "manifest_path": stable_manifest_path(package.get("manifest_path")),
        }
    )
sbom_out.write_text(
    json.dumps(
        {
            "schema": "spendguard.local-cargo-sbom.v1alpha1",
            "path_policy": "repository paths are relative; registry cache paths are stripped",
            "package_count": len(packages),
            "packages": packages,
        },
        indent=2,
        sort_keys=True,
    )
    + "\n",
    encoding="utf-8",
)
PY
rm -f "$metadata_raw"

if command -v syft >/dev/null 2>&1; then
  syft file:Cargo.lock \
    --source-name agentic-spendguard-cargo-lock \
    --source-version "$commit_sha" \
    -o spdx-json >"$output_dir/syft-sbom.spdx.json"
fi

if command -v trivy >/dev/null 2>&1; then
  trivy fs \
    --scanners vuln \
    --severity HIGH,CRITICAL \
    --ignore-unfixed \
    --exit-code 1 \
    --format json \
    --output "$output_dir/trivy-fs.json" Cargo.lock
fi

if command -v cargo-audit >/dev/null 2>&1; then
  cargo audit --json >"$output_dir/cargo-audit.json"
fi

python3 - "$output_dir" "$commit_sha" "$branch_name" "$scan_started_utc" "${missing_external[*]-}" "$worktree_dirty_at_start" <<'PY'
import json
import re
import subprocess
import sys
from pathlib import Path

output_dir = Path(sys.argv[1])
commit_sha = sys.argv[2]
branch_name = sys.argv[3]
scan_started_utc = sys.argv[4]
missing_external = [item for item in sys.argv[5].split() if item]
worktree_dirty_at_start = sys.argv[6] == "true"
root = Path.cwd()
errors = []
checks = {}


def record(name: str, ok: bool, detail: str) -> None:
    checks[name] = {"ok": ok, "detail": detail}
    if not ok:
        errors.append(f"{name}: {detail}")


def text(path: str) -> str:
    return (root / path).read_text(encoding="utf-8")


runtime_dockerfiles = sorted(
    path for path in (root / "deploy/demo/runtime").glob("Dockerfile.*") if path.is_file()
)
missing_user = [
    str(path.relative_to(root))
    for path in runtime_dockerfiles
    if "USER 65532:65532" not in path.read_text(encoding="utf-8")
]
record(
    "runtime_dockerfiles_user_65532",
    not missing_user,
    "all runtime Dockerfiles set USER 65532:65532"
    if not missing_user
    else ", ".join(missing_user),
)

workflow = text(".github/workflows/publish-images.yml")
record("publish_workflow_trivy", "aquasecurity/trivy-action" in workflow, "Trivy scan step present")
record("publish_workflow_sbom", re.search(r"^\s+sbom:\s*", workflow, re.M) is not None, "Buildx SBOM enabled")
record(
    "publish_workflow_provenance",
    re.search(r"^\s+provenance:\s*", workflow, re.M) is not None,
    "Buildx provenance enabled",
)
record("publish_workflow_cosign", "cosign sign --yes" in workflow, "cosign signing step present")
mutable_tokens = ["value=latest", "latest-main", ":{<tag>,latest}", ":latest}"]
mutable_hits = [token for token in mutable_tokens if token in workflow]
record(
    "publish_workflow_no_latest_promotion",
    not mutable_hits,
    "no latest/latest-main promotion"
    if not mutable_hits
    else f"mutable tokens present: {', '.join(mutable_hits)}",
)
record("publish_workflow_oidc", "id-token: write" in workflow, "OIDC permission present for keyless signing")
record(
    "publish_workflow_dispatch_has_sha_tag",
    "github.event_name == 'workflow_dispatch'" in workflow
    and "type=sha,prefix=sha-,format=short" in workflow,
    "manual dispatch publishes immutable sha tag",
)

sidecar_dockerfile = text("deploy/demo/runtime/Dockerfile.sidecar")
sidecar_entrypoint = text("deploy/demo/runtime/sidecar-entrypoint.sh")
pki_init = text("deploy/demo/init/pki/generate.sh")
bundles_init = text("deploy/demo/init/bundles/generate.sh")
compose = text("deploy/demo/compose.yaml")
record(
    "sidecar_image_precreates_secret_links",
    "/var/run/secrets/spendguard/tls.crt" in sidecar_dockerfile
    and "chown -R 65532:65532 /var/run/secrets/spendguard /var/run/spendguard" in sidecar_dockerfile,
    "sidecar image prepares root-owned paths before USER switch",
)
root_only_sidecar_tokens = [
    "update-ca-certificates",
    "/usr/local/share/ca-certificates",
    "mkdir -p /var/run/secrets/spendguard",
    "chmod 0755 /var/run/spendguard",
]
root_only_sidecar_hits = [token for token in root_only_sidecar_tokens if token in sidecar_entrypoint]
record(
    "sidecar_entrypoint_nonroot_safe",
    not root_only_sidecar_hits and "/var/run/spendguard is not writable" in sidecar_entrypoint,
    "sidecar entrypoint only verifies mounted paths after USER switch"
    if not root_only_sidecar_hits
    else f"root-only runtime operations present: {', '.join(root_only_sidecar_hits)}",
)
record(
    "pki_volume_chowned_for_runtime_uid",
    'chown -R 65532:65532 "$OUT"' in pki_init,
    "pki-init hands cert/key volume to runtime UID 65532",
)
record(
    "pki_ca_key_remains_root_only",
    'chown 0:0 "$OUT/ca.key"' in pki_init and 'chmod 0600 "$OUT/ca.key"' in pki_init,
    "pki-init keeps demo CA private key out of runtime UID",
)
record(
    "bundles_volume_chowned_for_runtime_uid",
    'chown -R 65532:65532 "$OUT"' in bundles_init and "ensure_nonroot_ownership" in bundles_init,
    "bundles-init hands writable bundle volume to runtime UID 65532",
)
record(
    "compose_sidecar_uds_volume_handoff",
    "sidecar-uds-init:" in compose
    and "chown -R 65532:65532 /var/run/spendguard" in compose
    and "sidecar-uds-init:" in compose.split("sidecar:", 1)[0],
    "compose hands existing sidecar UDS named volume to runtime UID before sidecar starts",
)

production_values = text("charts/spendguard/values-production.example.yaml")
rendered = (output_dir / "helm-production.yaml").read_text(encoding="utf-8")
published_services = set(re.findall(r"^\s*-\s+service:\s+([a-z0-9-]+)\s*$", workflow, re.M))
rendered_chart_services = set(
    re.findall(r"image:\s+\S+/spendguard/([a-z0-9-]+)(?=[:@])", rendered)
)
missing_published_images = sorted(rendered_chart_services - published_services)
record(
    "publish_workflow_covers_production_chart_images",
    not missing_published_images
    and "${{ env.REGISTRY }}/${{ env.IMAGE_OWNER }}/spendguard/${{ matrix.service }}" in workflow,
    f"publish workflow covers {len(rendered_chart_services)} production chart images under spendguard/<component>"
    if not missing_published_images
    else f"missing workflow images: {', '.join(missing_published_images)}",
)
plaintext_db = re.compile(r"(?i)\b(postgres(?:ql)?|mysql|mongodb)://")
record("production_values_no_plaintext_db", not plaintext_db.search(production_values), "no plaintext DB URL in production values")
record("production_render_no_plaintext_db", not plaintext_db.search(rendered), "no plaintext DB URL in production render")
record("production_render_has_networkpolicy", "kind: NetworkPolicy" in rendered, "NetworkPolicy rendered")
record("production_render_has_svid_certificate", "kind: Certificate" in rendered and "spiffe://spendguard.platform/predictor-client/" in rendered, "per-tenant SVID Certificate rendered")

def strip_sql_comments(sql: str) -> str:
    sql = re.sub(r"/\*.*?\*/", "", sql, flags=re.S)
    return "\n".join(line.split("--", 1)[0] for line in sql.splitlines())


sql_paths = list((root / "services").glob("*/migrations/**/*.sql")) + list((root / "services").glob("*/migrations/*.sql"))
rls_bypass_hits = []
for path in sql_paths:
    stripped = strip_sql_comments(path.read_text(encoding="utf-8"))
    if re.search(r"\b(CREATE|ALTER)\s+(ROLE|USER)\b[^\n;]*\bBYPASSRLS\b|\bBYPASSRLS\s*;", stripped, re.I):
        rls_bypass_hits.append(str(path.relative_to(root)))
record(
    "rls_no_bypassrls_grants",
    not rls_bypass_hits,
    "no executable BYPASSRLS grants"
    if not rls_bypass_hits
    else ", ".join(rls_bypass_hits),
)

replay_migration = text("services/canonical_ingest/migrations/0020_event_replay_dedup.sql")
record("replay_dedup_table", "canonical_event_replay_dedup" in replay_migration, "replay dedup table exists")
record(
    "replay_dedup_key",
    "PRIMARY KEY (producer_id, event_id)" in replay_migration and "UNIQUE (event_id)" in replay_migration,
    "producer/event and global event replay keys enforced",
)

tokenizer_security = text("services/tokenizer/src/shadow/security.rs")
tokenizer_worker = text("services/tokenizer/src/shadow/worker.rs")
record("pii_shadow_default_denied", "pii_shadow_enabled: false" in tokenizer_security, "PII shadow default denies raw text")
record("pii_shadow_worker_guard", "if !settings.pii_shadow_enabled" in tokenizer_worker, "shadow worker checks tenant opt-in")
record("count_tokens_quota_present", "count_tokens_quota_per_minute" in tokenizer_security, "per-tenant count_tokens quota present")

svid_template = text("charts/spendguard/templates/output_predictor_plugin_svid.yaml")
svid_runtime = text("services/output_predictor/src/plugin_svid.rs")
svid_template_ok = 'spiffe://spendguard.platform/predictor-client/%s' in svid_template
svid_runtime_ok = 'PREDICTOR_CLIENT_SVID_PREFIX: &str = "spiffe://spendguard.platform/predictor-client/"' in svid_runtime
record("svid_template_exact_uri", svid_template_ok, "Helm Certificate URI uses exact predictor-client tenant prefix")
record("svid_runtime_exact_uri", svid_runtime_ok, "runtime validator uses exact predictor-client tenant prefix")

sbom = json.loads((output_dir / "cargo-sbom.json").read_text(encoding="utf-8"))
record("cargo_sbom_generated", sbom.get("package_count", 0) > 0, f"{sbom.get('package_count', 0)} Cargo packages recorded")
local_path_tokens = [str(Path.home()), str(root)]
evidence_paths = [
    output_dir / "cargo-metadata.txt",
    output_dir / "cargo-sbom.json",
]
local_path_hits = []
for evidence_path in evidence_paths:
    contents = evidence_path.read_text(encoding="utf-8")
    for token in local_path_tokens:
        if token and token in contents:
            local_path_hits.append(f"{evidence_path.name}:{token}")
record(
    "cargo_evidence_no_local_paths",
    not local_path_hits,
    "cargo metadata/SBOM evidence strips developer-local paths"
    if not local_path_hits
    else ", ".join(local_path_hits),
)

summary = {
    "schema": "spendguard.ga09.security_scan.v1alpha1",
    "result": "pass" if not errors else "fail",
    "commit_sha": commit_sha,
    "branch": branch_name,
    "scan_started_utc": scan_started_utc,
    "worktree_dirty_at_start": worktree_dirty_at_start,
    "missing_external_tools": missing_external,
    "external_tool_install": "brew install syft trivy cosign cargo-audit",
    "release_mode": "scripts/security/ga-security-scan.sh --require-external-tools",
    "checks": checks,
    "errors": errors,
}
(output_dir / "scan-summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")

report_lines = [
    "# GA_09 Security Scan Evidence",
    "",
    f"- Result: {summary['result']}",
    f"- Commit: `{commit_sha}`",
    f"- Branch: `{branch_name}`",
    f"- Started UTC: `{scan_started_utc}`",
    f"- Worktree dirty at start: `{str(worktree_dirty_at_start).lower()}`",
    f"- Missing optional external tools: {', '.join(missing_external) if missing_external else 'none'}",
    f"- Release-mode command: `{summary['release_mode']}`",
    "",
    "## Checks",
    "",
]
for name, item in checks.items():
    status = "PASS" if item["ok"] else "FAIL"
    report_lines.append(f"- {status} `{name}`: {item['detail']}")
if missing_external:
    report_lines.extend(
        [
            "",
            "## External Scanner Install Path",
            "",
            "Install missing scanners before final release signing:",
            "",
            "```bash",
            summary["external_tool_install"],
            "scripts/security/ga-security-scan.sh --require-external-tools",
            "```",
        ]
    )
(output_dir / "README.md").write_text("\n".join(report_lines) + "\n", encoding="utf-8")

if errors:
    print("\n".join(errors), file=sys.stderr)
    sys.exit(1)
PY

printf 'GA_09 security scan PASS: %s\n' "$output_dir"
