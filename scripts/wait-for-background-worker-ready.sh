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

deployment="${RELEASE_NAME}-duihua-ai-services-background-worker"
startup_marker="background worker startup: recommended terminationGracePeriodSeconds="

if ! kubectl get deployment "${deployment}" -n "${NAMESPACE}" >/dev/null 2>&1; then
  echo "background worker deployment not present; skipping readiness wait"
  exit 0
fi

timeout_seconds="${TIMEOUT%s}"
if [[ ! "${timeout_seconds}" =~ ^[0-9]+$ ]]; then
  timeout_seconds=300
fi

echo "Waiting for background worker '${deployment}' to finish startup..."
deadline=$((SECONDS + timeout_seconds))
while (( SECONDS < deadline )); do
  if kubectl logs "deployment/${deployment}" -n "${NAMESPACE}" --tail=50 2>/dev/null \
    | grep -Fq "${startup_marker}"; then
    echo "Background worker finished startup."
    exit 0
  fi
  sleep 2
done

echo "background worker did not finish startup within ${TIMEOUT}" >&2
kubectl logs "deployment/${deployment}" -n "${NAMESPACE}" --tail=100 >&2 || true
exit 1