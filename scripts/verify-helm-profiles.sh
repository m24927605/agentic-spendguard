#!/usr/bin/env bash
# Render the SpendGuard chart across supported profiles and security-relevant
# values combinations.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

OUT_DIR="${OUT_DIR:-/tmp/spendguard-helm-verify}"
mkdir -p "${OUT_DIR}"

log() { echo "[verify-helm] $*" >&2; }

render() {
    local name="$1"
    shift
    log "render ${name}"
    helm template spendguard charts/spendguard "$@" >"${OUT_DIR}/${name}.yaml"
}

render demo --set chart.profile=demo
render demo-networkpolicy --set chart.profile=demo --set networkPolicy.enabled=true
render production -f scripts/helm-validate-test-values.yaml
render production-networkpolicy -f scripts/helm-validate-test-values.yaml --set networkPolicy.enabled=true --set networkPolicy.acknowledgeNoNetworkPolicy=false
render production-kms -f scripts/helm-validate-test-values.yaml \
    --set signing.mode=kms \
    --set signing.kms.ledgerArn=arn:aws:kms:us-east-1:111122223333:key/ledger \
    --set signing.kms.sidecarArn=arn:aws:kms:us-east-1:111122223333:key/sidecar \
    --set signing.kms.webhookReceiverArn=arn:aws:kms:us-east-1:111122223333:key/webhook \
    --set signing.kms.ttlSweeperArn=arn:aws:kms:us-east-1:111122223333:key/ttl \
    --set signing.kms.controlPlaneArn=arn:aws:kms:us-east-1:111122223333:key/control-plane

if grep -RE "postgres(ql)?://[^[:space:]\"']*" "${OUT_DIR}" >/dev/null; then
    log "FATAL: rendered manifests contain plaintext postgres URL"
    grep -RE "postgres(ql)?://[^[:space:]\"']*" "${OUT_DIR}" >&2
    exit 1
fi

CONTROL_PLANE_KMS_SECTION="${OUT_DIR}/production-kms-control-plane.yaml"
awk '
    /^# Source: spendguard\/templates\/control-plane.yaml$/ { in_section=1 }
    /^# Source: / && $0 !~ /control-plane.yaml$/ { in_section=0 }
    in_section { print }
' "${OUT_DIR}/production-kms.yaml" >"${CONTROL_PLANE_KMS_SECTION}"
if grep -n "spendguard-signing-keys\\|/etc/spendguard/signing\\|control-plane.pem" "${CONTROL_PLANE_KMS_SECTION}" >/dev/null; then
    log "FATAL: KMS control-plane render still references local signing material"
    grep -n "spendguard-signing-keys\\|/etc/spendguard/signing\\|control-plane.pem" "${CONTROL_PLANE_KMS_SECTION}" >&2
    exit 1
fi

ruby -ryaml - "${OUT_DIR}/production-networkpolicy.yaml" <<'RUBY'
docs = YAML.load_stream(File.read(ARGV.fetch(0)))
errors = []

docs.each do |doc|
  next unless doc.is_a?(Hash)
  kind = doc["kind"]
  next unless ["Deployment", "Job", "CronJob"].include?(kind)
  name = doc.dig("metadata", "name")
  pod_spec =
    if kind == "CronJob"
      doc.dig("spec", "jobTemplate", "spec", "template", "spec")
    else
      doc.dig("spec", "template", "spec")
    end
  next unless pod_spec

  containers = pod_spec["containers"] || []
  containers.each do |container|
    cname = container["name"]
    sc = container["securityContext"] || {}
    errors << "#{kind}/#{name}/#{cname}: readOnlyRootFilesystem != true" unless sc["readOnlyRootFilesystem"] == true
    errors << "#{kind}/#{name}/#{cname}: allowPrivilegeEscalation != false" unless sc["allowPrivilegeEscalation"] == false
    drops = sc.dig("capabilities", "drop") || []
    errors << "#{kind}/#{name}/#{cname}: capabilities.drop missing ALL" unless drops.include?("ALL")

    (container["env"] || []).each do |env|
      next unless env["name"].to_s.end_with?("DATABASE_URL") || env["name"].to_s.start_with?("PG_")
      unless env.dig("valueFrom", "secretKeyRef", "name") && env.dig("valueFrom", "secretKeyRef", "key")
        errors << "#{kind}/#{name}/#{cname}: #{env["name"]} is not sourced from secretKeyRef"
      end
    end
  end
end

unless errors.empty?
  warn "FATAL: rendered workload assertions failed:"
  errors.each { |error| warn "  - #{error}" }
  exit 1
end
RUBY

log "PASS outputs=${OUT_DIR}"
