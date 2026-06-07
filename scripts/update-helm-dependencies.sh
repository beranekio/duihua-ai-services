#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHART_DIR="${CHART_DIR:-$ROOT_DIR/charts/duihua-ai-services}"

echo "Updating Helm dependencies for ${CHART_DIR}..."
helm dependency update "${CHART_DIR}"

echo "Helm dependencies updated."