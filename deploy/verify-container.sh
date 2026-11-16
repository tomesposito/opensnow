#!/usr/bin/env bash
# Build only committed files and remove all test resources on exit.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v docker >/dev/null || { echo 'ERROR: Docker is required; container run is unverified.' >&2; exit 1; }
docker info >/dev/null
command -v python3 >/dev/null
work=$(mktemp -d)
suffix="opensnow-verify-$$"
cleanup() {
  docker rm -fv "$suffix-app" "$suffix-pg" >/dev/null 2>&1 || true
  docker volume rm "$suffix-warehouse" >/dev/null 2>&1 || true
  docker network rm "$suffix" >/dev/null 2>&1 || true
  docker image rm "$suffix" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT
# Default to the committed checkout; an explicit tree also permits testing
# a staged source snapshot before committing. Never include untracked files.
git archive "${1:-HEAD}" | tar -x -C "$work"
docker build --pull --platform linux/amd64 -t "$suffix" "$work"
docker network create "$suffix" >/dev/null
docker volume create "$suffix-warehouse" >/dev/null
password=$(python3 -c 'import secrets; print(secrets.token_hex(24))')
docker run -d --name "$suffix-pg" --network "$suffix" \
  -e POSTGRES_PASSWORD="$password" -e POSTGRES_DB=opensnow_verify postgres:16-bookworm >/dev/null
ready=false
for _ in {1..60}; do
  if docker exec "$suffix-pg" pg_isready -h 127.0.0.1 -U postgres -d opensnow_verify >/dev/null; then ready=true; break; fi
  sleep 1
done
$ready || { docker logs "$suffix-pg"; exit 1; }
docker run -d --name "$suffix-app" --network "$suffix" \
  -p 127.0.0.1::8080 -v "$suffix-warehouse:/warehouse" \
  -e OPENSNOW_CONTROL_DATABASE_URL="postgres://postgres:$password@$suffix-pg/opensnow_verify" \
  "$suffix" >/dev/null
port=$(docker port "$suffix-app" 8080/tcp | sed 's/.*://')
python3 deploy/round-trip.py "http://127.0.0.1:$port" || { docker logs "$suffix-app"; exit 1; }
echo 'PASS clean-checkout container build and round-trip'
