#!/usr/bin/env bash
set -euo pipefail

NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-120s}"
MOCK_VLLM_DEPLOYMENT="${MOCK_VLLM_DEPLOYMENT:-mock-vllm}"
MOCK_VLLM_DEPLOYMENT_REQUIRED="${MOCK_VLLM_DEPLOYMENT_REQUIRED:-false}"

if ! type -P kubectl >/dev/null 2>&1; then
  if [[ "${MOCK_VLLM_DEPLOYMENT_REQUIRED}" == "true" ]]; then
    echo "kubectl command not found; cannot verify mock-vllm deployment." >&2
    exit 1
  fi
  echo "kubectl command not found; skipping mock-vllm rollout restart."
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
lookup_output="$(kubectl get deployment "${MOCK_VLLM_DEPLOYMENT}" -n "${NAMESPACE}" 2>&1)" || lookup_status=$?

if [[ "${lookup_status}" -ne 0 ]]; then
  if [[ "${lookup_output}" == *"(NotFound)"* ]]; then
    if [[ "${MOCK_VLLM_DEPLOYMENT_REQUIRED}" == "true" ]]; then
      echo "mock-vllm deployment '${MOCK_VLLM_DEPLOYMENT}' not found in namespace '${NAMESPACE}'." >&2
      exit 1
    fi
    echo "mock-vllm deployment '${MOCK_VLLM_DEPLOYMENT}' not found in namespace '${NAMESPACE}'; skipping rollout restart."
    exit 0
  fi
  echo "${lookup_output}" >&2
  exit 1
fi

if [[ "${MOCK_VLLM_ROLLOUT_RESTART:-true}" == "true" ]]; then
  echo "Restarting mock-vllm deployment '${MOCK_VLLM_DEPLOYMENT}' to pick up the current image..."
  kubectl rollout restart deployment/"${MOCK_VLLM_DEPLOYMENT}" -n "${NAMESPACE}"
else
  echo "Skipping mock-vllm rollout restart (MOCK_VLLM_ROLLOUT_RESTART=${MOCK_VLLM_ROLLOUT_RESTART})."
fi

echo "Checking rollout status for mock-vllm deployment '${MOCK_VLLM_DEPLOYMENT}'..."
kubectl rollout status deployment/"${MOCK_VLLM_DEPLOYMENT}" -n "${NAMESPACE}" --timeout="${TIMEOUT}"