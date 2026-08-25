# API Versioning Specification

## Versioning Strategy

**URL Versioning**: All APIs are versioned in the URL path: `/v1/`, `/v2/`, etc.

## Version Lifecycle

| Stage | Description | Duration |
|-------|-------------|----------|
| **Alpha** | Internal testing, unstable | N/A |
| **Beta** | Limited external access, feedback | 3 months |
| **Stable** | General availability, SLA | 12 months minimum |
| **Deprecated** | Sunset header, migration period | 6 months |
| **Retired** | Removed, 410 Gone | After deprecation |

## Deprecation Process

1. **Announce**: Add `Sunset` header with RFC 7234 date
2. **Document**: Migration guide in `/docs/api/migration-v{X}-v{Y}.md`
3. **Support**: Both versions operational during overlap
4. **Remove**: After deprecation period, return 410 with `Link` header to new version

## Header Examples

```http
# Stable response
API-Version: v1

# Deprecated response
API-Version: v1
Sunset: Sat, 01 Jan 2026 00:00:00 GMT
Link: </v2/>; rel="successor-version"
Deprecation: true

# Retired response
410 Gone
Link: </v2/>; rel="successor-version"
```

## Breaking Changes (Require New Version)

- Removing/renaming fields in request/response
- Changing field types
- Removing endpoints
- Changing authentication requirements
- Changing error response format
- Changing pagination behavior

## Non-Breaking Changes (Same Version)

- Adding optional fields
- Adding new endpoints
- Adding new enum values (with `unknown` default)
- Improving performance
- Fixing bugs (non-behavioral)
- Adding new optional query parameters

## Client Compatibility

- SDKs pin to specific API version
- `User-Agent` header must include SDK version
- Server rejects requests without version in path
- Graceful degradation: unknown fields ignored

## Version Negotiation (Future)

```http
GET /v1/documents
Accept-Version: v1, v2

Response:
API-Version: v2
Vary: Accept-Version
```
