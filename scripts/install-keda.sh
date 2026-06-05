#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${CLUSTER_NAME:-duihua-local}"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${CLUSTER_NAME}}"
KEDA_NAMESPACE="${KEDA_NAMESPACE:-keda}"
KEDA_CHART_VERSION="${KEDA_CHART_VERSION:-}"
KEDA_HTTP_ADDON_CHART_VERSION="${KEDA_HTTP_ADDON_CHART_VERSION:-}"

kubectl() {
  command kubectl --context "${KUBECTL_CONTEXT}" "$@"
}

helm repo add kedacore https://kedacore.github.io/charts >/dev/null
helm repo update >/dev/null

keda_version_args=()
if [[ -n "${KEDA_CHART_VERSION}" ]]; then
  keda_version_args+=(--version "${KEDA_CHART_VERSION}")
fi

keda_addon_version_args=()
if [[ -n "${KEDA_HTTP_ADDON_CHART_VERSION}" ]]; then
  keda_addon_version_args+=(--version "${KEDA_HTTP_ADDON_CHART_VERSION}")
fi

echo "Installing/upgrading KEDA in namespace '${KEDA_NAMESPACE}' (context '${KUBECTL_CONTEXT}')..."
helm upgrade --install keda kedacore/keda \
  --kube-context "${KUBECTL_CONTEXT}" \
  --namespace "${KEDA_NAMESPACE}" \
  --create-namespace \
  "${keda_version_args[@]}"

echo "Installing/upgrading KEDA HTTP add-on in namespace '${KEDA_NAMESPACE}'..."
helm upgrade --install keda-add-ons-http kedacore/keda-add-ons-http \
  --kube-context "${KUBECTL_CONTEXT}" \
  --namespace "${KEDA_NAMESPACE}" \
  --set interceptor.responseHeaderTimeout=120s \
  "${keda_addon_version_args[@]}"

kubectl rollout status deployment/keda-operator -n "${KEDA_NAMESPACE}" --timeout=180s
kubectl rollout status deployment/keda-add-ons-http-interceptor -n "${KEDA_NAMESPACE}" --timeout=180s
# kubectl rollout status deployment/keda-add-ons-http-operator -n "${KEDA_NAMESPACE}" --timeout=180s