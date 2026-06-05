#!/usr/bin/env bash
set -euo pipefail

NAMESPACE="${NAMESPACE:-duihua}"
MOCK_VLLM_SERVICE="${MOCK_VLLM_SERVICE:-mock-vllm}"
PROBE_JOB="mock-vllm-upstream-probe-$$"

kubectl() {
  if [[ -n "${KUBECTL_CONTEXT:-}" ]]; then
    command kubectl --context "${KUBECTL_CONTEXT}" "$@"
  else
    command kubectl "$@"
  fi
}

echo "Probing http://${MOCK_VLLM_SERVICE}:8000/health from inside the cluster..."
kubectl run "${PROBE_JOB}" -n "${NAMESPACE}" \
  --rm -i --restart=Never --wait=condition=Complete --timeout=90s \
  --image=curlimages/curl:8.12.1 \
  --command -- \
  curl -sf --max-time 10 "http://${MOCK_VLLM_SERVICE}:8000/health"

echo "mock-vllm upstream probe OK"