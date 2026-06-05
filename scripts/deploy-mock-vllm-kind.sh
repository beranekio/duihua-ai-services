#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${CLUSTER_NAME:-duihua-local}}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-120s}"
MOCK_VLLM_IMAGE="${MOCK_VLLM_IMAGE:-ghcr.io/beranekio/mock-vllm:latest}"
MANIFEST="${MANIFEST:-$ROOT_DIR/kind/mock-vllm.yaml}"

kubectl() {
  if [[ -n "${KUBECTL_CONTEXT}" ]]; then
    command kubectl --context "${KUBECTL_CONTEXT}" "$@"
  else
    command kubectl "$@"
  fi
}

echo "Deploying mock-vllm into namespace '${NAMESPACE}'..."
kubectl create namespace "${NAMESPACE}" --dry-run=client -o yaml | kubectl apply -f -

deployment_existed=false
if kubectl get deployment mock-vllm -n "${NAMESPACE}" >/dev/null 2>&1; then
  deployment_existed=true
fi

rendered_manifest="$(mktemp)"
trap 'rm -f "${rendered_manifest}"' EXIT
sed \
  -e "s|image: ghcr.io/beranekio/mock-vllm:latest|image: ${MOCK_VLLM_IMAGE}|g" \
  "${MANIFEST}" >"${rendered_manifest}"
kubectl apply -n "${NAMESPACE}" -f "${rendered_manifest}"

if [[ "${deployment_existed}" == "true" ]]; then
  NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" \
    MOCK_VLLM_DEPLOYMENT_REQUIRED=true \
    "${ROOT_DIR}/scripts/restart-mock-vllm-deployment.sh"
else
  echo "Waiting for initial mock-vllm rollout..."
  kubectl rollout status deployment/mock-vllm -n "${NAMESPACE}" --timeout="${TIMEOUT}"
fi

NAMESPACE="${NAMESPACE}" KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" TIMEOUT="${TIMEOUT}" \
  "${ROOT_DIR}/scripts/wait-for-service-endpoints.sh" mock-vllm

echo "mock-vllm deployment ready."