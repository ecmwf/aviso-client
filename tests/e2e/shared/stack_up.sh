#!/usr/bin/env bash
# Bring up the aviso e2e stack and poll readiness on each service before returning.
#
# Usage:
#   bash tests/e2e/shared/stack_up.sh
#
# Why this exists instead of `docker compose up -d --wait`: the upstream aviso-server,
# auth-o-tron, and nats images do not declare healthchecks in the compose file, so `--wait`
# only confirms processes started, not that the HTTP endpoints answer. This script polls each
# service's HTTP /health (or /healthz for NATS) directly and exits 0 only when all three are
# ready. On timeout, dumps `docker compose logs` and exits non-zero.
#
# Optional env vars:
#   AVISO_SERVER_HOST_PORT      (default 8000)
#   AUTH_O_TRON_HOST_PORT       (default 8080)
#   NATS_MONITORING_HOST_PORT   (default 8222)
#   AVISO_E2E_TIMEOUT_SECS      (default 60)

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &> /dev/null && pwd)"
E2E_DIR="$(cd -- "$SCRIPT_DIR/.." &> /dev/null && pwd)"

AVISO_PORT="${AVISO_SERVER_HOST_PORT:-8000}"
AUTH_PORT="${AUTH_O_TRON_HOST_PORT:-8080}"
NATS_PORT="${NATS_MONITORING_HOST_PORT:-8222}"
TIMEOUT="${AVISO_E2E_TIMEOUT_SECS:-60}"

cd "$E2E_DIR"
docker compose up -d

poll_endpoint() {
  local name="$1"
  local url="$2"
  local deadline
  deadline=$(( $(date +%s) + TIMEOUT ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    if curl -fsS --max-time 2 "$url" >/dev/null 2>&1; then
      echo "$name ready at $url"
      return 0
    fi
    sleep 0.25
  done
  echo "ERROR: $name did not become ready at $url within ${TIMEOUT}s" >&2
  docker compose logs >&2
  return 1
}

poll_endpoint "aviso-server" "http://127.0.0.1:${AVISO_PORT}/health"
poll_endpoint "auth-o-tron"  "http://127.0.0.1:${AUTH_PORT}/health"
poll_endpoint "nats"         "http://127.0.0.1:${NATS_PORT}/healthz"

echo "all services ready"
