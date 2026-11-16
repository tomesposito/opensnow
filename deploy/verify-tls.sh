#!/usr/bin/env bash
# Real Caddy and the shipped proxy block. Backend must use a throwaway DB.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v caddy >/dev/null
command -v openssl >/dev/null
work=$(mktemp -d)
proxy_pid=
cleanup() { if [[ -n "$proxy_pid" ]]; then kill "$proxy_pid" 2>/dev/null || true; wait "$proxy_pid" 2>/dev/null || true; fi; rm -rf "$work"; }
trap cleanup EXIT
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=localhost \
  -addext subjectAltName=DNS:localhost -keyout "$work/key.pem" -out "$work/cert.pem" 2>"$work/openssl.log"
export OPENSNOW_DOMAIN=localhost:8443 OPENSNOW_TLS_EMAIL=local@example.test
# Production config is parsed unchanged before substituting only certificate
# issuance for a self-signed certificate and moving the local test port.
caddy validate --config deploy/Caddyfile --adapter caddyfile
python3 - "$work" <<'PY'
import pathlib, sys
work = pathlib.Path(sys.argv[1])
config = pathlib.Path('deploy/Caddyfile').read_text()
start = config.index('\ttls {')
end = config.index('\n\treverse_proxy', start)
config = config[:start] + '\ttls ' + str(work/'cert.pem') + ' ' + str(work/'key.pem') + '\n' + config[end:]
config = config.replace('auto_https disable_redirects', 'auto_https disable_redirects\n\tadmin off')
(work/'Caddyfile').write_text(config)
PY
caddy validate --config "$work/Caddyfile" --adapter caddyfile
caddy run --config "$work/Caddyfile" --adapter caddyfile >"$work/caddy.log" 2>&1 &
proxy_pid=$!
OPENSNOW_TEST_CA="$work/cert.pem" python3 deploy/round-trip.py https://localhost:8443 || { cat "$work/caddy.log"; exit 1; }
echo 'PASS HTTPS via real Caddy; shipped config validated; self-signed test certificate'
