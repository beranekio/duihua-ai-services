#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-duihua-local}"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${CLUSTER_NAME}}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-120s}"
MOCK_VLLM_IMAGE="${MOCK_VLLM_IMAGE:-ghcr.io/beranekio/mock-vllm:latest}"

echo "Pulling mock-vllm image '${MOCK_VLLM_IMAGE}'..."
if ! docker pull "${MOCK_VLLM_IMAGE}"; then
  if docker image inspect "${MOCK_VLLM_IMAGE}" >/dev/null 2>&1; then
    echo "Warning: Failed to pull '${MOCK_VLLM_IMAGE}', using existing local image."
  else
    echo "Error: Failed to pull '${MOCK_VLLM_IMAGE}' and no local image found." >&2
    exit 1
  fi
fi

echo "Loading image '${MOCK_VLLM_IMAGE}' into kind cluster '${CLUSTER_NAME}'..."
kind load docker-image "${MOCK_VLLM_IMAGE}" --name "${CLUSTER_NAME}"

NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" \
  "${ROOT_DIR}/scripts/restart-mock-vllm-deployment.sh"