#!/usr/bin/env bash
set -euo pipefail

scripts/create-kind-cluster.sh
scripts/install-keda.sh
scripts/deploy-kind.sh
