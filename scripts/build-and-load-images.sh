#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-duihua-local}"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${CLUSTER_NAME}}"
RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
TIMEOUT="${TIMEOUT:-300s}"

BACKGROUND_WORKER_IMAGE_REPO="${BACKGROUND_WORKER_IMAGE_REPO:-duihua-background-worker}"
BACKGROUND_WORKER_IMAGE_TAG="${BACKGROUND_WORKER_IMAGE_TAG:-kind}"
BACKGROUND_WORKER_IMAGE="${BACKGROUND_WORKER_IMAGE_REPO}:${BACKGROUND_WORKER_IMAGE_TAG}"
BACKGROUND_WORKER_SRC="${BACKGROUND_WORKER_SRC:-$ROOT_DIR/../duihua-background-worker}"
BACKGROUND_WORKER_GHCR_IMAGE="${BACKGROUND_WORKER_GHCR_IMAGE:-ghcr.io/beranekio/duihua-background-worker:05959c81646bf67e39cc14b320f37f2ece5f1aaf}"

if [[ -f "${BACKGROUND_WORKER_SRC}/Dockerfile" ]]; then
  echo "Building background worker image '${BACKGROUND_WORKER_IMAGE}' from ${BACKGROUND_WORKER_SRC}..."
  docker build -t "${BACKGROUND_WORKER_IMAGE}" "${BACKGROUND_WORKER_SRC}"
else
  echo "Building background worker image '${BACKGROUND_WORKER_IMAGE}' from published GHCR image..."
  docker pull "${BACKGROUND_WORKER_GHCR_IMAGE}"
  docker tag "${BACKGROUND_WORKER_GHCR_IMAGE}" "${BACKGROUND_WORKER_IMAGE}"
fi

echo "Loading image '${BACKGROUND_WORKER_IMAGE}' into kind cluster '${CLUSTER_NAME}'..."
kind load docker-image "${BACKGROUND_WORKER_IMAGE}" --name "${CLUSTER_NAME}"

RELEASE_NAME="${RELEASE_NAME}" NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" \
  KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" \
  "${ROOT_DIR}/scripts/restart-background-worker-deployment.sh"