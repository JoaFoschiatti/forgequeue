#!/usr/bin/env bash
set -euo pipefail

api_url="${API_URL:?Definí API_URL con la URL HTTPS de Koyeb.}"
frontend_url="${FRONTEND_URL:?Definí FRONTEND_URL con la URL HTTPS de Cloudflare Pages.}"
api_url="${api_url%/}"
frontend_url="${frontend_url%/}"
allow_local_http="${VERIFY_ALLOW_HTTP:-false}"

validate_url() {
  local name="$1"
  local url="$2"
  case "$url" in
    https://*) ;;
    http://localhost:*|http://127.0.0.1:*)
      if [[ "$allow_local_http" != true ]]; then
        echo "$name sólo admite HTTP local con VERIFY_ALLOW_HTTP=true." >&2
        exit 2
      fi
      ;;
    *) echo "$name debe usar HTTPS." >&2; exit 2 ;;
  esac
}

validate_url API_URL "$api_url"
validate_url FRONTEND_URL "$frontend_url"

frontend_body="$(mktemp)"
openapi_body="$(mktemp)"
cors_headers="$(mktemp)"
trap 'rm -f "$frontend_body" "$openapi_body" "$cors_headers"' EXIT

echo "Esperando que el backend esté listo (incluye cold start)..."
ready=false
for _ in $(seq 1 150); do
  if curl -fsS --max-time 10 "$api_url/health/ready" >/dev/null; then
    ready=true
    break
  fi
  sleep 2
done
if [[ "$ready" != true ]]; then
  echo "El backend no quedó listo dentro de cinco minutos." >&2
  exit 1
fi

curl -fsS --max-time 30 "$frontend_url/" -o "$frontend_body"
grep -q '<title>ForgeQueue' "$frontend_body"

curl -fsS --max-time 30 "$api_url/api/openapi.json" -o "$openapi_body"
python3 - "$openapi_body" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    document = json.load(handle)

assert document["info"]["title"] == "ForgeQueue API", document["info"]
assert "/api/v1/jobs/{id}/events" in document["paths"], document["paths"].keys()
assert "/api/v1/jobs/{job_id}/outputs/{output_id}" in document["paths"], document["paths"].keys()
PY

cors_status="$(curl -sS --max-time 30 -D "$cors_headers" -o /dev/null -w '%{http_code}' \
  -X OPTIONS "$api_url/api/v1/jobs" \
  -H "Origin: $frontend_url" \
  -H 'Access-Control-Request-Method: POST' \
  -H 'Access-Control-Request-Headers: authorization,idempotency-key')"
test "$cors_status" = 200
python3 - "$cors_headers" "$frontend_url" <<'PY'
import sys

headers_path, expected_origin = sys.argv[1:]
headers = {}
with open(headers_path, encoding="utf-8") as handle:
    for line in handle:
        if ":" in line:
            name, value = line.split(":", 1)
            headers[name.strip().lower()] = value.strip()

assert headers.get("access-control-allow-origin") == expected_origin, headers
allowed_headers = {value.strip().lower() for value in headers.get("access-control-allow-headers", "").split(",")}
assert {"authorization", "idempotency-key"} <= allowed_headers, headers
PY

API_URL="$api_url" WORKER_METRICS_URLS= bash "$(dirname "$0")/smoke.sh"
echo "Demo pública verificada: frontend=$frontend_url backend=$api_url"
