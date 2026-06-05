#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-duihua-local}"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${CLUSTER_NAME}}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-120s}"
MOCK_VLLM_IMAGE_REPO="${MOCK_VLLM_IMAGE_REPO:-duihua-mock-vllm}"
MOCK_VLLM_IMAGE_TAG="${MOCK_VLLM_IMAGE_TAG:-local}"
MOCK_VLLM_IMAGE="${MOCK_VLLM_IMAGE_REPO}:${MOCK_VLLM_IMAGE_TAG}"

echo "Building mock-vllm image '${MOCK_VLLM_IMAGE}'..."
docker build -t "${MOCK_VLLM_IMAGE}" "${ROOT_DIR}/services/mock-vllm"

echo "Loading image '${MOCK_VLLM_IMAGE}' into kind cluster '${CLUSTER_NAME}'..."
kind load docker-image "${MOCK_VLLM_IMAGE}" --name "${CLUSTER_NAME}"

NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" \
  "${ROOT_DIR}/scripts/restart-mock-vllm-deployment.sh"