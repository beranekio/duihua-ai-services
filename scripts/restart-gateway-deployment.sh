#!/usr/bin/env bash
set -euo pipefail

RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-300s}"
GATEWAY_DEPLOYMENT="${RELEASE_NAME}-duihua-ai-services-gateway"

if [[ "${GATEWAY_ROLLOUT_RESTART:-true}" != "true" ]]; then
  echo "Skipping gateway rollout restart (GATEWAY_ROLLOUT_RESTART=${GATEWAY_ROLLOUT_RESTART})."
  exit 0
fi

if ! kubectl get deployment "${GATEWAY_DEPLOYMENT}" -n "${NAMESPACE}" &>/dev/null; then
  echo "Gateway deployment '${GATEWAY_DEPLOYMENT}' not found in namespace '${NAMESPACE}'; skipping rollout restart."
  exit 0
fi

echo "Restarting gateway deployment '${GATEWAY_DEPLOYMENT}' to pick up the current image..."
kubectl rollout restart deployment/"${GATEWAY_DEPLOYMENT}" -n "${NAMESPACE}"
kubectl rollout status deployment/"${GATEWAY_DEPLOYMENT}" -n "${NAMESPACE}" --timeout="${TIMEOUT}"