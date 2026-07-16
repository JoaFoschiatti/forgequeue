#!/usr/bin/env bash
set -euo pipefail

api_url="${API_URL:-http://localhost:8080}"
image_fixture="$(mktemp --suffix=.png)"
pdf_fixture="$(mktemp --suffix=.pdf)"
excessive_pdf_fixture="$(mktemp --suffix=.pdf)"
fake_image_fixture="$(mktemp --suffix=.png)"
large_fixture="$(mktemp --suffix=.png)"
problem_headers="$(mktemp)"
problem_body="$(mktemp)"
trap 'rm -f "$image_fixture" "$pdf_fixture" "$excessive_pdf_fixture" "$fake_image_fixture" "$large_fixture" "$problem_headers" "$problem_body"' EXIT

printf '%s' 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=' | base64 --decode >"$image_fixture"

printf '%s' 'this is not an image' >"$fake_image_fixture"

python3 - "$pdf_fixture" "$excessive_pdf_fixture" "$large_fixture" <<'PY'
import sys

def write_pdf(output: str, page_count: int) -> None:
    objects: list[bytes] = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]
    page_ids = [4 + index * 2 for index in range(page_count)]
    kids = " ".join(f"{page_id} 0 R" for page_id in page_ids)
    objects[1] = f"<< /Type /Pages /Kids [{kids}] /Count {page_count} >>".encode()

    for index, page_id in enumerate(page_ids, start=1):
        content_id = page_id + 1
        stream = f"BT /F1 24 Tf 72 720 Td (ForgeQueue PDF smoke page {index}) Tj ET\n".encode()
        objects.append(
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
            f"/Resources << /Font << /F1 3 0 R >> >> /Contents {content_id} 0 R >>".encode()
        )
        objects.append(b"<< /Length %d >>\nstream\n" % len(stream) + stream + b"endstream")

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


write_pdf(sys.argv[1], 4)
write_pdf(sys.argv[2], 21)
with open(sys.argv[3], "wb") as handle:
    handle.truncate(10 * 1024 * 1024 + 1)
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
excessive_pdf_upload_path="$(native_path "$excessive_pdf_fixture")"
fake_image_upload_path="$(native_path "$fake_image_fixture")"
large_upload_path="$(native_path "$large_fixture")"

assert_problem() {
  local actual_status="$1"
  local expected_status="$2"
  local expected_code="$3"
  test "$actual_status" = "$expected_status"
  python3 - "$problem_headers" "$problem_body" "$expected_status" "$expected_code" <<'PY'
import json
import sys

headers_path, body_path, expected_status, expected_code = sys.argv[1:]
with open(headers_path, encoding="utf-8") as handle:
    header_lines = [line.strip() for line in handle if ":" in line]
request_ids = [line.split(":", 1)[1].strip() for line in header_lines if line.lower().startswith("x-request-id:")]
content_types = [line.split(":", 1)[1].strip() for line in header_lines if line.lower().startswith("content-type:")]
with open(body_path, encoding="utf-8") as handle:
    problem = json.load(handle)

assert request_ids, header_lines
assert content_types and content_types[-1].startswith("application/problem+json"), content_types
assert problem["status"] == int(expected_status), problem
assert problem["code"] == expected_code, problem
assert problem["request_id"] == request_ids[-1], (problem, request_ids)
PY
}

for _ in $(seq 1 120); do
  if curl -fsS "$api_url/health/ready" >/dev/null; then
    break
  fi
  sleep 1
done
curl -fsS "$api_url/health/ready" >/dev/null

session_json="$(curl -fsS -X POST "$api_url/api/v1/sessions")"
token="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])' <<<"$session_json")"

fake_status="$(curl -sS -D "$problem_headers" -o "$problem_body" -w '%{http_code}' \
  -H "Authorization: Bearer $token" \
  -H 'Idempotency-Key: rejected-fake-mime' \
  -F "file=@$fake_image_upload_path;filename=disfraz.png;type=image/png" \
  "$api_url/api/v1/jobs")"
assert_problem "$fake_status" 422 validation_error

large_status="$(curl -sS -D "$problem_headers" -o "$problem_body" -w '%{http_code}' \
  -H "Authorization: Bearer $token" \
  -H 'Idempotency-Key: rejected-large-file' \
  -F "file=@$large_upload_path;filename=grande.png;type=image/png" \
  "$api_url/api/v1/jobs")"
assert_problem "$large_status" 422 validation_error

excessive_pdf_status="$(curl -sS -D "$problem_headers" -o "$problem_body" -w '%{http_code}' \
  -H "Authorization: Bearer $token" \
  -H 'Idempotency-Key: rejected-page-limit' \
  -F "file=@$excessive_pdf_upload_path;type=application/pdf" \
  "$api_url/api/v1/jobs")"
assert_problem "$excessive_pdf_status" 422 validation_error

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
assert {item["name"] for item in detail["outputs"]} == {
    "metadata.json", "page-1.png", "page-2.png", "page-3.png"
}, detail
' <<<"$pdf_detail"
done

# Los rechazos anteriores no consumen cuota y el upload idempotente cuenta una sola vez.
# Después de una imagen y dos PDFs quedan dos lugares en la cuota horaria de cinco.
for number in 4 5; do
  quota_status="$(curl -sS -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer $token" \
    -H "Idempotency-Key: smoke-quota-$number" \
    -F "file=@$image_upload_path;type=image/png" \
    "$api_url/api/v1/jobs")"
  test "$quota_status" = '202'
done

rate_status="$(curl -sS -D "$problem_headers" -o "$problem_body" -w '%{http_code}' \
  -H "Authorization: Bearer $token" \
  -H 'Idempotency-Key: smoke-quota-6' \
  -F "file=@$image_upload_path;type=image/png" \
  "$api_url/api/v1/jobs")"
assert_problem "$rate_status" 429 rate_limited

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
