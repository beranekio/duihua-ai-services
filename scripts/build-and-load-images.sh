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
GATEWAY_ROOT="${GATEWAY_ROOT:-${ROOT_DIR}/../duihua-gateway}"
GATEWAY_SOURCE_IMAGE="${GATEWAY_SOURCE_IMAGE:-ghcr.io/beranekio/duihua-gateway:latest}"

BACKGROUND_WORKER_IMAGE_REPO="${BACKGROUND_WORKER_IMAGE_REPO:-duihua-background-worker}"
BACKGROUND_WORKER_IMAGE_TAG="${BACKGROUND_WORKER_IMAGE_TAG:-${GATEWAY_IMAGE_TAG}}"
BACKGROUND_WORKER_IMAGE="${BACKGROUND_WORKER_IMAGE_REPO}:${BACKGROUND_WORKER_IMAGE_TAG}"

if [[ -f "${GATEWAY_ROOT}/Dockerfile" ]]; then
  echo "Building gateway image '${GATEWAY_IMAGE}' from ${GATEWAY_ROOT}..."
  docker build -t "${GATEWAY_IMAGE}" --file "${GATEWAY_ROOT}/Dockerfile" "${GATEWAY_ROOT}"
else
  echo "duihua-gateway source not found at ${GATEWAY_ROOT}; pulling ${GATEWAY_SOURCE_IMAGE}..."
  docker pull "${GATEWAY_SOURCE_IMAGE}"
  docker tag "${GATEWAY_SOURCE_IMAGE}" "${GATEWAY_IMAGE}"
fi

echo "Building background worker image '${BACKGROUND_WORKER_IMAGE}'..."
docker build -t "${BACKGROUND_WORKER_IMAGE}" --file services/background-worker/Dockerfile services

echo "Loading image '${GATEWAY_IMAGE}' into kind cluster '${CLUSTER_NAME}'..."
kind load docker-image "${GATEWAY_IMAGE}" --name "${CLUSTER_NAME}"

echo "Loading image '${BACKGROUND_WORKER_IMAGE}' into kind cluster '${CLUSTER_NAME}'..."
kind load docker-image "${BACKGROUND_WORKER_IMAGE}" --name "${CLUSTER_NAME}"

RELEASE_NAME="${RELEASE_NAME}" NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" \
  KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" \
  "${ROOT_DIR}/scripts/restart-gateway-deployment.sh"

RELEASE_NAME="${RELEASE_NAME}" NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" \
  KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" \
  "${ROOT_DIR}/scripts/restart-background-worker-deployment.sh"