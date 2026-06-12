#!/usr/bin/env bash
# End-to-end kind CI path: KEDA, mock-vllm, Helm deploy, smoke tests.
# Used by .github/workflows/kind-integration.yml. Requires an existing kind cluster.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

export CLUSTER_NAME="${CLUSTER_NAME:-duihua-ci}"
export KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${CLUSTER_NAME}}"
export RELEASE_NAME="${RELEASE_NAME:-duihua}"
export NAMESPACE="${NAMESPACE:-duihua}"

export MOCK_VLLM_IMAGE="${MOCK_VLLM_IMAGE:-ghcr.io/beranekio/mock-vllm:latest}"

export VALUES_FILE="${VALUES_FILE:-${ROOT_DIR}/charts/duihua-ai-services/values-kind.yaml}"
export EXTRA_VALUES_FILE="${EXTRA_VALUES_FILE:-${ROOT_DIR}/charts/duihua-ai-services/values-kind-ci.yaml}"
export INFERENCE_ENABLED="${INFERENCE_ENABLED:-false}"

run_step() {
  echo
  echo "==> $*"
  echo
}

run_step "Installing KEDA"
"${ROOT_DIR}/scripts/install-keda.sh"

run_step "Deploying mock-vllm upstream (before gateway)"
"${ROOT_DIR}/scripts/deploy-mock-vllm-kind.sh"

run_step "Verifying mock-vllm is reachable in-cluster"
"${ROOT_DIR}/scripts/verify-mock-vllm-upstream.sh"

run_step "Deploying Helm chart (CI values overlay, inference disabled)"
"${ROOT_DIR}/scripts/deploy-kind.sh"

CHART_PATH="${ROOT_DIR}/charts/duihua-ai-services"
helm_values_args=(-f "${VALUES_FILE}")
if [[ -n "${EXTRA_VALUES_FILE}" ]]; then
  helm_values_args+=(-f "${EXTRA_VALUES_FILE}")
fi
# shellcheck source=scripts/_deploy-kind-probe.sh
source "${ROOT_DIR}/scripts/_deploy-kind-probe.sh"
gateway_deployment="$(probe_data_read gatewayFullname)"
if [[ -z "${gateway_deployment}" ]]; then
  echo "Failed to resolve gateway Deployment name from Helm probe." >&2
  exit 1
fi
echo "Gateway upstream configuration:"
kubectl get deployment "${gateway_deployment}" -n "${NAMESPACE}" \
  -o jsonpath='{range .spec.template.spec.containers[0].env[*]}{.name}={.value}{"\n"}{end}' \
  | grep -E '^UPSTREAM_BASE_URL=|^MODEL_UPSTREAMS='

run_step "Running gateway smoke tests"
"${ROOT_DIR}/scripts/smoke-test-kind.sh"

echo
echo "Kind integration smoke tests passed."