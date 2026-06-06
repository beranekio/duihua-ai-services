#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-duihua-local}"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${CLUSTER_NAME}}"
RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
CHART_PATH="${CHART_PATH:-$ROOT_DIR/charts/duihua-ai-services}"
VALUES_FILE="${VALUES_FILE:-$ROOT_DIR/charts/duihua-ai-services/values-kind.yaml}"
EXTRA_VALUES_FILE="${EXTRA_VALUES_FILE:-}"
GATEWAY_IMAGE_REPO="${GATEWAY_IMAGE_REPO:-duihua-gateway}"
GATEWAY_IMAGE_TAG="${GATEWAY_IMAGE_TAG:-local}"
BACKGROUND_WORKER_IMAGE_REPO="${BACKGROUND_WORKER_IMAGE_REPO:-duihua-background-worker}"
BACKGROUND_WORKER_IMAGE_TAG="${BACKGROUND_WORKER_IMAGE_TAG:-${GATEWAY_IMAGE_TAG}}"
INFERENCE_ENABLED="${INFERENCE_ENABLED:-true}"
TIMEOUT="${TIMEOUT:-300s}"

helm_values_args=(-f "${VALUES_FILE}")
if [[ -n "${EXTRA_VALUES_FILE}" ]]; then
  helm_values_args+=(-f "${EXTRA_VALUES_FILE}")
fi

kubectl() {
  if [[ -n "${KUBECTL_CONTEXT}" ]]; then
    command kubectl --context "${KUBECTL_CONTEXT}" "$@"
  else
    command kubectl "$@"
  fi
}

echo "Deploying Helm release '${RELEASE_NAME}' into namespace '${NAMESPACE}'..."
helm upgrade --install "${RELEASE_NAME}" "${CHART_PATH}" \
  --kube-context "${KUBECTL_CONTEXT}" \
  --namespace "${NAMESPACE}" \
  --create-namespace \
  "${helm_values_args[@]}" \
  --set gateway.image.repository="${GATEWAY_IMAGE_REPO}" \
  --set gateway.image.tag="${GATEWAY_IMAGE_TAG}" \
  --set backgroundWorker.image.repository="${BACKGROUND_WORKER_IMAGE_REPO}" \
  --set backgroundWorker.image.tag="${BACKGROUND_WORKER_IMAGE_TAG}" \
  --set inference.enabled="${INFERENCE_ENABLED}"

RELEASE_NAME="${RELEASE_NAME}" NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" \
  KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" GATEWAY_DEPLOYMENT_REQUIRED=true \
  "${ROOT_DIR}/scripts/restart-gateway-deployment.sh"

RELEASE_NAME="${RELEASE_NAME}" NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" \
  KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" \
  "${ROOT_DIR}/scripts/restart-background-worker-deployment.sh"

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
