# Runbook: Pseudonym Key Compromise

## Trigger

- Suspected or confirmed exposure of pseudonymization key material
- Alert: PseudonymKeyAccessAnomaly firing
- Security team notification

## Impact

- **CRITICAL**: All pseudonymized data potentially reversible
- GDPR Article 33 notification required (72 hours)
- All tenants affected if master key compromised
- Audit chain integrity maintained (keys not stored in chain)

## Immediate Response (First 15 minutes)

```bash
# 1. Rotate active key immediately
# Generate new key
NEW_KEY=$(openssl rand -hex 32)

# 2. Update Vault / ExternalSecret
# In Vault UI or CLI:
vault kv put secret/hacienda/pseudonym/active key=$NEW_KEY
vault kv put secret/hacienda/pseudonym/key_$(date +%s) key=$OLD_KEY  # Archive old

# 3. Force ExternalSecret refresh
kubectl annotate externalsecret hacienda-pseudonym -n hacienda-prod   force-sync=$(date +%s) --overwrite

# 4. Restart API pods to pick up new key
kubectl rollout restart deployment/hacienda-api -n hacienda-prod
```

## Assessment

| Scope | Action |
|-------|--------|
| Single tenant key | Rotate that tenant's key only |
| Master/active key | Rotate ALL tenant keys (see below) |
| Key in git history | Rewrite history; rotate all |
| Key in logs | Rotate all; scrub logs |

## Full Key Rotation (If Master Key Compromised)

```bash
# 1. Generate new keys for ALL tenants
for tenant in $(kubectl exec -n hacienda-prod <pg-pod> -- psql -t -c "SELECT id FROM tenants;"); do
  NEW_KEY=$(openssl rand -hex 32)
  vault kv put secret/hacienda/pseudonym/tenant_${tenant} key=$NEW_KEY
  # Update tenant record with new key_id
  kubectl exec -n hacienda-prod <pg-pod> -- psql -c \
    "UPDATE tenants SET pseudonym_key_id = 'key_$(date +%s)_${tenant}' WHERE id = '${tenant}';";
done

# 2. Re-pseudonymize all existing data (BACKGROUND JOB)
# This is expensive - schedule during low traffic
hacienda pii re-pseudonymize --all-tenants --key-rotation

# 3. Verify
- Check audit chain: hacienda audit verify
- Spot-check: hacienda pii reveal <token> (should fail with old key)
```

## GDPR Notification

```text
Subject: Personal Data Breach Notification - Pseudonymization Key Exposure

Controller: [Company Name]
DPO: [DPO Contact]
Date of Breach: [Date]
Date of Discovery: [Date]

Categories of Data: Pseudonymized personal data (reversible with compromised key)
Number of Data Subjects: [Count per tenant]

Likely Consequences: Re-identification of pseudonymized data
Measures Taken: Key rotation, re-pseudonymization, enhanced monitoring

Contact: [Security Team Contact]
```

## Escalation

- **Immediate**: Security lead, DPO, Legal
- **1 hour**: CISO, CEO notification
- **24 hours**: Regulatory notification (if required)
- **72 hours**: GDPR Article 33 notification complete

## Prevention

- HSM-backed Vault (never store keys in config/env)
- Quarterly rotation drills
- Audit key access (Vault audit logs)
- Separate keys per tenant
- Key hierarchy: master encrypts tenant keys
