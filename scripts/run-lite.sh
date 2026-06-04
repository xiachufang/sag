#!/usr/bin/env bash
# Load .env and run the gateway in Lite mode via cargo.
#
# Usage:
#   ./scripts/run-lite.sh                 # use config/lite.yaml
#   ./scripts/run-lite.sh --config xxx    # extra args are forwarded to gateway

set -euo pipefail

# Always run from the repo root so relative paths in config files resolve.
cd "$(dirname "$0")/.."

# Look for .env in the repo root first, then the parent directory.
ENV_FILE=""
if [[ -f .env ]]; then
  ENV_FILE=.env
elif [[ -f ../.env ]]; then
  ENV_FILE=../.env
else
  echo "error: .env not found at $(pwd)/.env or $(cd .. && pwd)/.env" >&2
  echo "       cp .env.example .env, then fill in GATEWAY_MASTER_KEY etc." >&2
  exit 1
fi

# Export every variable defined in the .env file. `set -a` makes plain
# assignments auto-exported; `set +a` turns it off again so we don't pollute
# later commands.
echo "loading env from $ENV_FILE"
set -a
# shellcheck disable=SC1091
source "$ENV_FILE"
set +a

# If the caller passed args, use them as-is; otherwise default to lite config.
if [[ $# -gt 0 ]]; then
  exec cargo run --bin gateway -- "$@"
else
  exec cargo run --bin gateway -- --config config/lite.yaml
fi
