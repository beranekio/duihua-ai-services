#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-300s}"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-}"
DEPLOYMENT="${BACKGROUND_WORKER_DEPLOYMENT:-${RELEASE_NAME}-duihua-background-worker}"
STARTUP_MARKER="background worker startup: recommended terminationGracePeriodSeconds="

kubectl() {
  if [[ -n "${KUBECTL_CONTEXT}" ]]; then
    command kubectl --context "${KUBECTL_CONTEXT}" "$@"
  else
    command kubectl "$@"
  fi
}

if ! kubectl get deployment "${DEPLOYMENT}" -n "${NAMESPACE}" >/dev/null 2>&1; then
  echo "background worker deployment not present; skipping readiness wait"
  exit 0
fi

desired_replicas="$(kubectl get deployment "${DEPLOYMENT}" -n "${NAMESPACE}" \
  -o jsonpath='{.spec.replicas}' 2>/dev/null || true)"
desired_replicas="${desired_replicas:-0}"
if [[ "${desired_replicas}" == "0" ]]; then
  echo "background worker scaled to zero; skipping startup wait"
  exit 0
fi

newest_worker_pod() {
  kubectl get pods -n "${NAMESPACE}" \
    -l "app.kubernetes.io/name=duihua-background-worker,app.kubernetes.io/instance=${RELEASE_NAME}" \
    --field-selector=status.phase=Running \
    --sort-by=.metadata.creationTimestamp \
    -o name 2>/dev/null | tail -1 | sed 's|^pod/||'
}

worker_startup_complete() {
  local pod="$1"
  local logs
  [[ -n "${pod}" ]] || return 1
  logs="$(kubectl logs "${pod}" -n "${NAMESPACE}" 2>/dev/null || true)"
  [[ "${logs}" == *"${STARTUP_MARKER}"* ]]
}

timeout_seconds="${TIMEOUT%s}"
if [[ ! "${timeout_seconds}" =~ ^[0-9]+$ ]]; then
  timeout_seconds=300
fi

echo "Waiting for background worker '${DEPLOYMENT}' to finish startup..."
deadline=$((SECONDS + timeout_seconds))
while (( SECONDS < deadline )); do
  pod="$(newest_worker_pod)"
  if worker_startup_complete "${pod}"; then
    echo "Background worker finished startup."
    exit 0
  fi
  sleep 2
done

echo "background worker did not finish startup within ${TIMEOUT}" >&2
pod="$(newest_worker_pod)"
if [[ -n "${pod}" ]]; then
  kubectl logs "${pod}" -n "${NAMESPACE}" --tail=100 >&2 || true
else
  kubectl get pods -n "${NAMESPACE}" \
    -l "app.kubernetes.io/name=duihua-background-worker,app.kubernetes.io/instance=${RELEASE_NAME}" >&2 || true
fi
exit 1