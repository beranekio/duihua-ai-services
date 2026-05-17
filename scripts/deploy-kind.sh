#!/usr/bin/env bash
set -euo pipefail

RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-default}"
CHART_PATH="${CHART_PATH:-charts/duihua-ai-services}"
GATEWAY_IMAGE_REPO="${GATEWAY_IMAGE_REPO:-duihua-gateway}"
GATEWAY_IMAGE_TAG="${GATEWAY_IMAGE_TAG:-local}"
INFERENCE_ENABLED="${INFERENCE_ENABLED:-false}"
TIMEOUT="${TIMEOUT:-300s}"

echo "Deploying Helm release '${RELEASE_NAME}' into namespace '${NAMESPACE}'..."
helm upgrade --install "${RELEASE_NAME}" "${CHART_PATH}" \
  --namespace "${NAMESPACE}" \
  --create-namespace \
  --set gateway.image.repository="${GATEWAY_IMAGE_REPO}" \
  --set gateway.image.tag="${GATEWAY_IMAGE_TAG}" \
  --set inference.enabled="${INFERENCE_ENABLED}"

echo "Checking rollout status for gateway deployment..."
kubectl rollout status deployment/"${RELEASE_NAME}"-duihua-ai-services-gateway -n "${NAMESPACE}" --timeout="${TIMEOUT}"

if [[ "${INFERENCE_ENABLED}" == "true" ]]; then
  echo "Checking rollout status for inference deployments..."
  kubectl get deployment -n "${NAMESPACE}" -l app.kubernetes.io/instance="${RELEASE_NAME}",app.kubernetes.io/component=inference -o name \
    | while read -r deployment; do
        kubectl rollout status "${deployment}" -n "${NAMESPACE}" --timeout="${TIMEOUT}"
      done
fi

echo "Deployment checks complete."
