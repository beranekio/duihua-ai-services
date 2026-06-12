#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/_rollout_helpers.sh
source "${ROOT_DIR}/scripts/_rollout_helpers.sh"

RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-300s}"
BACKGROUND_WORKER_DEPLOYMENT_REQUIRED="${BACKGROUND_WORKER_DEPLOYMENT_REQUIRED:-false}"
BACKGROUND_WORKER_DEPLOYMENT="${BACKGROUND_WORKER_DEPLOYMENT:-${RELEASE_NAME}-duihua-background-worker}"

if ! type -P kubectl >/dev/null 2>&1; then
  if [[ "${BACKGROUND_WORKER_DEPLOYMENT_REQUIRED}" == "true" ]]; then
    echo "kubectl command not found; cannot verify background worker deployment." >&2
    exit 1
  fi
  echo "kubectl command not found; skipping background worker rollout restart."
  exit 0
fi

kubectl() {
  if [[ -n "${KUBECTL_CONTEXT:-}" ]]; then
    command kubectl --context "${KUBECTL_CONTEXT}" "$@"
  else
    command kubectl "$@"
  fi
}

lookup_output=""
lookup_status=0
lookup_output="$(kubectl get deployment "${BACKGROUND_WORKER_DEPLOYMENT}" -n "${NAMESPACE}" 2>&1)" || lookup_status=$?

if [[ "${lookup_status}" -ne 0 ]]; then
  if [[ "${lookup_output}" == *"(NotFound)"* ]]; then
    if [[ "${BACKGROUND_WORKER_DEPLOYMENT_REQUIRED}" == "true" ]]; then
      echo "Background worker deployment '${BACKGROUND_WORKER_DEPLOYMENT}' not found in namespace '${NAMESPACE}' after deploy." >&2
      exit 1
    fi
    echo "Background worker deployment '${BACKGROUND_WORKER_DEPLOYMENT}' not found in namespace '${NAMESPACE}'; skipping rollout restart."
    exit 0
  fi
  echo "${lookup_output}" >&2
  exit 1
fi

if [[ "${BACKGROUND_WORKER_ROLLOUT_RESTART:-true}" == "true" ]]; then
  echo "Restarting background worker deployment '${BACKGROUND_WORKER_DEPLOYMENT}' to pick up the current image..."
  kubectl rollout restart deployment/"${BACKGROUND_WORKER_DEPLOYMENT}" -n "${NAMESPACE}"
else
  echo "Skipping background worker rollout restart (BACKGROUND_WORKER_ROLLOUT_RESTART=${BACKGROUND_WORKER_ROLLOUT_RESTART})."
fi

echo "Checking rollout status for background worker deployment '${BACKGROUND_WORKER_DEPLOYMENT}'..."
rollout_status_with_diagnostics \
  "${BACKGROUND_WORKER_DEPLOYMENT}" \
  "${NAMESPACE}" \
  "${TIMEOUT}" \
  "${BACKGROUND_WORKER_ROLLOUT_STRICT:-${ROLLOUT_STRICT:-true}}"