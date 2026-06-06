#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/_rollout_helpers.sh
source "${ROOT_DIR}/scripts/_rollout_helpers.sh"

RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-300s}"
GATEWAY_DEPLOYMENT_REQUIRED="${GATEWAY_DEPLOYMENT_REQUIRED:-false}"
GATEWAY_DEPLOYMENT="${RELEASE_NAME}-duihua-ai-services-gateway"

if ! type -P kubectl >/dev/null 2>&1; then
  if [[ "${GATEWAY_DEPLOYMENT_REQUIRED}" == "true" ]]; then
    echo "kubectl command not found; cannot verify gateway deployment." >&2
    exit 1
  fi
  echo "kubectl command not found; skipping gateway rollout restart."
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
lookup_output="$(kubectl get deployment "${GATEWAY_DEPLOYMENT}" -n "${NAMESPACE}" 2>&1)" || lookup_status=$?

if [[ "${lookup_status}" -ne 0 ]]; then
  if [[ "${lookup_output}" == *"(NotFound)"* ]]; then
    if [[ "${GATEWAY_DEPLOYMENT_REQUIRED}" == "true" ]]; then
      echo "Gateway deployment '${GATEWAY_DEPLOYMENT}' not found in namespace '${NAMESPACE}' after deploy." >&2
      exit 1
    fi
    echo "Gateway deployment '${GATEWAY_DEPLOYMENT}' not found in namespace '${NAMESPACE}'; skipping rollout restart."
    exit 0
  fi
  echo "${lookup_output}" >&2
  exit 1
fi

if [[ "${GATEWAY_ROLLOUT_RESTART:-true}" == "true" ]]; then
  echo "Restarting gateway deployment '${GATEWAY_DEPLOYMENT}' to pick up the current image..."
  kubectl rollout restart deployment/"${GATEWAY_DEPLOYMENT}" -n "${NAMESPACE}"
else
  echo "Skipping gateway rollout restart (GATEWAY_ROLLOUT_RESTART=${GATEWAY_ROLLOUT_RESTART})."
fi

echo "Checking rollout status for gateway deployment '${GATEWAY_DEPLOYMENT}'..."
rollout_status_with_diagnostics \
  "${GATEWAY_DEPLOYMENT}" \
  "${NAMESPACE}" \
  "${TIMEOUT}" \
  "${GATEWAY_ROLLOUT_STRICT:-${ROLLOUT_STRICT:-true}}"