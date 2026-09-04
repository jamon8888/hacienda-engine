# Secrets Management Specification

## Architecture

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│   Vault         │────▶│  ExternalSecrets │────▶│  Kubernetes     │
│  (Source of     │     │  Operator        │     │  Secrets        │
│   Truth)        │     │  (Sync Controller)│    │  (Consumers)    │
└─────────────────┘     └──────────────────┘     └─────────────────┘
```

## Secret Categories

| Category | Secrets | Rotation | Access |
|----------|---------|----------|--------|
| **Database** | `DATABASE_URL` | Quarterly | API pods |
| **Pseudonymization** | Active key, FPE key, key hierarchy | Quarterly | API pods |
| **Object Storage** | S3 access/secret keys | Quarterly | API pods |
| **Authentication** | JWT secret, API key salt | Annual | API pods |
| **Redis** | Redis password/URL | Quarterly | API pods |
| **Monitoring** | Grafana admin, Alertmanager | Annual | Monitoring stack |
| **TLS** | Cert/key (cert-manager) | Auto (90 days) | Ingress |

## Vault Structure

```
secret/
  hacienda/
    prod/
      database/
        url
      pseudonym/
        active_key
        fpe_key
        key_<timestamp>_<tenant>  # Archived keys
      s3/
        access_key
        secret_key
      auth/
        jwt_secret
        api_key_salt
      redis/
        url
    staging/
      ... (same structure)
    dev/
      ... (same structure)
```

## ExternalSecrets Configuration

```yaml
apiVersion: external-secrets.io/v1beta1
kind: ClusterSecretStore
metadata:
  name: vault-backend
spec:
  provider:
    vault:
      server: "https://vault.example.com"
      path: "secret"
      version: "v2"
      auth:
        kubernetes:
          mountPath: "kubernetes"
          role: "hacienda-external-secrets"
---
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata:
  name: hacienda-secrets
  namespace: hacienda-prod
spec:
  refreshInterval: 1h
  secretStoreRef:
    name: vault-backend
    kind: ClusterSecretStore
  target:
    name: hacienda-secrets
    creationPolicy: Owner
  data:
    - secretKey: database-url
      remoteRef:
        key: hacienda/prod/database
        property: url
    - secretKey: pseudonym-active-key
      remoteRef:
        key: hacienda/prod/pseudonym
        property: active_key
    # ... etc
```

## Rotation Procedures

### Quarterly Rotation (Automated)

```bash
# 1. Generate new secrets
NEW_DB_PASS=$(openssl rand -base64 32)
NEW_PSEUDO_KEY=$(openssl rand -hex 32)
NEW_S3_KEY=$(aws iam create-access-key --user-name hacienda-prod)

# 2. Update Vault
vault kv put secret/hacienda/prod/database url="postgresql://user:$NEW_DB_PASS@..."
vault kv put secret/hacienda/prod/pseudonym active_key=$NEW_PSEUDO_KEY
vault kv put secret/hacienda/prod/s3 access_key=$NEW_S3_KEY secret_key=$NEW_S3_SECRET

# 3. ExternalSecrets auto-syncs within 1h
# 4. Force sync if needed
kubectl annotate externalsecret hacienda-secrets -n hacienda-prod \
  force-sync=$(date +%s) --overwrite

# 5. Rollout restart to pick up new secrets
kubectl rollout restart deployment/hacienda-api -n hacienda-prod
```

### Emergency Rotation (Incident Response)

```bash
# 1. Immediately revoke compromised credentials
# 2. Generate replacements
# 3. Update Vault
# 4. Force sync + restart (5 min max)
# 5. Verify application health
# 6. Audit access logs for misuse
```

## Key Hierarchy (Pseudonymization)

```
Master Key (HSM)
    │
    ├── Tenant A Key (encrypted)
    │   └── Document Keys (derived)
    │
    ├── Tenant B Key (encrypted)
    │   └── Document Keys (derived)
    │
    └── Tenant C Key (encrypted)
        └── Document Keys (derived)
```

- Master key never leaves HSM
- Tenant keys encrypted with master, stored in Vault
- Document keys derived per-operation (HKDF)
- Compromise of one tenant key ≠ others

## Access Control

```hcl
# Vault policy for hacienda-external-secrets role
path "secret/data/hacienda/prod/*" {
  capabilities = ["read", "list"]
}

path "secret/metadata/hacienda/prod/*" {
  capabilities = ["list"]
}
```

## Audit

- All secret access logged in Vault audit log
- Alert on anomalous access patterns
- Quarterly access review

## Disaster Recovery

- Vault backup: daily snapshots to separate region
- Seal/unseal procedure documented
- Root token split (Shamir) stored offline
- Recovery time objective: < 1 hour
