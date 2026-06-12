#!/usr/bin/env bash
set -euo pipefail

# Restart a Deployment and wait for rollout. Set COMPONENT to one of:
#   gateway | background-worker | mock-vllm
# Legacy per-component env vars (GATEWAY_DEPLOYMENT, GATEWAY_ROLLOUT_RESTART, etc.) are still honored.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/_common.sh
source "${ROOT_DIR}/scripts/_common.sh"
# shellcheck source=scripts/_rollout_helpers.sh
source "${ROOT_DIR}/scripts/_rollout_helpers.sh"

RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-300s}"
COMPONENT="${COMPONENT:?COMPONENT is required (gateway, background-worker, or mock-vllm)}"

case "${COMPONENT}" in
  gateway)
    deployment_label="gateway"
    deployment="${GATEWAY_DEPLOYMENT:-${RELEASE_NAME}-duihua-gateway}"
    deployment_required="${GATEWAY_DEPLOYMENT_REQUIRED:-false}"
    rollout_restart="${GATEWAY_ROLLOUT_RESTART:-true}"
    rollout_strict="${GATEWAY_ROLLOUT_STRICT:-${ROLLOUT_STRICT:-true}}"
    ;;
  background-worker)
    deployment_label="background worker"
    deployment="${BACKGROUND_WORKER_DEPLOYMENT:-${RELEASE_NAME}-duihua-background-worker}"
    deployment_required="${BACKGROUND_WORKER_DEPLOYMENT_REQUIRED:-false}"
    rollout_restart="${BACKGROUND_WORKER_ROLLOUT_RESTART:-true}"
    rollout_strict="${BACKGROUND_WORKER_ROLLOUT_STRICT:-${ROLLOUT_STRICT:-true}}"
    ;;
  mock-vllm)
    deployment_label="mock-vllm"
    deployment="${MOCK_VLLM_DEPLOYMENT:-mock-vllm}"
    deployment_required="${MOCK_VLLM_DEPLOYMENT_REQUIRED:-false}"
    rollout_restart="${MOCK_VLLM_ROLLOUT_RESTART:-true}"
    rollout_strict="${ROLLOUT_STRICT:-true}"
    ;;
  *)
    echo "Unsupported COMPONENT '${COMPONENT}'." >&2
    exit 1
    ;;
esac

if ! type -P kubectl >/dev/null 2>&1; then
  if [[ "${deployment_required}" == "true" ]]; then
    echo "kubectl command not found; cannot verify ${deployment_label} deployment." >&2
    exit 1
  fi
  echo "kubectl command not found; skipping ${deployment_label} rollout restart."
  exit 0
fi

lookup_output=""
lookup_status=0
lookup_output="$(kubectl get deployment "${deployment}" -n "${NAMESPACE}" 2>&1)" || lookup_status=$?

if [[ "${lookup_status}" -ne 0 ]]; then
  if [[ "${lookup_output}" == *"(NotFound)"* ]]; then
    if [[ "${deployment_required}" == "true" ]]; then
      echo "Deployment '${deployment}' (${deployment_label}) not found in namespace '${NAMESPACE}'." >&2
      exit 1
    fi
    echo "Deployment '${deployment}' (${deployment_label}) not found in namespace '${NAMESPACE}'; skipping rollout restart."
    exit 0
  fi
  echo "${lookup_output}" >&2
  exit 1
fi

if [[ "${rollout_restart}" == "true" ]]; then
  echo "Restarting ${deployment_label} deployment '${deployment}' to pick up the current image..."
  kubectl rollout restart "deployment/${deployment}" -n "${NAMESPACE}"
else
  echo "Skipping ${deployment_label} rollout restart (ROLLOUT_RESTART=${rollout_restart})."
fi

echo "Checking rollout status for ${deployment_label} deployment '${deployment}'..."
rollout_status_with_diagnostics \
  "${deployment}" \
  "${NAMESPACE}" \
  "${TIMEOUT}" \
  "${rollout_strict}"