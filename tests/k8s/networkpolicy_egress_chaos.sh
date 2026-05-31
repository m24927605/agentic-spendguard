#!/usr/bin/env bash
# Prove the chart's NetworkPolicy blocks direct external egress for
# spendguard.io/enforced=true app pods while allowing egress to the proxy.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO_ROOT}"

CLUSTER_NAME="${KIND_CLUSTER_NAME:-spendguard-netpol}"
NAMESPACE="${KIND_NAMESPACE:-spendguard-netpol}"
CTX="kind-${CLUSTER_NAME}"
CALICO_VERSION="${CALICO_VERSION:-v3.28.2}"
WORK_DIR="$(mktemp -d -t spendguard-netpol-XXXXXX)"

log() { echo "[netpol-chaos] $*" >&2; }
trap 'log "tempdir: ${WORK_DIR}"' EXIT

if ! command -v kind >/dev/null 2>&1; then
    log "FATAL: kind is required"
    exit 1
fi
if ! command -v kubectl >/dev/null 2>&1; then
    log "FATAL: kubectl is required"
    exit 1
fi

if ! kind get clusters | grep -q "^${CLUSTER_NAME}$"; then
    cat >"${WORK_DIR}/kind.yaml" <<EOF
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
networking:
  disableDefaultCNI: true
nodes:
  - role: control-plane
EOF
    log "creating kind cluster ${CLUSTER_NAME} with default CNI disabled"
    kind create cluster --name "${CLUSTER_NAME}" --config "${WORK_DIR}/kind.yaml" --wait 60s
    log "installing Calico ${CALICO_VERSION}"
    kubectl --context "${CTX}" apply -f "https://raw.githubusercontent.com/projectcalico/calico/${CALICO_VERSION}/manifests/calico.yaml" >/dev/null
    kubectl --context "${CTX}" -n kube-system rollout status daemonset/calico-node --timeout=180s
else
    log "using existing kind cluster ${CLUSTER_NAME}"
fi

kubectl --context "${CTX}" create namespace "${NAMESPACE}" --dry-run=client -o yaml \
    | kubectl --context "${CTX}" apply -f - >/dev/null

helm template spendguard charts/spendguard \
    --namespace "${NAMESPACE}" \
    --set networkPolicy.enabled=true \
    --show-only templates/networkpolicy.yaml \
    | kubectl --context "${CTX}" -n "${NAMESPACE}" apply -f - >/dev/null

cat >"${WORK_DIR}/proxy.yaml" <<'EOF'
apiVersion: apps/v1
kind: Deployment
metadata:
  name: spendguard-spendguard-egress-proxy
spec:
  replicas: 1
  selector:
    matchLabels:
      app.kubernetes.io/name: spendguard
      app.kubernetes.io/instance: spendguard
      app.kubernetes.io/component: egress-proxy
  template:
    metadata:
      labels:
        app.kubernetes.io/name: spendguard
        app.kubernetes.io/instance: spendguard
        app.kubernetes.io/component: egress-proxy
    spec:
      containers:
        - name: proxy
          image: python:3.12-alpine
          command: ["python", "-m", "http.server", "9000"]
          ports:
            - containerPort: 9000
---
apiVersion: v1
kind: Service
metadata:
  name: spendguard-spendguard-egress-proxy
spec:
  selector:
    app.kubernetes.io/name: spendguard
    app.kubernetes.io/instance: spendguard
    app.kubernetes.io/component: egress-proxy
  ports:
    - name: http
      port: 9000
      targetPort: 9000
EOF
kubectl --context "${CTX}" -n "${NAMESPACE}" apply -f "${WORK_DIR}/proxy.yaml" >/dev/null
kubectl --context "${CTX}" -n "${NAMESPACE}" rollout status deployment/spendguard-spendguard-egress-proxy --timeout=120s

run_curl() {
    local name="$1"
    local url="$2"
    local labels="${3:-}"
    local label_args=()
    if [ -n "${labels}" ]; then
        label_args=(--labels="${labels}")
    fi
    if [ "${#label_args[@]}" -gt 0 ]; then
        kubectl --context "${CTX}" -n "${NAMESPACE}" run "${name}" \
            --rm -i --restart=Never \
            --image=curlimages/curl:8.10.1 \
            "${label_args[@]}" \
            --command -- sh -c "curl -k -sS --connect-timeout 5 --max-time 8 -o /dev/null ${url}" >/tmp/"${name}".out 2>/tmp/"${name}".err
    else
        kubectl --context "${CTX}" -n "${NAMESPACE}" run "${name}" \
            --rm -i --restart=Never \
            --image=curlimages/curl:8.10.1 \
            --command -- sh -c "curl -k -sS --connect-timeout 5 --max-time 8 -o /dev/null ${url}" >/tmp/"${name}".out 2>/tmp/"${name}".err
    fi
}

log "checking cluster has baseline external egress before attributing deny to NetworkPolicy"
run_curl spendguard-netpol-control https://1.1.1.1

log "checking app pod can reach egress proxy"
run_curl spendguard-netpol-allow http://spendguard-spendguard-egress-proxy:9000 'spendguard.io/enforced=true'

log "checking direct external egress is blocked"
if run_curl spendguard-netpol-deny https://1.1.1.1 'spendguard.io/enforced=true'; then
    log "FATAL: enforced pod reached external HTTPS directly"
    cat /tmp/spendguard-netpol-deny.out >&2 || true
    exit 1
fi

log "PASS"
