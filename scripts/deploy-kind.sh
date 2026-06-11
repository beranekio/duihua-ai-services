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
GATEWAY_STORE_ENDPOINT="${GATEWAY_STORE_ENDPOINT:-http://${RELEASE_NAME}-responses-api-store:50051}"
BACKGROUND_WORKER_IMAGE_REPO="${BACKGROUND_WORKER_IMAGE_REPO:-duihua-background-worker}"
BACKGROUND_WORKER_IMAGE_TAG="${BACKGROUND_WORKER_IMAGE_TAG:-local}"
INFERENCE_ENABLED="${INFERENCE_ENABLED:-true}"
TIMEOUT="${TIMEOUT:-300s}"

helm_values_args=(-f "${VALUES_FILE}")
if [[ -n "${EXTRA_VALUES_FILE}" ]]; then
  helm_values_args+=(-f "${EXTRA_VALUES_FILE}")
fi

PARENT_SA_NAME="${RELEASE_NAME}-duihua-ai-services"
helm_set_args=(
  --set "duihua-gateway.responsesApiStore.endpoint=${GATEWAY_STORE_ENDPOINT}"
  --set "duihua-gateway.serviceAccount.create=false"
  --set "duihua-gateway.serviceAccount.name=${PARENT_SA_NAME}"
  --set "backgroundWorker.image.repository=${BACKGROUND_WORKER_IMAGE_REPO}"
  --set "backgroundWorker.image.tag=${BACKGROUND_WORKER_IMAGE_TAG}"
  --set "inference.enabled=${INFERENCE_ENABLED}"
)

kubectl() {
  if [[ -n "${KUBECTL_CONTEXT}" ]]; then
    command kubectl --context "${KUBECTL_CONTEXT}" "$@"
  else
    command kubectl "$@"
  fi
}

"${ROOT_DIR}/scripts/update-helm-dependencies.sh"

if [[ "${INFERENCE_ENABLED}" == "true" ]]; then
  values_files_for_python=("${VALUES_FILE}")
  if [[ -n "${EXTRA_VALUES_FILE}" ]]; then
    values_files_for_python+=("${EXTRA_VALUES_FILE}")
  fi
  MODEL_UPSTREAMS="$(python3 - "${RELEASE_NAME}" "${values_files_for_python[@]}" <<'PY'
import sys
import yaml

release = sys.argv[1]
merged: dict = {}
for path in sys.argv[2:]:
    with open(path, encoding="utf-8") as handle:
        doc = yaml.safe_load(handle) or {}
    for key, value in doc.items():
        if isinstance(value, dict) and isinstance(merged.get(key), dict):
            stack = [(merged[key], value)]
            while stack:
                left, right = stack.pop()
                for nested_key, nested_value in right.items():
                    if (
                        nested_key in left
                        and isinstance(left[nested_key], dict)
                        and isinstance(nested_value, dict)
                    ):
                        stack.append((left[nested_key], nested_value))
                    else:
                        left[nested_key] = nested_value
        else:
            merged[key] = value

models = merged.get("inference", {}).get("models", [])
parent = f"{release}-duihua-ai-services"
parts = [
    f"{model['name']}=http://{parent}-inference-{index}-proxy:8080/v1"
    for index, model in enumerate(models)
]
print(",".join(parts), end="")
PY
)"
  if [[ -z "${MODEL_UPSTREAMS}" ]]; then
    echo "Failed to compute duihua-gateway.env.modelUpstreams for inference.enabled=true" >&2
    exit 1
  fi
  helm_set_args+=(--set "duihua-gateway.env.modelUpstreams=${MODEL_UPSTREAMS}")
fi

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
    | grep "^deployment.apps/${RELEASE_NAME}-duihua-ai-services-inference-" || true)"

  if [[ -z "${inference_deployments}" ]]; then
    echo "No inference deployments found for release '${RELEASE_NAME}' in namespace '${NAMESPACE}'." >&2
    exit 1
  fi

  while read -r deployment; do
    kubectl rollout status "${deployment}" -n "${NAMESPACE}" --timeout="${TIMEOUT}"
  done <<< "${inference_deployments}"
fi

echo "Deployment checks complete."
