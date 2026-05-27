#!/usr/bin/env bash
# Lifecycle entry point for the aviso e2e docker-compose stack.
#
# Subcommands:
#   bash tests/e2e/shared/stack.sh up           Bring the stack up and poll each service for readiness.
#   bash tests/e2e/shared/stack.sh down         Stop the stack and remove the JetStream volume (data wiped).
#   bash tests/e2e/shared/stack.sh restart      down then up.
#   bash tests/e2e/shared/stack.sh logs [...]   Tail docker compose logs; extra args go to `docker compose logs`.
#   bash tests/e2e/shared/stack.sh status       Print `docker compose ps` and probe each readiness endpoint once.
#
# Why this exists instead of bare `docker compose`: the upstream aviso-server, auth-o-tron, and
# nats images do not declare healthchecks, so `docker compose up -d --wait` only confirms that
# the processes started, not that the HTTP endpoints answer. `up` and `status` poll the HTTP
# endpoints directly so a test runner cannot start before the stack is actually ready.
#
# Optional env vars (consumed by up / restart / status):
#   AVISO_SERVER_HOST_PORT       (default 8000)
#   AUTH_O_TRON_HOST_PORT        (default 8080)
#   NATS_HOST_PORT               (default 4222; NATS client port)
#   NATS_MONITORING_HOST_PORT    (default 8222; NATS monitoring port the readiness probe polls)
#   AVISO_E2E_TIMEOUT_SECS       (default 60; readiness-probe deadline for `up`)

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &> /dev/null && pwd)"
E2E_DIR="$(cd -- "$SCRIPT_DIR/.." &> /dev/null && pwd)"

AVISO_PORT="${AVISO_SERVER_HOST_PORT:-8000}"
AUTH_PORT="${AUTH_O_TRON_HOST_PORT:-8080}"
NATS_PORT="${NATS_MONITORING_HOST_PORT:-8222}"
TIMEOUT="${AVISO_E2E_TIMEOUT_SECS:-60}"

usage() {
  cat >&2 <<EOF
Usage: bash $0 <up|down|restart|logs|status> [extra args...]

  up            Bring the stack up and poll each service for readiness.
  down          Stop the stack and remove the JetStream volume (data wiped).
  restart       down then up.
  logs [...]    Tail docker compose logs. Extra args pass through to docker compose logs.
  status        Print docker compose ps and probe each readiness endpoint once.

Env vars for up/restart/status:
  AVISO_SERVER_HOST_PORT (default 8000)
  AUTH_O_TRON_HOST_PORT (default 8080)
  NATS_HOST_PORT (default 4222)
  NATS_MONITORING_HOST_PORT (default 8222)
  AVISO_E2E_TIMEOUT_SECS (default 60)
EOF
}

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
  docker compose logs >&2 || true
  return 1
}

probe_endpoint() {
  local name="$1"
  local url="$2"
  if curl -fsS --max-time 2 "$url" >/dev/null 2>&1; then
    printf '  %-14s READY  %s\n' "$name" "$url"
  else
    printf '  %-14s DOWN   %s\n' "$name" "$url"
  fi
}

cmd_up() {
  cd "$E2E_DIR"
  docker compose up -d
  poll_endpoint "aviso-server" "http://127.0.0.1:${AVISO_PORT}/health"
  poll_endpoint "auth-o-tron"  "http://127.0.0.1:${AUTH_PORT}/health"
  poll_endpoint "nats"         "http://127.0.0.1:${NATS_PORT}/healthz"
  echo "all services ready"
}

cmd_down() {
  cd "$E2E_DIR"
  docker compose down -v
}

cmd_restart() {
  cmd_down
  cmd_up
}

cmd_logs() {
  cd "$E2E_DIR"
  docker compose logs --tail 100 "$@"
}

cmd_status() {
  cd "$E2E_DIR"
  docker compose ps
  echo
  echo "readiness probes:"
  probe_endpoint "aviso-server" "http://127.0.0.1:${AVISO_PORT}/health"
  probe_endpoint "auth-o-tron"  "http://127.0.0.1:${AUTH_PORT}/health"
  probe_endpoint "nats"         "http://127.0.0.1:${NATS_PORT}/healthz"
}

if [ $# -lt 1 ]; then
  usage
  exit 2
fi

cmd="$1"
shift

case "$cmd" in
  up)             cmd_up ;;
  down)           cmd_down ;;
  restart)        cmd_restart ;;
  logs)           cmd_logs "$@" ;;
  status)         cmd_status ;;
  -h|--help|help) usage; exit 0 ;;
  *)
    echo "unknown subcommand: $cmd" >&2
    usage
    exit 2
    ;;
esac
