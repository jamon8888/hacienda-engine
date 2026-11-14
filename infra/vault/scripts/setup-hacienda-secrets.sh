#!/bin/bash
set -euo pipefail

# Setup hacienda secrets in Vault
# Run after Vault is initialized, unsealed, and KV v2 engine enabled at 'secret/'
# Required environment variables:
#   HACIENDA_DB_PASSWORD_PROD
#   HACIENDA_DB_PASSWORD_STAGING
#   HACIENDA_S3_ACCESS_KEY_PROD
#   HACIENDA_S3_SECRET_KEY_PROD
#   HACIENDA_S3_ACCESS_KEY_STAGING
#   HACIENDA_S3_SECRET_KEY_STAGING
#   HACIENDA_REDIS_PASSWORD_PROD
#   HACIENDA_REDIS_PASSWORD_STAGING

VAULT_ADDR=${VAULT_ADDR:-http://localhost:8200}
VAULT_TOKEN=${VAULT_TOKEN:-}

if [ -z "$VAULT_TOKEN" ]; then
  echo "ERROR: VAULT_TOKEN not set"
  exit 1
fi

echo "Setting up hacienda secret structure at $VAULT_ADDR"

# Validate KV v2 mount at secret/
MOUNT_INFO=$(vault secrets list -format=json 2>/dev/null || echo "{}")
if ! echo "$MOUNT_INFO" | jq -e '."secret/" | select(.type=="kv" and .options.version=="2")' >/dev/null 2>&1; then
  if echo "$MOUNT_INFO" | jq -e '."secret/"' >/dev/null 2>&1; then
    echo "ERROR: secret/ exists but is not KV v2. Migrate or recreate it."
    exit 1
  else
    echo "Enabling KV v2 secrets engine at secret/"
    vault secrets enable -path=secret kv-v2
  fi
else
  echo "KV v2 already enabled at secret/"
fi

# Function to create secret with validation
create_secret() {
  local path=$1
  shift
  echo "Creating $path"
  vault kv put "secret/$path" "$@"
}

uri_encode() { jq -nr --arg value "$1" '$value|@uri'; }

# Validate required environment variables
validate_env() {
  local var=$1
  local val=${!var:-}
  if [ -z "$val" ] || [ "$val" = "CHANGE_ME" ]; then
    echo "ERROR: Required environment variable $var is not set or is CHANGE_ME"
    exit 1
  fi
}

# Production secrets
echo "=== Creating Production Secrets ==="
validate_env HACIENDA_DB_PASSWORD_PROD
validate_env HACIENDA_S3_ACCESS_KEY_PROD
validate_env HACIENDA_S3_SECRET_KEY_PROD
validate_env HACIENDA_REDIS_PASSWORD_PROD
DB_PASSWORD_PROD_URI=$(uri_encode "$HACIENDA_DB_PASSWORD_PROD")
REDIS_PASSWORD_PROD_URI=$(uri_encode "$HACIENDA_REDIS_PASSWORD_PROD")

create_secret hacienda/prod/database \
  url="postgresql://hacienda:${DB_PASSWORD_PROD_URI}@hacienda-postgres:5432/hacienda?sslmode=require"

create_secret hacienda/prod/pseudonym \
  active_key="key_1" \
  key_1="$(openssl rand -hex 32)" \
  fpe_key="$(openssl rand -hex 32)"

create_secret hacienda/prod/s3 \
  access_key="${HACIENDA_S3_ACCESS_KEY_PROD}" \
  secret_key="${HACIENDA_S3_SECRET_KEY_PROD}" \
  endpoint="https://storage.googleapis.com" \
  region="us-east-1" \
  bucket="hacienda-prod"

create_secret hacienda/prod/auth \
  jwt_secret="$(openssl rand -base64 32)" \
  api_key_salt="$(openssl rand -base64 16)"

create_secret hacienda/prod/redis \
  url="rediss://:${REDIS_PASSWORD_PROD_URI}@hacienda-redis:6378"

# Staging secrets
echo "=== Creating Staging Secrets ==="
validate_env HACIENDA_DB_PASSWORD_STAGING
validate_env HACIENDA_S3_ACCESS_KEY_STAGING
validate_env HACIENDA_S3_SECRET_KEY_STAGING
validate_env HACIENDA_REDIS_PASSWORD_STAGING
DB_PASSWORD_STAGING_URI=$(uri_encode "$HACIENDA_DB_PASSWORD_STAGING")
REDIS_PASSWORD_STAGING_URI=$(uri_encode "$HACIENDA_REDIS_PASSWORD_STAGING")

create_secret hacienda/staging/database \
  url="postgresql://hacienda:${DB_PASSWORD_STAGING_URI}@hacienda-postgres-staging:5432/hacienda?sslmode=require"

create_secret hacienda/staging/pseudonym \
  active_key="key_1" \
  key_1="$(openssl rand -hex 32)" \
  fpe_key="$(openssl rand -hex 32)"

create_secret hacienda/staging/s3 \
  access_key="${HACIENDA_S3_ACCESS_KEY_STAGING}" \
  secret_key="${HACIENDA_S3_SECRET_KEY_STAGING}" \
  endpoint="https://storage.googleapis.com" \
  region="us-east-1" \
  bucket="hacienda-staging"

create_secret hacienda/staging/auth \
  jwt_secret="$(openssl rand -base64 32)" \
  api_key_salt="$(openssl rand -base64 16)"

create_secret hacienda/staging/redis \
  url="rediss://:${REDIS_PASSWORD_STAGING_URI}@hacienda-redis-staging:6378"

# Development secrets (can use defaults)
echo "=== Creating Development Secrets ==="
create_secret hacienda/dev/database \
  url="postgresql://hacienda:devpass@localhost:5432/hacienda?sslmode=disable"

create_secret hacienda/dev/pseudonym \
  active_key="key_1" \
  key_1="$(openssl rand -hex 32)" \
  fpe_key="$(openssl rand -hex 32)"

create_secret hacienda/dev/s3 \
  access_key="minioadmin" \
  secret_key="minioadmin" \
  endpoint="http://minio:9000" \
  region="us-east-1" \
  bucket="hacienda-dev"

create_secret hacienda/dev/auth \
  jwt_secret="$(openssl rand -base64 32)" \
  api_key_salt="$(openssl rand -base64 16)"

create_secret hacienda/dev/redis \
  url="redis://:devpass@localhost:6379"

echo "=== Setup Complete ==="
echo "Verify with:"
echo "  vault kv get secret/hacienda/prod/database"
echo "  vault kv get secret/hacienda/prod/pseudonym"
