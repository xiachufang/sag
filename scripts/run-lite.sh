#!/usr/bin/env bash
# Load .env and run the gateway in Lite mode via cargo.
#
# Usage:
#   ./scripts/run-lite.sh                 # use config/example.lite.yaml
#   ./scripts/run-lite.sh --config xxx    # extra args are forwarded to gateway

set -euo pipefail

# Always run from the repo root so relative paths in config files resolve.
cd "$(dirname "$0")/.."

if [[ ! -f .env ]]; then
  echo "error: .env not found at $(pwd)/.env" >&2
  echo "       cp .env.example .env, then fill in GATEWAY_MASTER_KEY etc." >&2
  exit 1
fi

# Export every variable defined in .env. `set -a` makes plain assignments
# auto-exported; `set +a` turns it off again so we don't pollute later commands.
set -a
# shellcheck disable=SC1091
source .env
set +a

# If the caller passed args, use them as-is; otherwise default to lite config.
if [[ $# -gt 0 ]]; then
  exec cargo run --bin gateway -- "$@"
else
  exec cargo run --bin gateway -- --config config/example.lite.yaml
fi
