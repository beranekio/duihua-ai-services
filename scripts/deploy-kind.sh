#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER_NAME="${CLUSTER_NAME:-duihua-local}"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${CLUSTER_NAME}}"
RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
CHART_PATH="${CHART_PATH:-$ROOT_DIR/charts/duihua-ai-services}"
VALUES_FILE="${VALUES_FILE:-$ROOT_DIR/charts/duihua-ai-services/values-kind.yaml}"
EXTRA_VALUES_FILE="${EXTRA_VALUES_FILE:-}"
BACKGROUND_WORKER_IMAGE_REPO="${BACKGROUND_WORKER_IMAGE_REPO:-duihua-background-worker}"
BACKGROUND_WORKER_IMAGE_TAG="${BACKGROUND_WORKER_IMAGE_TAG:-local}"
INFERENCE_ENABLED="${INFERENCE_ENABLED:-true}"
TIMEOUT="${TIMEOUT:-300s}"

helm_values_args=(-f "${VALUES_FILE}")
if [[ -n "${EXTRA_VALUES_FILE}" ]]; then
  helm_values_args+=(-f "${EXTRA_VALUES_FILE}")
fi

helm_set_args=(
  --set "backgroundWorker.image.repository=${BACKGROUND_WORKER_IMAGE_REPO}"
  --set "backgroundWorker.image.tag=${BACKGROUND_WORKER_IMAGE_TAG}"
  --set "inference.enabled=${INFERENCE_ENABLED}"
)

GATEWAY_ENV_VALUES_FILE=""
cleanup_gateway_env_values() {
  if [[ -n "${GATEWAY_ENV_VALUES_FILE}" && -f "${GATEWAY_ENV_VALUES_FILE}" ]]; then
    rm -f "${GATEWAY_ENV_VALUES_FILE}"
  fi
}
trap cleanup_gateway_env_values EXIT

# shellcheck source=scripts/_deploy-kind-probe.sh
source "${ROOT_DIR}/scripts/_deploy-kind-probe.sh"

yaml_double_quote() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  printf '%s' "${value}"
}

write_gateway_model_upstreams_file() {
  local file="$1"
  local upstreams="$2"
  local quoted
  quoted="$(yaml_double_quote "${upstreams}")"
  cat > "${file}" <<EOF
duihua-gateway:
  env:
    modelUpstreams: "${quoted}"
EOF
}

kubectl() {
  if [[ -n "${KUBECTL_CONTEXT}" ]]; then
    command kubectl --context "${KUBECTL_CONTEXT}" "$@"
  else
    command kubectl "$@"
  fi
}

"${ROOT_DIR}/scripts/update-helm-dependencies.sh"

if [[ -v GATEWAY_STORE_ENDPOINT ]]; then
  helm_set_args+=(--set "duihua-gateway.responsesApiStore.endpoint=${GATEWAY_STORE_ENDPOINT}")
else
  configured_store_endpoint="$(probe_data_read configuredStoreEndpoint)"
  if [[ -z "${configured_store_endpoint}" && "$(probe_data_read responsesApiStoreServiceEnabled)" == "true" ]]; then
    helm_set_args+=(
      --set "duihua-gateway.responsesApiStore.endpoint=http://${RELEASE_NAME}-responses-api-store:50051"
    )
  fi
fi

PARENT_FULLNAME="$(probe_data duihuaFullname)"
GATEWAY_FULLNAME="$(probe_data gatewayFullname)"
if [[ -z "${PARENT_FULLNAME}" || -z "${GATEWAY_FULLNAME}" ]]; then
  echo "Failed to render deploy-kind probe values from Helm." >&2
  exit 1
fi

if [[ "$(probe_data parentServiceAccountCreate)" == "true" ]]; then
  helm_set_args+=(
    --set "duihua-gateway.serviceAccount.create=false"
    --set "duihua-gateway.serviceAccount.name=$(probe_data serviceAccountName)"
  )
fi

GATEWAY_ENV_VALUES_FILE="$(mktemp)"
if [[ "${INFERENCE_ENABLED}" == "true" ]]; then
  MODEL_UPSTREAMS="$(probe_data modelUpstreams)"
  if [[ -z "${MODEL_UPSTREAMS}" ]]; then
    echo "Failed to compute duihua-gateway.env.modelUpstreams for inference.enabled=true" >&2
    exit 1
  fi
  write_gateway_model_upstreams_file "${GATEWAY_ENV_VALUES_FILE}" "${MODEL_UPSTREAMS}"
else
  write_gateway_model_upstreams_file "${GATEWAY_ENV_VALUES_FILE}" ""
fi
helm_values_args+=(-f "${GATEWAY_ENV_VALUES_FILE}")

echo "Deploying Helm release '${RELEASE_NAME}' into namespace '${NAMESPACE}'..."
helm upgrade --install "${RELEASE_NAME}" "${CHART_PATH}" \
  --kube-context "${KUBECTL_CONTEXT}" \
  --namespace "${NAMESPACE}" \
  --create-namespace \
  "${helm_values_args[@]}" \
  "${helm_set_args[@]}"

RELEASE_NAME="${RELEASE_NAME}" NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" \
  KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" \
  "${ROOT_DIR}/scripts/wait-for-responses-api-store.sh"

RELEASE_NAME="${RELEASE_NAME}" NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" \
  KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" GATEWAY_DEPLOYMENT_REQUIRED=true \
  GATEWAY_DEPLOYMENT="${GATEWAY_FULLNAME}" \
  "${ROOT_DIR}/scripts/restart-gateway-deployment.sh"

RELEASE_NAME="${RELEASE_NAME}" NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" \
  KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" \
  "${ROOT_DIR}/scripts/restart-background-worker-deployment.sh"

RELEASE_NAME="${RELEASE_NAME}" NAMESPACE="${NAMESPACE}" TIMEOUT="${TIMEOUT}" \
  KUBECTL_CONTEXT="${KUBECTL_CONTEXT}" \
  "${ROOT_DIR}/scripts/wait-for-background-worker-ready.sh"

if [[ "${INFERENCE_ENABLED}" == "true" ]]; then
  echo "Checking rollout status for inference deployments..."
  inference_deployments="$(kubectl get deployment -n "${NAMESPACE}" -o name \
    | grep "^deployment.apps/${PARENT_FULLNAME}-inference-" || true)"

  if [[ -z "${inference_deployments}" ]]; then
    echo "No inference deployments found for release '${RELEASE_NAME}' in namespace '${NAMESPACE}'." >&2
    exit 1
  fi

  while read -r deployment; do
    kubectl rollout status "${deployment}" -n "${NAMESPACE}" --timeout="${TIMEOUT}"
  done <<< "${inference_deployments}"
fi

echo "Deployment checks complete."