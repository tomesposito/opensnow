#!/usr/bin/env bash
# The brief's runtime-gap fallback: real binary, throwaway DB, HTTP and HTTPS.
set -euo pipefail
cd "$(dirname "$0")/.."
binary=$(realpath "${1:?usage: verify-native.sh /path/to/release/opensnow-mcp}")
[[ -x "$binary" ]]
command -v psql >/dev/null
command -v caddy >/dev/null
admin=${OPENSNOW_TEST_ADMIN_URL:-postgres://postgres@localhost:5433/postgres}
db="opensnow_deploy_$(python3 -c 'import uuid; print(uuid.uuid4().hex)')"
work=$(mktemp -d)
server_pid=
cleanup() {
  if [[ -n "$server_pid" ]]; then kill "$server_pid" 2>/dev/null || true; wait "$server_pid" 2>/dev/null || true; fi
  psql "$admin" -v ON_ERROR_STOP=1 -c "DROP DATABASE IF EXISTS $db WITH (FORCE)" >/dev/null
  rm -rf "$work"
}
trap cleanup EXIT
psql "$admin" -v ON_ERROR_STOP=1 -c "CREATE DATABASE $db" >/dev/null
uri=$(python3 - "$admin" "$db" <<'PY'
import sys, urllib.parse
url = urllib.parse.urlsplit(sys.argv[1])
print(urllib.parse.urlunsplit(url._replace(path='/' + sys.argv[2])))
PY
)
OPENSNOW_CONTROL_DATABASE_URL="$uri" OPENSNOW_WAREHOUSE="$work/warehouse" \
  "$binary" --http 127.0.0.1:8080 >"$work/server.log" 2>&1 &
server_pid=$!
python3 deploy/round-trip.py http://127.0.0.1:8080 || { cat "$work/server.log"; exit 1; }
./deploy/verify-tls.sh || { cat "$work/server.log"; exit 1; }
echo 'PASS native binary HTTP and HTTPS; throwaway database and warehouse removed on exit'
