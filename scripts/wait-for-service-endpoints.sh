#!/usr/bin/env bash
set -euo pipefail

SERVICE_NAME="${1:?service name required}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-120s}"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-2}"

kubectl() {
  if [[ -n "${KUBECTL_CONTEXT:-}" ]]; then
    command kubectl --context "${KUBECTL_CONTEXT}" "$@"
  else
    command kubectl "$@"
  fi
}

deadline=$((SECONDS + $(echo "${TIMEOUT}" | sed -E 's/s$//')))
echo "Waiting for endpoints on Service/${SERVICE_NAME} in namespace '${NAMESPACE}' (timeout ${TIMEOUT})..."

while ((SECONDS < deadline)); do
  addresses="$(kubectl get endpoints "${SERVICE_NAME}" -n "${NAMESPACE}" \
    -o jsonpath='{.subsets[*].addresses[*].ip}' 2>/dev/null || true)"
  if [[ -n "${addresses}" ]]; then
    echo "Service/${SERVICE_NAME} has ready endpoints: ${addresses}"
    exit 0
  fi
  sleep "${POLL_INTERVAL_SECONDS}"
done

echo "Service/${SERVICE_NAME} did not gain endpoints within ${TIMEOUT}" >&2
kubectl get endpoints "${SERVICE_NAME}" -n "${NAMESPACE}" -o wide >&2 || true
exit 1