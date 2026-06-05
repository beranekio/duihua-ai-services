#!/usr/bin/env bash
# End-to-end kind CI path: KEDA, gateway + mock-vllm images, Helm deploy, smoke tests.
# Used by .github/workflows/kind-integration.yml. Requires an existing kind cluster.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

export CLUSTER_NAME="${CLUSTER_NAME:-duihua-ci}"
export KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${CLUSTER_NAME}}"
export RELEASE_NAME="${RELEASE_NAME:-duihua}"
export NAMESPACE="${NAMESPACE:-duihua}"

export GATEWAY_IMAGE_TAG="${GATEWAY_IMAGE_TAG:-local}"
export MOCK_VLLM_IMAGE_TAG="${MOCK_VLLM_IMAGE_TAG:-local}"

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

run_step "Building and loading gateway image"
"${ROOT_DIR}/scripts/build-and-load-images.sh"

run_step "Building and loading mock-vllm image"
"${ROOT_DIR}/scripts/build-and-load-mock-vllm.sh"

run_step "Deploying mock-vllm upstream (before gateway)"
"${ROOT_DIR}/scripts/deploy-mock-vllm-kind.sh"

run_step "Verifying mock-vllm is reachable in-cluster"
"${ROOT_DIR}/scripts/verify-mock-vllm-upstream.sh"

run_step "Deploying Helm chart (CI values overlay, inference disabled)"
"${ROOT_DIR}/scripts/deploy-kind.sh"

gateway_deployment="${RELEASE_NAME}-duihua-ai-services-gateway"
echo "Gateway upstream configuration:"
kubectl get deployment "${gateway_deployment}" -n "${NAMESPACE}" \
  -o jsonpath='{range .spec.template.spec.containers[0].env[*]}{.name}={.value}{"\n"}{end}' \
  | grep -E '^UPSTREAM_BASE_URL=|^MODEL_UPSTREAMS='

run_step "Running gateway smoke tests"
"${ROOT_DIR}/scripts/smoke-test-kind.sh"

echo
echo "Kind integration smoke tests passed."