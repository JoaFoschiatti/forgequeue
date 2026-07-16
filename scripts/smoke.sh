#!/usr/bin/env bash
set -euo pipefail

api_url="${API_URL:-http://localhost:8080}"
image_fixture="$(mktemp --suffix=.png)"
pdf_fixture="$(mktemp --suffix=.pdf)"
trap 'rm -f "$image_fixture" "$pdf_fixture"' EXIT

printf '%s' 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=' | base64 --decode >"$image_fixture"

python3 - "$pdf_fixture" <<'PY'
import sys

output = sys.argv[1]
stream = b"BT /F1 24 Tf 72 720 Td (ForgeQueue PDF smoke) Tj ET\n"
objects = [
    b"<< /Type /Catalog /Pages 2 0 R >>",
    b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
    b"/Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
    b"<< /Length %d >>\nstream\n" % len(stream) + stream + b"endstream",
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
]

pdf = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
offsets = [0]
for number, body in enumerate(objects, start=1):
    offsets.append(len(pdf))
    pdf.extend(f"{number} 0 obj\n".encode())
    pdf.extend(body)
    pdf.extend(b"\nendobj\n")
xref = len(pdf)
pdf.extend(f"xref\n0 {len(objects) + 1}\n".encode())
pdf.extend(b"0000000000 65535 f \n")
for offset in offsets[1:]:
    pdf.extend(f"{offset:010d} 00000 n \n".encode())
pdf.extend(
    f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\n"
    f"startxref\n{xref}\n%%EOF\n".encode()
)
with open(output, "wb") as handle:
    handle.write(pdf)
PY

native_path() {
  if [[ -n "${MSYSTEM:-}" ]] && command -v cygpath >/dev/null 2>&1; then
    cygpath --windows "$1"
  else
    printf '%s\n' "$1"
  fi
}

image_upload_path="$(native_path "$image_fixture")"
pdf_upload_path="$(native_path "$pdf_fixture")"

for _ in $(seq 1 120); do
  if curl -fsS "$api_url/health/ready" >/dev/null; then
    break
  fi
  sleep 1
done
curl -fsS "$api_url/health/ready" >/dev/null

session_json="$(curl -fsS -X POST "$api_url/api/v1/sessions")"
token="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])' <<<"$session_json")"

job_json="$(curl -fsS \
  -H "Authorization: Bearer $token" \
  -H 'Idempotency-Key: smoke-upload' \
  -F "file=@$image_upload_path;type=image/png" \
  "$api_url/api/v1/jobs")"
job_id="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' <<<"$job_json")"

duplicate_json="$(curl -fsS \
  -H "Authorization: Bearer $token" \
  -H 'Idempotency-Key: smoke-upload' \
  -F "file=@$image_upload_path;type=image/png" \
  "$api_url/api/v1/jobs")"
duplicate_id="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' <<<"$duplicate_json")"
test "$duplicate_id" = "$job_id"

detail_json=''
for _ in $(seq 1 120); do
  detail_json="$(curl -fsS -H "Authorization: Bearer $token" "$api_url/api/v1/jobs/$job_id")"
  status="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])' <<<"$detail_json")"
  case "$status" in
    succeeded) break ;;
    dead_lettered|cancelled|expired)
      echo "job ended unexpectedly with status=$status" >&2
      exit 1
      ;;
  esac
  sleep 1
done

python3 -c '
import json, sys
detail = json.load(sys.stdin)
assert detail["status"] == "succeeded", detail
assert detail["attempt_count"] == 1, detail
assert {item["name"] for item in detail["outputs"]} == {"metadata.json", "thumbnail.webp", "preview.webp"}, detail
' <<<"$detail_json"

output_id="$(python3 -c 'import json,sys; print(next(item["id"] for item in json.load(sys.stdin)["outputs"] if item["name"] == "preview.webp"))' <<<"$detail_json")"
curl -fsS -H "Authorization: Bearer $token" \
  "$api_url/api/v1/jobs/$job_id/outputs/$output_id" -o /tmp/forgequeue-smoke-preview.webp
test -s /tmp/forgequeue-smoke-preview.webp
rm -f /tmp/forgequeue-smoke-preview.webp

other_session="$(curl -fsS -X POST "$api_url/api/v1/sessions")"
other_token="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])' <<<"$other_session")"
http_status="$(curl -sS -o /tmp/forgequeue-isolation.json -w '%{http_code}' \
  -H "Authorization: Bearer $other_token" "$api_url/api/v1/jobs/$job_id")"
test "$http_status" = '404'
rm -f /tmp/forgequeue-isolation.json

pdf_job_ids=()
for number in 1 2; do
  pdf_job_json="$(curl -fsS \
    -H "Authorization: Bearer $token" \
    -H "Idempotency-Key: smoke-pdf-$number" \
    -F "file=@$pdf_upload_path;type=application/pdf" \
    "$api_url/api/v1/jobs")"
  pdf_job_ids+=("$(python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' <<<"$pdf_job_json")")
done

for pdf_job_id in "${pdf_job_ids[@]}"; do
  pdf_detail=''
  for _ in $(seq 1 120); do
    pdf_detail="$(curl -fsS -H "Authorization: Bearer $token" "$api_url/api/v1/jobs/$pdf_job_id")"
    status="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])' <<<"$pdf_detail")"
    case "$status" in
      succeeded) break ;;
      dead_lettered|cancelled|expired)
        echo "PDF job ended unexpectedly with status=$status" >&2
        exit 1
        ;;
    esac
    sleep 1
  done

  python3 -c '
import json, sys
detail = json.load(sys.stdin)
assert detail["status"] == "succeeded", detail
assert detail["attempt_count"] == 1, detail
assert {item["name"] for item in detail["outputs"]} == {"metadata.json", "page-1.png"}, detail
' <<<"$pdf_detail"
done

curl -fsS "$api_url/metrics" | grep -q 'forgequeue_jobs_created_total'

worker_metrics_urls="${WORKER_METRICS_URLS-http://localhost:9101,http://localhost:9102}"
if [[ -n "$worker_metrics_urls" ]]; then
  worker_metrics=''
  IFS=',' read -r -a metrics_urls <<<"$worker_metrics_urls"
  for metrics_url in "${metrics_urls[@]}"; do
    worker_metrics+="$(curl -fsS "$metrics_url/metrics")"
  done
  grep -q 'forgequeue_jobs_completed_total' <<<"$worker_metrics"
fi
echo "ForgeQueue smoke test passed: image=$job_id pdfs=${pdf_job_ids[*]}"
