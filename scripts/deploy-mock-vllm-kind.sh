#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${CLUSTER_NAME:-duihua-local}}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-120s}"
MOCK_VLLM_IMAGE_REPO="${MOCK_VLLM_IMAGE_REPO:-duihua-mock-vllm}"
MOCK_VLLM_IMAGE_TAG="${MOCK_VLLM_IMAGE_TAG:-local}"
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

rendered_manifest="$(mktemp)"
sed \
  -e "s|image: duihua-mock-vllm:local|image: ${MOCK_VLLM_IMAGE_REPO}:${MOCK_VLLM_IMAGE_TAG}|g" \
  "${MANIFEST}" >"${rendered_manifest}"
kubectl apply -f "${rendered_manifest}"
rm -f "${rendered_manifest}"

echo "Waiting for mock-vllm rollout..."
kubectl rollout status deployment/mock-vllm -n "${NAMESPACE}" --timeout="${TIMEOUT}"

echo "mock-vllm deployment ready."