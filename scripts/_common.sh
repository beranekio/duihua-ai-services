# Shared helpers for kind deploy and smoke scripts.
# shellcheck shell=bash

if [[ -z "${ROOT_DIR:-}" ]]; then
  ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fi

kubectl() {
  if [[ -n "${KUBECTL_CONTEXT:-}" ]]; then
    command kubectl --context "${KUBECTL_CONTEXT}" "$@"
  else
    command kubectl "$@"
  fi
}

require_command() {
  local command_name="$1"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "Required command not found: ${command_name}" >&2
    exit 1
  fi
}

wait_for_service_endpoints() {
  local service_name="$1"
  local namespace="${NAMESPACE:-duihua}"
  local timeout="${TIMEOUT:-120s}"
  local poll_interval_seconds="${POLL_INTERVAL_SECONDS:-2}"

  local deadline=$((SECONDS + $(echo "${timeout}" | sed -E 's/s$//')))
  echo "Waiting for endpoints on Service/${service_name} in namespace '${namespace}' (timeout ${timeout})..."

  while ((SECONDS < deadline)); do
    local addresses
    addresses="$(kubectl get endpoints "${service_name}" -n "${namespace}" \
      -o jsonpath='{.subsets[*].addresses[*].ip}' 2>/dev/null || true)"
    if [[ -n "${addresses}" ]]; then
      echo "Service/${service_name} has ready endpoints: ${addresses}"
      return 0
    fi
    sleep "${poll_interval_seconds}"
  done

  echo "Service/${service_name} did not gain endpoints within ${timeout}" >&2
  kubectl get endpoints "${service_name}" -n "${namespace}" -o wide >&2 || true
  return 1
}

verify_mock_vllm_upstream() {
  local namespace="${NAMESPACE:-duihua}"
  local mock_vllm_service="${MOCK_VLLM_SERVICE:-mock-vllm}"
  local probe_job="mock-vllm-upstream-probe-$$"
  local timeout="${TIMEOUT:-90s}"

  cleanup() {
    kubectl delete pod "${probe_job}" -n "${namespace}" --ignore-not-found >/dev/null 2>&1 || true
  }
  trap cleanup RETURN

  echo "Probing http://${mock_vllm_service}:8000/health from inside the cluster..."
  kubectl run "${probe_job}" -n "${namespace}" \
    --restart=Never \
    --image=curlimages/curl:8.12.1 \
    --command -- \
    curl -sf --max-time 10 "http://${mock_vllm_service}:8000/health"

  if ! kubectl wait --for=jsonpath='{.status.phase}'=Succeeded "pod/${probe_job}" \
    -n "${namespace}" --timeout="${timeout}"; then
    echo "mock-vllm upstream probe failed; pod status:" >&2
    kubectl get pod "${probe_job}" -n "${namespace}" -o wide >&2 || true
    kubectl logs "${probe_job}" -n "${namespace}" >&2 || true
    return 1
  fi

  kubectl logs "${probe_job}" -n "${namespace}"
  echo "mock-vllm upstream probe OK"
}