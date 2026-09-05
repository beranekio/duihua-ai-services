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
if ! command -v python3 >/dev/null; then
  echo "python3 is required" >&2
  exit 1
fi
if ! command -v helm >/dev/null; then
  echo "helm is required" >&2
  exit 1
fi

OWNER="${GITHUB_REPOSITORY_OWNER:-beranekio}"

# Read or write a dependency version in Chart.yaml without yq/PyYAML.
# Expects the standard Helm list shape under `dependencies:`.
chart_dep_version() {
  local op="$1" name="$2" version="${3:-}"
  NAME="$name" VERSION="$version" OP="$op" CHART_YAML="$CHART_YAML" python3 <<'PY'
import os, pathlib, re, sys

path = pathlib.Path(os.environ["CHART_YAML"])
name = os.environ["NAME"]
op = os.environ["OP"]
new_version = os.environ.get("VERSION", "")
text = path.read_text()
lines = text.splitlines(keepends=True)

in_deps = False
current_name = None
found = False
out = []

for line in lines:
    if re.match(r"^dependencies:\s*$", line):
        in_deps = True
        out.append(line)
        continue

    if in_deps and re.match(r"^[A-Za-z]", line):
        in_deps = False
        current_name = None

    if in_deps:
        m_name = re.match(r"^(\s*)-\s*name:\s*(.+?)\s*$", line)
        if m_name:
            current_name = m_name.group(2).strip().strip("\"'")
            out.append(line)
            continue

        m_ver = re.match(r"^(\s*)version:\s*(.+?)\s*$", line)
        if m_ver and current_name == name:
            found = True
            current = m_ver.group(2).strip().strip("\"'")
            if op == "get":
                print(current)
                sys.exit(0)
            if op == "set":
                indent = m_ver.group(1)
                out.append(f"{indent}version: {new_version}\n")
                current_name = None
                continue

    out.append(line)

if not found:
    print(f"dependency {name!r} not found in {path}", file=sys.stderr)
    sys.exit(1)

if op == "get":
    print(f"dependency {name!r} has no version field in {path}", file=sys.stderr)
    sys.exit(1)

if op == "set":
    path.write_text("".join(out))
PY
}

changed=0
summary=()

for name in duihua-gateway duihua-background-worker responses-api-store; do
  repo="$name"
  sha="$(gh api "repos/${OWNER}/${repo}/commits/main" --jq .sha)"
  version="0.0.0-${sha}"
  current="$(chart_dep_version get "${name}")"
  if [[ -z "${current}" || "${current}" == "null" ]]; then
    echo "dependency ${name}: missing version in Chart.yaml" >&2
    exit 1
  fi
  if [[ "${current}" == "${version}" ]]; then
    echo "${name}: already at ${version}"
    continue
  fi
  echo "${name}: ${current} -> ${version}"
  chart_dep_version set "${name}" "${version}"
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
