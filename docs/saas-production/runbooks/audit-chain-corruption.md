# Runbook: Audit Chain Corruption

## Trigger

- `hacienda audit verify` fails
- Alert: `AuditChainVerificationFailed` firing
- Manual verification shows hash mismatch

## Impact

- **CRITICAL**: Tamper-evident property violated
- Legal/compliance implications (GDPR, AI Act)
- All audit data since corruption point suspect
- May require regulatory notification

## Diagnosis

```bash
# 1. Identify corrupted segment
cd /var/log/hacienda/audit
for node in */; do
  echo "=== $node ==="
  hacienda audit verify "$node" 2>&1 | head -20

done

# 2. Check specific segment
hacienda audit verify /var/log/hacienda/audit/node-1 --verbose

# 3. Check for disk issues
dmesg -T | grep -i error
smartctl -a /dev/nvme0n1

# 4. Verify backup integrity
aws s3 ls s3://hacienda-prod-audit/exports/
# Download latest export and verify
hacienda audit verify /tmp/audit-export-20260825
```

## Common Causes

| Cause | Indicators | Fix |
|-------|------------|-----|
| Disk corruption | SMART errors, dmesg | Replace disk, restore from backup |
| Concurrent writes | Multiple writers same node | Fix writer coordination |
| Bug in chain logic | Consistent offset error | Code fix + re-chain |
| Truncated write | Partial entry at end | Truncate, re-chain |
| Clock skew | Timestamp anomalies | NTP fix, re-chain |

## Recovery Procedures

### Scenario 1: Single Segment Corrupted (Recent)

```bash
# 1. Stop writers to affected node
kubectl scale deployment hacienda-api -n hacienda-prod --replicas=0

# 2. Restore from latest S3 export
aws s3 cp s3://hacienda-prod-audit/exports/latest/ /tmp/restore/ --recursive
hacienda audit verify /tmp/restore

# 3. Re-chain from restore point
# (Requires hacienda-core admin function)
cargo run --bin hacienda-admin -- rechain --from /tmp/restore --to /var/log/hacienda/audit

# 4. Verify full chain
hacienda audit verify /var/log/hacienda/audit

# 5. Restart writers
kubectl scale deployment hacienda-api -n hacienda-prod --replicas=3
```

### Scenario 2: Historical Corruption (Old Segment)

```bash
# 1. Isolate corrupted segment
mv /var/log/hacienda/audit/node-1/corrupted_segment /tmp/quarantine/

# 2. Verify rest of chain
hacienda audit verify /var/log/hacienda/audit

# 3. If rest valid, document gap
# 4. Notify compliance team
# 5. Gap recorded in compliance register
```

### Scenario 3: Widespread Corruption

```bash
# 1. EMERGENCY: Freeze all writes
kubectl scale deployment hacienda-api -n hacienda-prod --replicas=0
kubectl scale deployment hacienda-worker -n hacienda-prod --replicas=0

# 2. Full restore from S3
aws s3 cp s3://hacienda-prod-audit/exports/latest/ /var/log/hacienda/audit/ --recursive

# 3. Verify
hacienda audit verify /var/log/hacienda/audit

# 4. Re-process documents since last export (if needed)
# 5. Gradual restart with enhanced monitoring
```

## Escalation

- **Immediate**: Backend lead, Security lead, Compliance lead
- **1 hour**: CTO, Legal, DPO
- **4 hours**: Regulatory notification if required (GDPR 72h)

## Post-Incident

- Full chain verification
- Root cause analysis (5 whys)
- Update chain implementation if bug
- Improve monitoring (real-time verification)
- Document in compliance register

## Prevention

- Real-time verification sidecar (every 5 min)
- WAL-style writing (fsync after each entry)
- Single writer per node (enforced)
- Regular automated verify in CI
- Disk health monitoring
