#!/usr/bin/env bash

rollout_status_with_diagnostics() {
  local deployment="$1"
  local namespace="$2"
  local timeout="$3"
  local strict="${4:-true}"

  if kubectl rollout status "deployment/${deployment}" -n "${namespace}" --timeout="${timeout}"; then
    return 0
  fi

  echo "Rollout for deployment/${deployment} did not finish within ${timeout}." >&2
  kubectl get deployment "${deployment}" -n "${namespace}" -o wide >&2 || true
  kubectl describe deployment "${deployment}" -n "${namespace}" 2>&1 | tail -40 >&2 || true

  local available ready desired
  available="$(kubectl get deployment "${deployment}" -n "${namespace}" \
    -o jsonpath='{.status.conditions[?(@.type=="Available")].status}' 2>/dev/null || echo "")"
  ready="$(kubectl get deployment "${deployment}" -n "${namespace}" \
    -o jsonpath='{.status.readyReplicas}' 2>/dev/null || echo "0")"
  desired="$(kubectl get deployment "${deployment}" -n "${namespace}" \
    -o jsonpath='{.spec.replicas}' 2>/dev/null || echo "0")"

  if [[ "${strict}" != "true" && "${available}" == "True" && "${ready}" =~ ^[1-9] ]]; then
    echo "Deployment ${deployment} is Available with ${ready}/${desired} ready replica(s); continuing." >&2
    return 0
  fi

  return 1
}