#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/_common.sh
source "${ROOT_DIR}/scripts/_common.sh"

RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-300s}"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-}"

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

timeout_seconds="${TIMEOUT%s}"
if [[ ! "${timeout_seconds}" =~ ^[0-9]+$ ]]; then
  timeout_seconds=300
fi

echo "Waiting for responses-api-store gRPC health probe..."
deadline=$((SECONDS + timeout_seconds))
while (( SECONDS < deadline )); do
  if kubectl exec "deployment/${store_deployment}" -n "${NAMESPACE}" -- \
    /responses-api-store-probe >/dev/null 2>&1; then
    echo "responses-api-store health probe OK."
    break
  fi
  sleep 2
done

if (( SECONDS >= deadline )); then
  echo "responses-api-store health probe did not succeed within ${TIMEOUT}" >&2
  exit 1
fi

echo "responses-api-store dependencies are available."