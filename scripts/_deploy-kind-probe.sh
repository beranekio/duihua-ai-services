# Shared Helm probe helpers for scripts/deploy-kind.sh and scripts/kind.sh ci.
# Requires: RELEASE_NAME, CHART_PATH, INFERENCE_ENABLED, and helm_values_args (bash array).
# Optional: helm_set_args (bash array) for deploy-time overrides during the full probe.

render_deploy_kind_probe_read() {
  helm template "${RELEASE_NAME}" "${CHART_PATH}" \
    "${helm_values_args[@]}" \
    --set "configValidation.enabled=false" \
    --set "deployKindProbe.enabled=true" \
    --set "inference.enabled=${INFERENCE_ENABLED}" \
    --set "duihua-gateway.responsesApiStore.enabled=false" \
    --set "duihua-background-worker.enabled=false" \
    --show-only templates/deploy-kind-probe.yaml
}

render_deploy_kind_probe() {
  local -a set_args=()
  if [[ -n "${helm_set_args+set}" ]]; then
    set_args=("${helm_set_args[@]}")
  fi
  helm template "${RELEASE_NAME}" "${CHART_PATH}" \
    "${helm_values_args[@]}" \
    "${set_args[@]}" \
    --set "configValidation.enabled=false" \
    --set "deployKindProbe.enabled=true" \
    --set "inference.enabled=${INFERENCE_ENABLED}" \
    --show-only templates/deploy-kind-probe.yaml
}

_probe_data_from_render() {
  local render_fn="$1"
  local key="$2"
  "${render_fn}" | awk -v key="${key}" '
    $0 ~ "^  " key ": " {
      sub("^  " key ": ", "")
      gsub(/^"/, "")
      gsub(/"$/, "")
      print
      exit
    }'
}

probe_data_read() {
  _probe_data_from_render render_deploy_kind_probe_read "$1"
}

probe_data() {
  _probe_data_from_render render_deploy_kind_probe "$1"
}