# Vault Secret Structure for hacienda

## Structure

```
secret/
  hacienda/
    prod/
      database/
        url                           # postgresql://user:pass@host:5432/db?sslmode=require
      pseudonym/
        active_key                    # Current pseudonymization key (hex)
        fpe_key                       # FPE key for deterministic encryption (hex)
        key_<timestamp>_<tenant>      # Archived keys (never delete)
      s3/
        access_key                    # AWS/GCS access key
        secret_key                    # AWS/GCS secret key
        endpoint                      # S3 endpoint (optional, for MinIO)
        region                        # S3 region
        bucket                        # Bucket name
      auth/
        jwt_secret                    # JWT signing secret (base64, 256-bit)
        api_key_salt                  # Argon2id salt for API key hashing
      redis/
        url                           # redis://:password@host:6379
    staging/
      ... (same structure)
    dev/
      ... (same structure)
```

## Key Rotation

- **Active key**: Rotated quarterly via `rotate-pseudonym-key.sh`
- **Archived keys**: Prefixed with timestamp and tenant, never deleted
- **FPE key**: Rotated annually (breaking change - requires re-encryption)
- **JWT secret**: Rotated annually
- **Database password**: Rotated quarterly
- **S3 credentials**: Rotated quarterly

## Access Control

| Role | Paths | Capabilities |
|------|-------|--------------|
| `hacienda-external-secrets` | `secret/data/hacienda/*` | read, list |
| `hacienda-admin` | `secret/data/hacienda/*`, `secret/metadata/hacienda/*` | create, read, update, delete, list |
| `hacienda-backup` | `secret/data/hacienda/*` | read, list |

## Initial Setup

```bash
# Run after Vault is initialized and unsealed
./setup-hacienda-secrets.sh
```