#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${CLUSTER_NAME:-duihua-local}"
KIND_CONFIG="${KIND_CONFIG:-}"

if ! command -v kind >/dev/null 2>&1; then
  echo "kind is required but not installed." >&2
  exit 1
fi

if kind get clusters | grep -Fxq "${CLUSTER_NAME}"; then
  echo "kind cluster '${CLUSTER_NAME}' already exists."
  exit 0
fi

if [[ -n "${KIND_CONFIG}" ]]; then
  echo "Creating kind cluster '${CLUSTER_NAME}' with config '${KIND_CONFIG}'..."
  kind create cluster --name "${CLUSTER_NAME}" --config "${KIND_CONFIG}"
else
  echo "Creating kind cluster '${CLUSTER_NAME}'..."
  kind create cluster --name "${CLUSTER_NAME}"
fi
