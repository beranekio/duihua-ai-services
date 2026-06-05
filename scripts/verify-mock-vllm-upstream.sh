#!/usr/bin/env bash
set -euo pipefail

NAMESPACE="${NAMESPACE:-duihua}"
MOCK_VLLM_SERVICE="${MOCK_VLLM_SERVICE:-mock-vllm}"
PROBE_JOB="mock-vllm-upstream-probe-$$"
TIMEOUT="${TIMEOUT:-90s}"

kubectl() {
  if [[ -n "${KUBECTL_CONTEXT:-}" ]]; then
    command kubectl --context "${KUBECTL_CONTEXT}" "$@"
  else
    command kubectl "$@"
  fi
}

cleanup() {
  kubectl delete pod "${PROBE_JOB}" -n "${NAMESPACE}" --ignore-not-found >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "Probing http://${MOCK_VLLM_SERVICE}:8000/health from inside the cluster..."
kubectl run "${PROBE_JOB}" -n "${NAMESPACE}" \
  --restart=Never \
  --image=curlimages/curl:8.12.1 \
  --command -- \
  curl -sf --max-time 10 "http://${MOCK_VLLM_SERVICE}:8000/health"

if ! kubectl wait --for=jsonpath='{.status.phase}'=Succeeded "pod/${PROBE_JOB}" \
  -n "${NAMESPACE}" --timeout="${TIMEOUT}"; then
  echo "mock-vllm upstream probe failed; pod status:" >&2
  kubectl get pod "${PROBE_JOB}" -n "${NAMESPACE}" -o wide >&2 || true
  kubectl logs "${PROBE_JOB}" -n "${NAMESPACE}" >&2 || true
  exit 1
fi

kubectl logs "${PROBE_JOB}" -n "${NAMESPACE}"
echo "mock-vllm upstream probe OK"