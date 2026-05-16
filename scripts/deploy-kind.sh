#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-duihua-local}"
RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
IMAGE_REPOSITORY="${IMAGE_REPOSITORY:-duihua-gateway}"
IMAGE_TAG="${IMAGE_TAG:-kind}"
IMAGE="${IMAGE_REPOSITORY}:${IMAGE_TAG}"
KIND_CONFIG="${KIND_CONFIG:-$ROOT_DIR/kind/cluster.yaml}"
VALUES_FILE="${VALUES_FILE:-$ROOT_DIR/charts/duihua-ai-services/values-kind.yaml}"
SKIP_CLUSTER_CREATION="${SKIP_CLUSTER_CREATION:-0}"
SKIP_IMAGE_BUILD="${SKIP_IMAGE_BUILD:-0}"
SKIP_SMOKE_TEST="${SKIP_SMOKE_TEST:-0}"
HELM_ARGS=()

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

for cmd in kind kubectl helm docker; do
  require_command "$cmd"
done

if [[ "$SKIP_SMOKE_TEST" != "1" ]]; then
  require_command curl
fi

if [[ "$SKIP_CLUSTER_CREATION" != "1" ]]; then
  if ! kind get clusters | grep -Fxq "$CLUSTER_NAME"; then
    kind create cluster --name "$CLUSTER_NAME" --config "$KIND_CONFIG"
  fi
fi

if [[ "$SKIP_IMAGE_BUILD" != "1" ]]; then
  docker build -t "$IMAGE" -f "$ROOT_DIR/services/gateway/Dockerfile" "$ROOT_DIR/services/gateway"
fi

kind load docker-image "$IMAGE" --name "$CLUSTER_NAME"

helm upgrade --install "$RELEASE_NAME" "$ROOT_DIR/charts/duihua-ai-services" \
  --namespace "$NAMESPACE" \
  --create-namespace \
  -f "$VALUES_FILE" \
  --set gateway.image.repository="$IMAGE_REPOSITORY" \
  --set gateway.image.tag="$IMAGE_TAG" \
  "${HELM_ARGS[@]}"

kubectl rollout status "deployment/$RELEASE_NAME-duihua-ai-services-gateway" \
  --namespace "$NAMESPACE" \
  --timeout=180s

if kubectl get deployment "$RELEASE_NAME-duihua-ai-services-inference-0" --namespace "$NAMESPACE" >/dev/null 2>&1; then
  kubectl rollout status "deployment/$RELEASE_NAME-duihua-ai-services-inference-0" \
    --namespace "$NAMESPACE" \
    --timeout=600s
fi

if [[ "$SKIP_SMOKE_TEST" != "1" ]]; then
  curl --fail --silent --show-error --retry 30 --retry-connrefused --retry-delay 2 \
    http://127.0.0.1:8080/healthz >/dev/null
  curl --fail --silent --show-error http://127.0.0.1:8080/v1/models >/dev/null
fi

echo "Kind deployment is ready at http://127.0.0.1:8080"
