#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/_common.sh
source "${ROOT_DIR}/scripts/_common.sh"
# shellcheck source=scripts/_kind-bootstrap.sh
source "${ROOT_DIR}/scripts/_kind-bootstrap.sh"

usage() {
  cat <<'EOF'
Usage: scripts/kind.sh <command>

Commands:
  bootstrap   Create the kind cluster (if missing) and install KEDA
  up          bootstrap + deploy the Helm chart (local first-time setup)
  deploy      Upgrade/install the chart on an existing kind cluster
  smoke       Run gateway smoke tests against the deployed release
  ci          CI integration path: KEDA, mock-vllm, deploy, smoke

Environment variables match the legacy per-script defaults documented in README.md.
EOF
}

run_step() {
  echo
  echo "==> $*"
  echo
}

cmd_bootstrap() {
  create_kind_cluster
  install_keda
}

cmd_up() {
  cmd_bootstrap
  cmd_deploy
}

cmd_deploy() {
  "${ROOT_DIR}/scripts/deploy-kind.sh"
}

cmd_smoke() {
  "${ROOT_DIR}/scripts/smoke-test-kind.sh"
}

cmd_ci() {
  export CLUSTER_NAME="${CLUSTER_NAME:-duihua-ci}"
  export KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-kind-${CLUSTER_NAME}}"
  export RELEASE_NAME="${RELEASE_NAME:-duihua}"
  export NAMESPACE="${NAMESPACE:-duihua}"
  export MOCK_VLLM_IMAGE="${MOCK_VLLM_IMAGE:-ghcr.io/beranekio/mock-vllm:latest}"
  export VALUES_FILE="${VALUES_FILE:-${ROOT_DIR}/charts/duihua-ai-services/values-kind.yaml}"
  export EXTRA_VALUES_FILE="${EXTRA_VALUES_FILE:-${ROOT_DIR}/charts/duihua-ai-services/values-kind-ci.yaml}"
  export INFERENCE_ENABLED="${INFERENCE_ENABLED:-false}"

  run_step "Installing KEDA"
  install_keda

  run_step "Deploying mock-vllm upstream (before gateway)"
  "${ROOT_DIR}/scripts/deploy-mock-vllm-kind.sh"

  run_step "Verifying mock-vllm is reachable in-cluster"
  verify_mock_vllm_upstream

  run_step "Deploying Helm chart (CI values overlay, inference disabled)"
  cmd_deploy

  CHART_PATH="${ROOT_DIR}/charts/duihua-ai-services"
  local -a helm_values_args=(-f "${VALUES_FILE}")
  if [[ -n "${EXTRA_VALUES_FILE}" ]]; then
    helm_values_args+=(-f "${EXTRA_VALUES_FILE}")
  fi
  # shellcheck source=scripts/_deploy-kind-probe.sh
  source "${ROOT_DIR}/scripts/_deploy-kind-probe.sh"
  local gateway_deployment
  gateway_deployment="$(probe_data_read gatewayFullname)"
  if [[ -z "${gateway_deployment}" ]]; then
    echo "Failed to resolve gateway Deployment name from Helm probe." >&2
    exit 1
  fi
  echo "Gateway upstream configuration:"
  kubectl get deployment "${gateway_deployment}" -n "${NAMESPACE}" \
    -o jsonpath='{range .spec.template.spec.containers[0].env[*]}{.name}={.value}{"\n"}{end}' \
    | grep -E '^UPSTREAM_BASE_URL=|^MODEL_UPSTREAMS='

  run_step "Running gateway smoke tests"
  cmd_smoke

  echo
  echo "Kind integration smoke tests passed."
}

main() {
  local command="${1:-}"
  case "${command}" in
    bootstrap) cmd_bootstrap ;;
    up) cmd_up ;;
    deploy) cmd_deploy ;;
    smoke) cmd_smoke ;;
    ci) cmd_ci ;;
    -h | --help | help | "") usage ;;
    *)
      echo "Unknown command: ${command}" >&2
      usage >&2
      exit 1
      ;;
  esac
}

main "$@"