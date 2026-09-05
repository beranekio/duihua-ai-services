#!/usr/bin/env bash
# Bump OCI subchart pins in charts/duihua-ai-services to each source repo's main SHA.
# Chart versions are published as 0.0.0-<gitsha> (see component publish workflows).
# Stock Dependabot cannot usefully order those versions, so this script is the updater.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHART_DIR="${ROOT}/charts/duihua-ai-services"
CHART_YAML="${CHART_DIR}/Chart.yaml"

if ! command -v gh >/dev/null; then
  echo "gh is required" >&2
  exit 1
fi
if ! command -v yq >/dev/null; then
  echo "yq is required" >&2
  exit 1
fi
if ! command -v helm >/dev/null; then
  echo "helm is required" >&2
  exit 1
fi

OWNER="${GITHUB_REPOSITORY_OWNER:-beranekio}"

declare -A SOURCE_REPO=(
  [duihua-gateway]=duihua-gateway
  [duihua-background-worker]=duihua-background-worker
  [responses-api-store]=responses-api-store
)

changed=0
summary=()

for name in duihua-gateway duihua-background-worker responses-api-store; do
  repo="${SOURCE_REPO[$name]}"
  sha="$(gh api "repos/${OWNER}/${repo}/commits/main" --jq .sha)"
  version="0.0.0-${sha}"
  current="$(yq -r ".dependencies[] | select(.name == \"${name}\") | .version" "${CHART_YAML}")"
  if [[ "${current}" == "${version}" ]]; then
    echo "${name}: already at ${version}"
    continue
  fi
  echo "${name}: ${current} -> ${version}"
  yq -i "(.dependencies[] | select(.name == \"${name}\") | .version) = \"${version}\"" "${CHART_YAML}"
  changed=1
  summary+=("- ${name}: \`${current}\` → \`${version}\`")
done

if [[ "${changed}" -eq 0 ]]; then
  echo "No subchart pin updates."
  exit 0
fi

echo "Refreshing Chart.lock via helm dependency update..."
helm dependency update "${CHART_DIR}"

if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  {
    echo "### OCI subchart pin updates"
    printf '%s\n' "${summary[@]}"
  } >> "${GITHUB_STEP_SUMMARY}"
fi

echo "Updated Chart.yaml and Chart.lock."
