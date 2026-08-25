#!/bin/bash
set -euo pipefail

# Setup hacienda secrets in Vault
# Run after Vault is initialized, unsealed, and KV v2 engine enabled at 'secret/'

VAULT_ADDR=${VAULT_ADDR:-http://localhost:8200}
VAULT_TOKEN=${VAULT_TOKEN:-}

if [ -z "$VAULT_TOKEN" ]; then
  echo "ERROR: VAULT_TOKEN not set"
  exit 1
fi

echo "Setting up hacienda secret structure at $VAULT_ADDR"

# Enable KV v2 if not already enabled
if ! vault secrets list -format=json | jq -e '."secret/"' >/dev/null 2>&1; then
  echo "Enabling KV v2 secrets engine at secret/"
  vault secrets enable -path=secret kv-v2
else
  echo "KV v2 already enabled at secret/"
fi

# Function to create secret with placeholder
create_secret() {
  local path=$1
  shift
  echo "Creating $path"
  vault kv put "secret/$path" "$@"
}

# Production secrets (PLACEHOLDER VALUES - REPLACE BEFORE PRODUCTION)
echo "=== Creating Production Secrets ==="
create_secret hacienda/prod/database \
  url="postgresql://hacienda:CHANGE_ME@hacienda-postgres:5432/hacienda?sslmode=require"

create_secret hacienda/prod/pseudonym \
  active_key="$(openssl rand -hex 32)" \
  fpe_key="$(openssl rand -hex 32)"

create_secret hacienda/prod/s3 \
  access_key="CHANGE_ME" \
  secret_key="CHANGE_ME" \
  endpoint="https://s3.example.com" \
  region="us-east-1" \
  bucket="hacienda-prod"

create_secret hacienda/prod/auth \
  jwt_secret="$(openssl rand -base64 32)" \
  api_key_salt="$(openssl rand -base64 16)"

create_secret hacienda/prod/redis \
  url="redis://:CHANGE_ME@hacienda-redis:6379"

# Staging secrets
echo "=== Creating Staging Secrets ==="
create_secret hacienda/staging/database \
  url="postgresql://hacienda:CHANGE_ME@hacienda-postgres-staging:5432/hacienda?sslmode=require"

create_secret hacienda/staging/pseudonym \
  active_key="$(openssl rand -hex 32)" \
  fpe_key="$(openssl rand -hex 32)"

create_secret hacienda/staging/s3 \
  access_key="CHANGE_ME" \
  secret_key="CHANGE_ME" \
  endpoint="https://s3.example.com" \
  region="us-east-1" \
  bucket="hacienda-staging"

create_secret hacienda/staging/auth \
  jwt_secret="$(openssl rand -base64 32)" \
  api_key_salt="$(openssl rand -base64 16)"

create_secret hacienda/staging/redis \
  url="redis://:CHANGE_ME@hacienda-redis-staging:6379"

# Development secrets
echo "=== Creating Development Secrets ==="
create_secret hacienda/dev/database \
  url="postgresql://hacienda:devpass@localhost:5432/hacienda?sslmode=disable"

create_secret hacienda/dev/pseudonym \
  active_key="$(openssl rand -hex 32)" \
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
echo "IMPORTANT: Replace all CHANGE_ME values with real credentials before production use!"
echo ""
echo "Verify with:"
echo "  vault kv get secret/hacienda/prod/database"
echo "  vault kv get secret/hacienda/prod/pseudonym"
