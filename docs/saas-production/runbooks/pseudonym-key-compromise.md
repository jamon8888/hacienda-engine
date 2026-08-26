# Runbook: Pseudonym Key Compromise

## Trigger

- Suspected or confirmed exposure of pseudonymization key material
- Alert: PseudonymKeyAccessAnomaly firing
- Security team notification

## Impact

- **CRITICAL**: All pseudonymized data potentially reversible
- GDPR Article 33 notification depends on a documented risk assessment to individuals; if required, the 72-hour deadline begins when the breach is recognized
- All tenants affected if master key compromised
- Audit chain integrity maintained (keys not stored in chain)

## Immediate Response (First 15 minutes)

```bash
# 1. Retrieve current active key (fail if not found)
OLD_KEY=$(vault kv get -field=active_key "secret/hacienda/prod/pseudonym" 2>/dev/null || {
  echo "ERROR: Failed to retrieve current active_key"
  exit 1
})

# 2. Archive old key
TIMESTAMP=$(date +%s)
ARCHIVE_KEY="key_${TIMESTAMP}"
echo "Archiving current key as $ARCHIVE_KEY"
vault kv put "secret/hacienda/prod/pseudonym/$ARCHIVE_KEY" key="$OLD_KEY"

# 3. Generate new key
NEW_KEY=$(openssl rand -hex 32)
echo "Generated new key"

# 4. Update active key (same path consumed by ExternalSecret)
vault kv put "secret/hacienda/prod/pseudonym" active_key="$NEW_KEY"
echo "Updated active_key"

# 5. Force ExternalSecrets sync
kubectl annotate externalsecret hacienda-secrets -n hacienda-prod \
  force-sync=$(date +%s) --overwrite

# 6. Verify the rendered secret changed before restart
sleep 5
if kubectl get secret hacienda-secrets -n hacienda-prod -o jsonpath='{.data.HACIENDA_PSEUDONYM_ACTIVE_KEY}' | base64 -d | grep -q "$NEW_KEY"; then
  echo "Secret verified with new key"
else
  echo "WARNING: Secret may not have updated yet"
fi

# 7. Restart API pods to pick up new key
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
  vault kv put secret/hacienda/prod/pseudonym/tenant_${tenant} key=$NEW_KEY
  # Update tenant record with new key_id
  kubectl exec -n hacienda-prod <pg-pod> -- psql -c \
    "UPDATE tenants SET pseudonym_key_id = 'key_$(date +%s)_${tenant}' WHERE id = '${tenant}';";
done

# 2. Update global active_key as well
vault kv put secret/hacienda/prod/pseudonym active_key="$(openssl rand -hex 32)"

# 4. Force ExternalSecrets sync & restart
kubectl annotate externalsecret hacienda-secrets -n hacienda-prod force-sync=$(date +%s) --overwrite
kubectl rollout restart deployment/hacienda-api -n hacienda-prod

# 5. Re-pseudonymize all existing data (BACKGROUND JOB)
# This is expensive - schedule during low traffic
hacienda pii re-pseudonymize --all-tenants --key-rotation

# 6. Verify
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

> **Note**: GDPR Article 33 notification is required only if the breach is likely to result in a risk to the rights and freedoms of individuals. A documented risk assessment must be performed first. If notification is required, the 72-hour deadline begins when the breach is recognized.

## Escalation

- **Immediate**: Security lead, DPO, Legal
- **1 hour**: CISO, CEO notification
- **24 hours**: Regulatory notification (if required)
- **72 hours**: GDPR Article 33 notification complete (if required)

## Prevention

- HSM-backed Vault (never store keys in config/env)
- Quarterly rotation drills
- Audit key access (Vault audit logs)
- Separate keys per tenant
- Key hierarchy: master encrypts tenant keys
