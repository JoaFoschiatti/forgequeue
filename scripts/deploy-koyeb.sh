#!/usr/bin/env bash
set -euo pipefail

app_name="${KOYEB_APP_NAME:-forgequeue}"
region="${KOYEB_REGION:-was}"
repository="${GITHUB_REPOSITORY:-JoaFoschiatti/forgequeue}"
bucket="${S3_BUCKET:-forgequeue}"
secret_prefix="${KOYEB_SECRET_PREFIX:-forgequeue}"

required=(DATABASE_URL S3_ENDPOINT S3_REGION S3_ACCESS_KEY_ID S3_SECRET_ACCESS_KEY CORS_ORIGIN RATE_LIMIT_SALT)
for variable in "${required[@]}"; do
  if [[ -z "${!variable:-}" ]]; then
    echo "Falta la variable requerida $variable." >&2
    exit 2
  fi
done

if ! command -v koyeb >/dev/null 2>&1; then
  echo "Instalá y autenticá Koyeb CLI antes de ejecutar este script." >&2
  exit 2
fi

case "$region" in
  was|fra) ;;
  *)
    echo "La instancia gratuita sólo admite KOYEB_REGION=was o fra." >&2
    exit 2
    ;;
esac

upsert_secret() {
  local name="$1"
  local value="$2"
  if koyeb secrets get "$name" >/dev/null 2>&1; then
    printf '%s' "$value" | koyeb secrets update "$name" --value-from-stdin >/dev/null
  else
    printf '%s' "$value" | koyeb secrets create "$name" --value-from-stdin >/dev/null
  fi
}

database_secret="${secret_prefix}-database-url"
s3_access_secret="${secret_prefix}-s3-access-key"
s3_secret_secret="${secret_prefix}-s3-secret-key"
rate_salt_secret="${secret_prefix}-rate-limit-salt"

upsert_secret "$database_secret" "$DATABASE_URL"
upsert_secret "$s3_access_secret" "$S3_ACCESS_KEY_ID"
upsert_secret "$s3_secret_secret" "$S3_SECRET_ACCESS_KEY"
upsert_secret "$rate_salt_secret" "$RATE_LIMIT_SALT"

service="$app_name/$app_name"
source_args=(
  --git "github.com/$repository"
  --git-branch main
  --git-builder docker
  --git-docker-command all
)
runtime_args=(
  --checks "8080:http:/health/ready"
  --checks-grace-period "8080=180"
  --env "BIND_ADDRESS=0.0.0.0:8080"
  --env "DATABASE_URL={{secret.${database_secret}}}"
  --env "OBJECT_STORE_URL=s3://${bucket}"
  --env "S3_ENDPOINT=${S3_ENDPOINT}"
  --env "S3_REGION=${S3_REGION}"
  --env "S3_ACCESS_KEY_ID={{secret.${s3_access_secret}}}"
  --env "S3_SECRET_ACCESS_KEY={{secret.${s3_secret_secret}}}"
  --env "CORS_ORIGIN=${CORS_ORIGIN}"
  --env "TRUST_PROXY_HEADERS=true"
  --env "RATE_LIMIT_SALT={{secret.${rate_salt_secret}}}"
  --env "LOG_FORMAT=json"
  --env "RUST_LOG=forgequeue_server=info,tower_http=info"
  --wait
  --wait-timeout 20m
)

if koyeb services get "$service" >/dev/null 2>&1; then
  echo "Actualizando el servicio Koyeb existente $service..."
  koyeb services update "$service" \
    "${source_args[@]}" \
    "${runtime_args[@]}"
else
  echo "Creando el servicio Koyeb $service..."
  koyeb apps init "$app_name" \
    "${source_args[@]}" \
    --instance-type free \
    --regions "$region" \
    --ports "8080:http" \
    --routes "/:8080" \
    "${runtime_args[@]}"
fi

koyeb services get "$service"
