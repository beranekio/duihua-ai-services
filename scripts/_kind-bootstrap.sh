# Kind cluster and KEDA bootstrap helpers for scripts/kind.sh.
# shellcheck shell=bash

create_kind_cluster() {
  local cluster_name="${CLUSTER_NAME:-duihua-local}"
  local kind_config="${KIND_CONFIG:-${ROOT_DIR}/kind/cluster.yaml}"

  require_command kind

  if kind get clusters | grep -Fxq "${cluster_name}"; then
    echo "kind cluster '${cluster_name}' already exists."
    return 0
  fi

  if [[ -n "${kind_config}" ]]; then
    echo "Creating kind cluster '${cluster_name}' with config '${kind_config}'..."
    kind create cluster --name "${cluster_name}" --config "${kind_config}"
  else
    echo "Creating kind cluster '${cluster_name}'..."
    kind create cluster --name "${cluster_name}"
  fi
}

install_keda() {
  local cluster_name="${CLUSTER_NAME:-duihua-local}"
  KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${cluster_name}}"
  local keda_namespace="${KEDA_NAMESPACE:-keda}"
  local keda_chart_version="${KEDA_CHART_VERSION:-}"
  local keda_http_addon_chart_version="${KEDA_HTTP_ADDON_CHART_VERSION:-}"
  local keda_rollout_timeout="${KEDA_ROLLOUT_TIMEOUT:-300s}"

  # shellcheck source=scripts/_rollout_helpers.sh
  source "${ROOT_DIR}/scripts/_rollout_helpers.sh"

  helm repo add kedacore https://kedacore.github.io/charts >/dev/null
  helm repo update >/dev/null

  local -a keda_version_args=()
  if [[ -n "${keda_chart_version}" ]]; then
    keda_version_args+=(--version "${keda_chart_version}")
  fi

  local -a keda_addon_version_args=()
  if [[ -n "${keda_http_addon_chart_version}" ]]; then
    keda_addon_version_args+=(--version "${keda_http_addon_chart_version}")
  fi

  echo "Installing/upgrading KEDA in namespace '${keda_namespace}' (context '${KUBECTL_CONTEXT}')..."
  helm upgrade --install keda kedacore/keda \
    --kube-context "${KUBECTL_CONTEXT}" \
    --namespace "${keda_namespace}" \
    --create-namespace \
    "${keda_version_args[@]}"

  echo "Installing/upgrading KEDA HTTP add-on in namespace '${keda_namespace}'..."
  helm upgrade --install keda-add-ons-http kedacore/keda-add-ons-http \
    --kube-context "${KUBECTL_CONTEXT}" \
    --namespace "${keda_namespace}" \
    --set interceptor.responseHeaderTimeout=120s \
    "${keda_addon_version_args[@]}"

  rollout_status_with_diagnostics keda-operator "${keda_namespace}" "${keda_rollout_timeout}"
  rollout_status_with_diagnostics keda-add-ons-http-interceptor "${keda_namespace}" "${keda_rollout_timeout}"
}