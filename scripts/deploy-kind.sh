#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-duihua-local}"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${CLUSTER_NAME}}"
RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
CHART_PATH="${CHART_PATH:-$ROOT_DIR/charts/duihua-ai-services}"
VALUES_FILE="${VALUES_FILE:-$ROOT_DIR/charts/duihua-ai-services/values-kind.yaml}"
GATEWAY_IMAGE_REPO="${GATEWAY_IMAGE_REPO:-duihua-gateway}"
GATEWAY_IMAGE_TAG="${GATEWAY_IMAGE_TAG:-local}"
INFERENCE_ENABLED="${INFERENCE_ENABLED:-true}"
TIMEOUT="${TIMEOUT:-300s}"

kubectl() {
  if [[ -n "${KUBECTL_CONTEXT}" ]]; then
    command kubectl --context "${KUBECTL_CONTEXT}" "$@"
  else
    command kubectl "$@"
  fi
}

echo "Deploying Helm release '${RELEASE_NAME}' into namespace '${NAMESPACE}'..."
helm upgrade --install "${RELEASE_NAME}" "${CHART_PATH}" \
  --namespace "${NAMESPACE}" \
  --create-namespace \
  -f "${VALUES_FILE}" \
  --set gateway.image.repository="${GATEWAY_IMAGE_REPO}" \
  --set gateway.image.tag="${GATEWAY_IMAGE_TAG}" \
  --set inference.enabled="${INFERENCE_ENABLED}"

RELEASE_NAME="${RELEASE_NAME}" NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" \
  KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" \
  "${ROOT_DIR}/scripts/restart-gateway-deployment.sh"

if [[ "${INFERENCE_ENABLED}" == "true" ]]; then
  echo "Checking rollout status for inference deployments..."
  inference_deployments="$(kubectl get deployment -n "${NAMESPACE}" -o name \
    | grep "^deployment.apps/${RELEASE_NAME}-duihua-ai-services-inference-" || true)"

  if [[ -z "${inference_deployments}" ]]; then
    echo "No inference deployments found for release '${RELEASE_NAME}' in namespace '${NAMESPACE}'." >&2
    exit 1
  fi

  while read -r deployment; do
    kubectl rollout status "${deployment}" -n "${NAMESPACE}" --timeout="${TIMEOUT}"
  done <<< "${inference_deployments}"
fi

echo "Deployment checks complete."
