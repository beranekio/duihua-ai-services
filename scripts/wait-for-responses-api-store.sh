#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-300s}"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-}"

kubectl() {
  if [[ -n "${KUBECTL_CONTEXT}" ]]; then
    command kubectl --context "${KUBECTL_CONTEXT}" "$@"
  else
    command kubectl "$@"
  fi
}

store_deployment="${RELEASE_NAME}-responses-api-store"
valkey_deployment="${RELEASE_NAME}-responses-api-store-valkey"

if ! kubectl get deployment "${store_deployment}" -n "${NAMESPACE}" >/dev/null 2>&1; then
  echo "responses-api-store deployment not present; skipping readiness wait"
  exit 0
fi

echo "Waiting for responses-api-store deployment '${store_deployment}'..."
kubectl wait --for=condition=available "deployment/${store_deployment}" \
  -n "${NAMESPACE}" --timeout="${TIMEOUT}"

if kubectl get deployment "${valkey_deployment}" -n "${NAMESPACE}" >/dev/null 2>&1; then
  echo "Waiting for bundled Valkey deployment '${valkey_deployment}'..."
  kubectl wait --for=condition=available "deployment/${valkey_deployment}" \
    -n "${NAMESPACE}" --timeout="${TIMEOUT}"
fi

echo "responses-api-store dependencies are available."