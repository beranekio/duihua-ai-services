#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-duihua-local}"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${CLUSTER_NAME}}"
RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-300s}"
GATEWAY_IMAGE_REPO="${GATEWAY_IMAGE_REPO:-duihua-gateway}"
GATEWAY_IMAGE_TAG="${GATEWAY_IMAGE_TAG:-local}"
GATEWAY_IMAGE="${GATEWAY_IMAGE_REPO}:${GATEWAY_IMAGE_TAG}"

echo "Building gateway image '${GATEWAY_IMAGE}'..."
docker build -t "${GATEWAY_IMAGE}" services/gateway

echo "Loading image '${GATEWAY_IMAGE}' into kind cluster '${CLUSTER_NAME}'..."
kind load docker-image "${GATEWAY_IMAGE}" --name "${CLUSTER_NAME}"

RELEASE_NAME="${RELEASE_NAME}" NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" \
  KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" \
  "${ROOT_DIR}/scripts/restart-gateway-deployment.sh"
