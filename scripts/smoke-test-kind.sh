#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATEWAY_BASE_URL="${GATEWAY_BASE_URL:-http://127.0.0.1:8080}"
DEFAULT_MODEL="${DEFAULT_MODEL:-HuggingFaceTB/SmolLM2-135M-Instruct}"
RELEASE_NAME="${RELEASE_NAME:-duihua}"
NAMESPACE="${NAMESPACE:-duihua}"
KUBECTL_CONTEXT="${KUBECTL_CONTEXT:-}"
HEALTHZ_RETRIES="${HEALTHZ_RETRIES:-30}"
HEALTHZ_INTERVAL_SECONDS="${HEALTHZ_INTERVAL_SECONDS:-5}"
BACKGROUND_POLL_ATTEMPTS="${BACKGROUND_POLL_ATTEMPTS:-45}"
BACKGROUND_POLL_INTERVAL_SECONDS="${BACKGROUND_POLL_INTERVAL_SECONDS:-2}"
CANCEL_POLL_ATTEMPTS="${CANCEL_POLL_ATTEMPTS:-20}"
DELETE_POLL_ATTEMPTS="${DELETE_POLL_ATTEMPTS:-10}"

require_command() {
  local command_name="$1"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "Required command not found: ${command_name}" >&2
    exit 1
  fi
}

kubectl() {
  if [[ -n "${KUBECTL_CONTEXT}" ]]; then
    command kubectl --context "${KUBECTL_CONTEXT}" "$@"
  else
    command kubectl "$@"
  fi
}

json_get() {
  local payload="$1"
  local python_expr="$2"
  python3 -c 'import json, sys; payload = json.loads(sys.argv[1]); print('"${python_expr}"')' "${payload}"
}

wait_for_gateway() {
  echo "Waiting for gateway at ${GATEWAY_BASE_URL}/healthz..."
  for attempt in $(seq 1 "${HEALTHZ_RETRIES}"); do
    if curl -sf "${GATEWAY_BASE_URL}/healthz" >/dev/null; then
      echo "Gateway healthz OK"
      return 0
    fi
    echo "Attempt ${attempt}/${HEALTHZ_RETRIES} failed; retrying in ${HEALTHZ_INTERVAL_SECONDS}s..."
    sleep "${HEALTHZ_INTERVAL_SECONDS}"
  done

  echo "Gateway did not become ready at ${GATEWAY_BASE_URL}" >&2
  exit 1
}

wait_for_background_worker() {
  if ! background_queue_enabled; then
    echo "Background queue disabled; skipping worker readiness wait"
    return 0
  fi
  if ! background_worker_deployed; then
    echo "Background queue enabled without worker Deployment; skipping worker readiness wait"
    return 0
  fi

  local deployment="${RELEASE_NAME}-duihua-ai-services-background-worker"
  echo "Waiting for background worker deployment ${deployment}..."
  local wait_timeout="$((HEALTHZ_RETRIES * HEALTHZ_INTERVAL_SECONDS))"
  if ! kubectl wait --for=condition=available "deployment/${deployment}" \
    -n "${NAMESPACE}" --timeout="${wait_timeout}s"; then
    echo "Background worker deployment did not become available" >&2
    exit 1
  fi
  echo "Background worker deployment available"
}

post_response() {
  local payload="$1"
  curl -sf "${GATEWAY_BASE_URL}/v1/responses" \
    -H 'Content-Type: application/json' \
    -d "${payload}"
}

post_messages() {
  local payload="$1"
  curl -sf "${GATEWAY_BASE_URL}/v1/messages" \
    -H 'Content-Type: application/json' \
    -H 'anthropic-version: 2023-06-01' \
    -H 'x-api-key: dummy' \
    -d "${payload}"
}

post_messages_count_tokens() {
  local payload="$1"
  curl -sf "${GATEWAY_BASE_URL}/v1/messages/count_tokens" \
    -H 'Content-Type: application/json' \
    -H 'anthropic-version: 2023-06-01' \
    -H 'x-api-key: dummy' \
    -d "${payload}"
}

get_response_status() {
  local response_id="$1"
  curl -sf "${GATEWAY_BASE_URL}/v1/responses/${response_id}"
}

http_status() {
  local method="$1"
  local path="$2"
  local payload="${3:-}"
  if [[ -n "${payload}" ]]; then
    curl -s -o /dev/null -w '%{http_code}' -X "${method}" "${GATEWAY_BASE_URL}${path}" \
      -H 'Content-Type: application/json' \
      -d "${payload}"
  else
    curl -s -o /dev/null -w '%{http_code}' -X "${method}" "${GATEWAY_BASE_URL}${path}"
  fi
}

poll_until_terminal_status() {
  local response_id="$1"
  local attempts="$2"
  local interval_seconds="$3"
  local status=""

  for attempt in $(seq 1 "${attempts}"); do
    local body
    body="$(get_response_status "${response_id}")"
    status="$(json_get "${body}" 'payload["status"]')"
    echo "poll ${attempt}: ${status}" >&2
    case "${status}" in
      completed | failed | cancelled)
        echo "${status}"
        return 0
        ;;
    esac
    sleep "${interval_seconds}"
  done

  echo "response ${response_id} did not reach a terminal status (last: ${status:-unknown})" >&2
  exit 1
}

assert_status_equals() {
  local response_id="$1"
  local expected="$2"
  local attempts="$3"
  local interval_seconds="$4"

  for attempt in $(seq 1 "${attempts}"); do
    local body
    body="$(get_response_status "${response_id}")"
    local status
    status="$(json_get "${body}" 'payload["status"]')"
    if [[ "${status}" != "${expected}" ]]; then
      echo "expected status ${expected}, got ${status} for ${response_id}" >&2
      exit 1
    fi
    sleep "${interval_seconds}"
  done
}

test_messages_completion() {
  echo "=== messages API completion ==="
  local body
  body="$(post_messages "{\"model\":\"${DEFAULT_MODEL}\",\"max_tokens\":16,\"messages\":[{\"role\":\"user\",\"content\":\"Say hi in one word.\"}]}")"
  local message_type
  message_type="$(json_get "${body}" 'payload.get("type")')"
  if [[ "${message_type}" != "message" ]]; then
    echo "expected Anthropic message type 'message', got ${message_type}" >&2
    exit 1
  fi

  local output_tokens
  output_tokens="$(json_get "${body}" 'payload["usage"]["output_tokens"]')"
  if [[ "${output_tokens}" -le 0 ]]; then
    echo "expected output_tokens > 0, got ${output_tokens}" >&2
    exit 1
  fi

  local content_type
  content_type="$(json_get "${body}" 'payload["content"][0].get("type")')"
  if [[ "${content_type}" != "text" ]]; then
    echo "expected first content block type text, got ${content_type}" >&2
    exit 1
  fi
  echo "messages completion returned type=message with ${output_tokens} output token(s)"
}

test_messages_default_model() {
  echo "=== messages API default model ==="
  local body
  body="$(post_messages "{\"max_tokens\":8,\"messages\":[{\"role\":\"user\",\"content\":\"Hi\"}]}")"
  local model
  model="$(json_get "${body}" 'payload.get("model")')"
  if [[ "${model}" != "${DEFAULT_MODEL}" ]]; then
    echo "expected default model ${DEFAULT_MODEL}, got ${model}" >&2
    exit 1
  fi
  echo "messages request used default model ${model}"
}

test_messages_count_tokens() {
  echo "=== messages API count_tokens ==="
  local body
  body="$(post_messages_count_tokens "{\"model\":\"${DEFAULT_MODEL}\",\"messages\":[{\"role\":\"user\",\"content\":\"Hello\"}]}")"
  local input_tokens
  input_tokens="$(json_get "${body}" 'payload["input_tokens"]')"
  if [[ "${input_tokens}" -le 0 ]]; then
    echo "expected input_tokens > 0, got ${input_tokens}" >&2
    exit 1
  fi
  echo "messages count_tokens returned ${input_tokens} input token(s)"
}

test_sync_response_persistence() {
  echo "=== sync response persistence ==="
  local body
  body="$(post_response "{\"model\":\"${DEFAULT_MODEL}\",\"input\":\"Say hi in one word.\"}")"
  local response_id
  response_id="$(json_get "${body}" 'payload["id"]')"
  local stored_status
  stored_status="$(json_get "$(get_response_status "${response_id}")" 'payload["status"]')"
  if [[ "${stored_status}" != "completed" ]]; then
    echo "expected stored sync response to be completed, got ${stored_status}" >&2
    exit 1
  fi
  echo "sync response ${response_id} stored as completed"
}

test_background_response_completion() {
  echo "=== background response completion ==="
  local body
  body="$(post_response "{\"model\":\"${DEFAULT_MODEL}\",\"input\":\"Say bye in one word.\",\"background\":true}")"
  local response_id
  response_id="$(json_get "${body}" 'payload["id"]')"
  local initial_status
  initial_status="$(json_get "${body}" 'payload["status"]')"
  if [[ "${initial_status}" != "queued" ]]; then
    echo "expected initial background status queued, got ${initial_status}" >&2
    exit 1
  fi

  local final_status
  final_status="$(poll_until_terminal_status "${response_id}" "${BACKGROUND_POLL_ATTEMPTS}" "${BACKGROUND_POLL_INTERVAL_SECONDS}")"
  if [[ "${final_status}" != "completed" ]]; then
    echo "expected background response to complete, got ${final_status}" >&2
    exit 1
  fi
  echo "background response ${response_id} completed"
}

test_background_cancel_stays_cancelled() {
  echo "=== background cancel ==="
  local body
  body="$(post_response "{\"model\":\"${DEFAULT_MODEL}\",\"input\":\"Write a very long story about otters.\",\"background\":true}")"
  local response_id
  response_id="$(json_get "${body}" 'payload["id"]')"
  sleep 1

  local cancelled
  cancelled="$(curl -sf -X POST "${GATEWAY_BASE_URL}/v1/responses/${response_id}/cancel")"
  local cancel_status
  cancel_status="$(json_get "${cancelled}" 'payload["status"]')"
  if [[ "${cancel_status}" != "cancelled" ]]; then
    echo "expected cancel to return cancelled, got ${cancel_status}" >&2
    exit 1
  fi

  assert_status_equals "${response_id}" "cancelled" "${CANCEL_POLL_ATTEMPTS}" "${BACKGROUND_POLL_INTERVAL_SECONDS}"
  echo "background response ${response_id} remained cancelled"
}

test_background_delete_tombstone() {
  echo "=== background delete tombstone ==="
  local body
  body="$(post_response "{\"model\":\"${DEFAULT_MODEL}\",\"input\":\"Write a very long story about space otters.\",\"background\":true}")"
  local response_id
  response_id="$(json_get "${body}" 'payload["id"]')"
  sleep 1

  local deleted_body
  deleted_body="$(curl -sf -X DELETE "${GATEWAY_BASE_URL}/v1/responses/${response_id}")"
  if [[ "$(json_get "${deleted_body}" 'payload["deleted"]')" != "True" ]]; then
    echo "expected delete to return deleted=true, got: ${deleted_body}" >&2
    exit 1
  fi

  for attempt in $(seq 1 "${DELETE_POLL_ATTEMPTS}"); do
    local code
    code="$(http_status GET "/v1/responses/${response_id}")"
    if [[ "${code}" != "404" ]]; then
      echo "expected deleted response to return 404, got ${code}" >&2
      exit 1
    fi
    sleep "${BACKGROUND_POLL_INTERVAL_SECONDS}"
  done
  echo "deleted background response ${response_id} stayed unavailable"
}

test_in_flight_continuation_rejected() {
  echo "=== in-flight continuation rejection ==="
  local body
  body="$(post_response "{\"model\":\"${DEFAULT_MODEL}\",\"input\":\"Long story.\",\"background\":true}")"
  local response_id
  response_id="$(json_get "${body}" 'payload["id"]')"
  local code
  code="$(http_status POST "/v1/responses" "{\"model\":\"${DEFAULT_MODEL}\",\"previous_response_id\":\"${response_id}\",\"input\":\"continue\",\"background\":true}")"
  if [[ "${code}" != "409" ]]; then
    echo "expected in-flight continuation to return 409, got ${code}" >&2
    exit 1
  fi
  echo "in-flight continuation returned 409"
}

background_queue_enabled() {
  local deployment="${RELEASE_NAME}-duihua-ai-services-gateway"
  local enabled
  enabled="$(kubectl get deployment "${deployment}" -n "${NAMESPACE}" \
    -o jsonpath='{.spec.template.spec.containers[0].env[?(@.name=="RESPONSES_BACKGROUND_ENABLED")].value}' 2>/dev/null || true)"
  [[ "${enabled}" == "true" ]]
}

background_worker_deployed() {
  local deployment="${RELEASE_NAME}-duihua-ai-services-background-worker"
  kubectl get deployment "${deployment}" -n "${NAMESPACE}" >/dev/null 2>&1
}

skip_background_completion_tests() {
  if [[ "${SMOKE_TEST_SKIP_BACKGROUND_COMPLETION:-}" == "true" ]]; then
    return 0
  fi
  if background_queue_enabled && ! background_worker_deployed; then
    return 0
  fi
  return 1
}

test_background_worker_resources() {
  echo "=== background worker resources ==="
  if ! background_queue_enabled; then
    echo "background queue disabled; skipping worker resource checks"
    return 0
  fi
  if ! background_worker_deployed; then
    echo "background queue enabled without worker Deployment; skipping resource checks"
    return 0
  fi
  python3 - "${NAMESPACE}" "${RELEASE_NAME}" "${KUBECTL_CONTEXT}" <<'PY'
import json
import os
import subprocess
import sys

namespace = sys.argv[1]
release_name = sys.argv[2]
kubectl = ["kubectl"]
context = sys.argv[3] if len(sys.argv) > 3 else os.environ.get("KUBECTL_CONTEXT", "")
if context:
    kubectl.extend(["--context", context])
deployment = f"{release_name}-duihua-ai-services-background-worker"
raw = subprocess.check_output(
    [*kubectl, "get", "deployment", deployment, "-n", namespace, "-o", "json"],
    text=True,
)
resources = (
    json.loads(raw)
    .get("spec", {})
    .get("template", {})
    .get("spec", {})
    .get("containers", [{}])[0]
    .get("resources", {})
)
requests = resources.get("requests") or {}
if not requests:
    raise SystemExit(f"background worker deployment is missing resource requests: {resources}")

print(f"background worker resources OK: {resources}")
PY
}

main() {
  require_command curl
  require_command python3
  require_command kubectl

  wait_for_gateway
  wait_for_background_worker
  test_messages_completion
  test_messages_default_model
  test_messages_count_tokens
  test_sync_response_persistence
  if skip_background_completion_tests; then
    echo "=== background response completion ==="
    echo "background queue enabled without worker Deployment; skipping completion poll"
  else
    test_background_response_completion
  fi
  test_background_cancel_stays_cancelled
  test_background_delete_tombstone
  test_in_flight_continuation_rejected
  test_background_worker_resources

  echo
  echo "All kind gateway smoke tests passed."
}

main "$@"