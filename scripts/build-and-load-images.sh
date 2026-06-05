#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-duihua-local}"
GATEWAY_IMAGE_REPO="${GATEWAY_IMAGE_REPO:-duihua-gateway}"
GATEWAY_IMAGE_TAG="${GATEWAY_IMAGE_TAG:-local}"
GATEWAY_IMAGE="${GATEWAY_IMAGE_REPO}:${GATEWAY_IMAGE_TAG}"

echo "Building gateway image '${GATEWAY_IMAGE}'..."
docker build -t "${GATEWAY_IMAGE}" services/gateway

echo "Loading image '${GATEWAY_IMAGE}' into kind cluster '${CLUSTER_NAME}'..."
kind load docker-image "${GATEWAY_IMAGE}" --name "${CLUSTER_NAME}"

"${ROOT_DIR}/scripts/restart-gateway-deployment.sh"
