# Operational Runbooks Index

## Quick Reference

| Runbook | Trigger | Severity | RTO | Owner |
|---------|---------|----------|-----|-------|
| [api-down.md](api-down.md) | `/health` failing | Critical | 15 min | Platform |
| [db-failover.md](db-failover.md) | PG primary down | Critical | 5 min (auto) | Platform |
| [s3-outage.md](s3-outage.md) | S3 errors high | High | 30 min | Platform |
| [pseudonym-key-compromise.md](pseudonym-key-compromise.md) | Key exposure | Critical | 1 hour | Security |
| [audit-chain-corruption.md](audit-chain-corruption.md) | Verify fails | Critical | 4 hours | Backend |
| [tenant-data-deletion.md](tenant-data-deletion.md) | GDPR Art. 17 | High | 30 days | Backend/Legal |
| [capacity-exhaustion.md](capacity-exhaustion.md) | HPA at max | High | 15 min | Platform |
| [dependency-outage.md](dependency-outage.md) | Upstream down | Medium | 30 min | Backend |

## Incident Response Process

```
1. DETECT    → Alert fires (PagerDuty/Slack)
2. TRIAGE    → On-call acknowledges, checks runbook
3. DIAGNOSE  → Run diagnostic commands
4. MITIGATE  → Apply fix from runbook
5. VERIFY    → Confirm resolution
6. COMMUNICATE → Update status page, stakeholders
7. DOCUMENT  → Post-incident review within 48h
```

## Communication Channels

- **Critical**: PagerDuty → On-call → Slack #incidents → Status page
- **High**: Slack #incidents → Email stakeholders
- **Medium**: Slack #ops → Next business day

## Status Page

- **Template**: Replace with deployed status-page URL (e.g., https://status.yourdomain.com)
- Updated within 5 min of incident detection
- Templates for common scenarios
- **Release check**: Verify placeholder replaced before production deployment

## Post-Incident Review

```markdown
# Incident Report: <title>

**Date**: <date>
**Duration**: <start> - <end>
**Severity**: <Critical/High/Medium>
**Impact**: <description>

## Timeline
- HH:MM - Detection
- HH:MM - Triage
- HH:MM - Mitigation
- HH:MM - Resolution

## Root Cause
<5 whys analysis>

## Action Items
- [ ] <action> - @owner - <due date>

## Lessons Learned
<what worked, what didn't>
```

## Runbook Maintenance

- Review quarterly
- Update after every incident
- Test during DR drills
- Version controlled in Git
