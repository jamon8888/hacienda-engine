#!/bin/bash
set -euo pipefail

# Rotate pseudonymization active key
# Usage: ./rotate-pseudonym-key.sh <environment> [tenant_id]
#   environment: prod, staging, dev
#   tenant_id: optional, if provided rotates only that tenant's key

ENV=${1:-}
TENANT_ID=${2:-}

if [[ ! "$ENV" =~ ^(prod|staging|dev)$ ]]; then
  echo "Usage: $0 <prod|staging|dev> [tenant_id]"
  exit 1
fi

VAULT_ADDR=${VAULT_ADDR:-http://localhost:8200}
VAULT_TOKEN=${VAULT_TOKEN:-}

if [ -z "$VAULT_TOKEN" ]; then
  echo "ERROR: VAULT_TOKEN not set"
  exit 1
fi

echo "Rotating pseudonym key for $ENV environment${TENANT_ID:+ (tenant: $TENANT_ID)}"

# Get current active key (fail if not found)
OLD_KEY=$(vault kv get -field=active_key "secret/hacienda/$ENV/pseudonym" 2>/dev/null || {
  echo "ERROR: Failed to retrieve current active_key for $ENV"
  exit 1
})

# Archive old key
TIMESTAMP=$(date +%s)
ARCHIVE_KEY="key_${TIMESTAMP}${TENANT_ID:+_$TENANT_ID}"
echo "Archiving current key as $ARCHIVE_KEY"
vault kv put "secret/hacienda/$ENV/pseudonym/$ARCHIVE_KEY" key="$OLD_KEY"

# Generate new key
NEW_KEY=$(openssl rand -hex 32)
echo "Generated new key"

# Update active key (same path consumed by ExternalSecret)
if [ -n "$TENANT_ID" ]; then
  # For multi-tenant: update both the global active_key and tenant-specific key
  vault kv put "secret/hacienda/$ENV/pseudonym" active_key="$NEW_KEY"
  vault kv put "secret/hacienda/$ENV/pseudonym/tenant_$TENANT_ID" key="$NEW_KEY"
  echo "Updated active_key and tenant_$TENANT_ID key"
else
  # Single active key (legacy/single-tenant mode)
  vault kv put "secret/hacienda/$ENV/pseudonym" active_key="$NEW_KEY"
  echo "Updated active_key"
fi

# Force ExternalSecrets sync
if command -v kubectl >/dev/null 2>&1; then
  echo "Forcing ExternalSecrets sync..."
  kubectl annotate externalsecret hacienda-secrets -n hacienda-$ENV \
    force-sync=$(date +%s) --overwrite 2>/dev/null || echo "ExternalSecret not found, skipping"
  
  # Verify the rendered secret changed before restart
  echo "Verifying secret update..."
  sleep 5
  if kubectl get secret hacienda-secrets -n hacienda-$ENV -o jsonpath='{.data.HACIENDA_PSEUDONYM_ACTIVE_KEY}' | base64 -d | grep -q "$NEW_KEY"; then
    echo "Secret verified with new key"
  else
    echo "WARNING: Secret may not have updated yet"
  fi
  
  echo "Restarting hacienda-api pods to pick up new key..."
  kubectl rollout restart deployment/hacienda-api -n hacienda-$ENV 2>/dev/null || echo "Deployment not found, skipping"
fi

echo "=== Key Rotation Complete ==="
echo "New key active. Old key archived as $ARCHIVE_KEY"
echo ""
echo "Verify with:"
echo "  vault kv get secret/hacienda/$ENV/pseudonym"
echo "  hacienda pii reveal <token>  # Should fail with old key, work with new"
