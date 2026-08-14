//! OpenAPI document derived from the route table's `#[utoipa::path]`-annotated handlers.
//!
//! Every handler referenced by [`ApiDoc::paths`] carries its own `#[utoipa::path]`
//! attribute declaring method, parameters, request/response schemas, and the
//! capability it requires (in its `description`). This file only assembles those
//! declarations into one document — it must not hand-describe a route itself, or the
//! two sources of truth (this file and the handler) can drift the same way the old
//! hand-built stub silently could.
//!
//! `GET /openapi.json` itself is deliberately not one of the listed paths: it is the
//! endpoint serving this document, not a member of the surface the document describes.
//!
//! The tests `openapi_path_set_equals_route_table_minus_openapi_json` and
//! `every_openapi_path_has_at_least_one_typed_operation` are the guard against a new
//! route landing in [`crate::routes::ROUTE_TABLE`] without a corresponding
//! `#[utoipa::path]` — the failure mode this module replaces (a path present with no
//! method, no schema, no operation at all).

use axum::{extract::State, Json};
use serde_json::Value;
use utoipa::OpenApi;

use crate::state::ApiState;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "hacienda API",
        description = "PII redaction and compliance API. Document content never leaves the host.",
    ),
    paths(
        crate::handlers::info::health,
        crate::handlers::info::version,
        crate::handlers::info::info,
        crate::handlers::documents::process_documents,
        crate::handlers::documents::process_documents_async,
        crate::handlers::pii::scan_text,
        crate::handlers::pii::redact_text,
        crate::handlers::pii::pii_config,
        crate::handlers::pii::reveal_token,
        crate::handlers::pii::mint_token,
        crate::handlers::keys::list_keys,
        crate::handlers::audit_review::get_audit,
        crate::handlers::audit::audit_verify,
        crate::handlers::audit::audit_entries,
        crate::handlers::audit::audit_seals,
        crate::handlers::audit::audit_export,
        crate::handlers::audit::audit_tip,
        crate::handlers::audit_review::get_review,
        crate::handlers::audit_review::decide_review,
        crate::handlers::audit_review::get_compliance_dpia,
        crate::handlers::audit_review::get_compliance_report,
        crate::handlers::audit_review::get_glossary,
        crate::handlers::auth::issue_key,
        crate::handlers::auth::revoke_key,
        crate::handlers::auth::get_auth_config,
        crate::handlers::auth::whoami,
        crate::handlers::jobs::get_job,
        crate::handlers::jobs::list_jobs,
        crate::handlers::jobs::get_job_result,
        crate::handlers::rag::create_collection,
        crate::handlers::rag::get_collection,
        crate::handlers::rag::delete_collection,
        crate::handlers::rag::upsert_document,
        crate::handlers::rag::retrieve,
        crate::handlers::rag::list_collections,
        crate::handlers::rag::list_documents,
        crate::handlers::rag::migrate_embeddings,
        crate::handlers::rag::get_migrate_status,
        crate::handlers::rag_stream::answer,
        crate::handlers::presets::create_preset,
        crate::handlers::presets::list_presets,
        crate::handlers::presets::get_preset,
        crate::handlers::presets::delete_preset,
        crate::handlers::versions::list_document_versions,
        crate::handlers::versions::get_document,
        crate::handlers::versions::diff_document,
        crate::handlers::versions::get_diff_job,
        crate::handlers::uploads::presign_upload,
        crate::handlers::uploads::confirm_upload,
        crate::handlers::usage::get_usage,
    ),
    components(schemas(
        crate::dto::DocumentInput,
        crate::dto::ProcessDocumentsRequest,
        crate::dto::ScanTextRequest,
        crate::dto::RedactTextRequest,
        crate::dto::RevealTokenRequest,
        crate::dto::MintTokenRequest,
        crate::dto::MintTokenResponse,
        crate::dto::KeyStatusDto,
        crate::dto::KeyListResponse,
        crate::dto::ReviewDecideRequest,
        crate::dto::IssueKeyRequest,
        crate::dto::EntityDto,
        crate::dto::ScanTextResponse,
        crate::dto::RedactTextResponse,
        crate::dto::RevealTokenResponse,
        crate::dto::DocumentResult,
        crate::dto::ProcessDocumentsResponse,
        crate::dto::AsyncJobResponse,
        crate::dto::JobResponse,
        crate::dto::JobListQuery,
        crate::dto::JobListResponse,
        crate::dto::JobResultResponse,
        crate::dto::AuditEntryDto,
        crate::dto::ReviewItemDto,
        crate::dto::AuditResponse,
        crate::dto::AuditScope,
        crate::dto::NodeAuditEntryDto,
        crate::dto::NodeAuditPage,
        crate::dto::SegmentSealDto,
        crate::dto::NodeSealsResponse,
        crate::dto::AuditTipResponse,
        crate::dto::VerifyFailureKind,
        crate::dto::VerifyFailure,
        crate::dto::VerifyResponse,
        crate::dto::ReviewResponse,
        crate::dto::ReviewDecideResponse,
        crate::dto::ReviewDecisionWire,
        crate::dto::ComplianceDpiaResponse,
        crate::dto::ComplianceReportResponse,
        crate::dto::GlossaryResponse,
        crate::dto::IssueKeyResponse,
        crate::dto::AuthConfigResponse,
        crate::dto::WhoamiResponse,
        crate::dto::PiiConfigResponse,
        crate::dto::HealthResponse,
        crate::dto::VersionResponse,
        crate::dto::InfoResponse,
        crate::dto::UpsertDocumentRequest,
        crate::dto::UpsertDocumentResponse,
        crate::dto::RagListQuery,
        crate::dto::ListCollectionsResponse,
        crate::dto::ListDocumentsResponse,
        crate::dto::MigrateEmbeddingsRequest,
        crate::dto::MigrateEmbeddingsResponse,
        crate::dto::MigrateProgressDto,
        crate::dto::MigrateStatusResponse,
        crate::dto::AnswerRequest,
        crate::dto::CreatePresetRequest,
        crate::dto::PresetResponse,
        crate::dto::PresetListResponse,
        crate::dto::DocumentVersionSummaryDto,
        crate::dto::DocumentVersionListResponse,
        crate::dto::DocumentEnvelopeResponse,
        crate::dto::DocumentDiffQuery,
        crate::dto::DiffLineDto,
        crate::dto::DocumentDiffResponse,
        crate::dto::DiffJobAcceptedResponse,
        crate::dto::DiffJobResultResponse,
        crate::dto::PresignUploadRequest,
        crate::dto::PresignUploadResponse,
        crate::dto::ConfirmUploadRequest,
        crate::dto::ConfirmUploadResponse,
        crate::dto::UsageQuery,
        crate::dto::UsageRecordDto,
        crate::dto::UsageResponse,
    )),
    modifiers(&SecuritySchemeModifier)
)]
struct ApiDoc;

/// Registers the `bearerAuth` security scheme used by every non-public
/// `#[utoipa::path(security(("bearerAuth" = [])))]` operation above.
struct SecuritySchemeModifier;

impl utoipa::Modify for SecuritySchemeModifier {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::default);
        components.add_security_scheme(
            "bearerAuth",
            SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
        );
    }
}

/// `GET /openapi.json` — the OpenAPI 3.1 document for the entire API surface, assembled
/// from every handler's `#[utoipa::path]` annotation (see this module's doc comment).
/// Public: a client must be able to fetch the schema before it has a credential to
/// authenticate the routes the schema describes.
pub async fn openapi(_: State<ApiState>) -> Result<Json<Value>, crate::error::ApiError> {
    Ok(Json(build_openapi()?))
}

/// Build the OpenAPI document from the `#[utoipa::path]`-annotated handlers.
///
/// Called at request time, not cached at startup, so a future route addition (a new
/// `#[utoipa::path]` entry added to [`ApiDoc::paths`] above) is always reflected —
/// mirroring the previous implementation's request-time-generation rationale.
///
/// # Errors
///
/// [`crate::error::ApiError::internal`] if `ApiDoc::openapi()`'s output cannot be
/// serialised to JSON. Not expected to happen in practice — the value is built entirely
/// from static `#[utoipa::path]`/`#[derive(ToSchema)]` metadata, not request input — but
/// this crate's convention is to propagate a `Result` rather than panic on a fallible
/// call, even one this unlikely to fail.
pub fn build_openapi() -> Result<Value, crate::error::ApiError> {
    serde_json::to_value(ApiDoc::openapi()).map_err(|e| {
        tracing::error!(error = %e, "failed to serialise the OpenAPI document");
        crate::error::ApiError::internal()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::ROUTE_TABLE;

    /// The path set in the OpenAPI document must equal the path set in the route
    /// table, with the sole deliberate exception of `/openapi.json` itself (see this
    /// module's doc comment). These are two independently maintained lists — this
    /// crate has no mechanism to derive one from the other — so a route added to
    /// `ROUTE_TABLE` without a matching `#[utoipa::path]` entry in [`ApiDoc`] fails
    /// this test rather than shipping silently undocumented.
    #[test]
    fn openapi_path_set_equals_route_table_minus_openapi_json() {
        let doc = build_openapi().expect("static ApiDoc metadata always serialises");
        let doc_paths: std::collections::HashSet<String> = doc["paths"]
            .as_object()
            .expect("paths must be an object")
            .keys()
            .cloned()
            .collect();

        let table_paths: std::collections::HashSet<String> = ROUTE_TABLE
            .iter()
            .map(|s| s.path.to_string())
            .filter(|p| p != "/openapi.json")
            .collect();

        assert_eq!(
            doc_paths, table_paths,
            "OpenAPI path set diverged from ROUTE_TABLE (excluding /openapi.json) — \
             a route was added without a matching #[utoipa::path] handler annotation, \
             or vice versa"
        );
    }

    /// Every path in the document must have at least one HTTP method with a described
    /// request/response shape — the failure mode of the old hand-built stub, where
    /// `paths` existed but carried only a free-text `description`, no `get`/`post`/…
    /// keys, and nothing an OpenAPI codegen tool could act on.
    #[test]
    fn every_openapi_path_has_at_least_one_typed_operation() {
        const HTTP_METHODS: &[&str] = &[
            "get", "post", "put", "delete", "patch", "options", "head", "trace",
        ];

        let doc = build_openapi().expect("static ApiDoc metadata always serialises");
        let paths = doc["paths"].as_object().expect("paths must be an object");

        for (path, path_item) in paths {
            let item = path_item.as_object().unwrap_or_else(|| {
                panic!("path {path} has a non-object path item — no operations at all")
            });
            let has_operation = HTTP_METHODS.iter().any(|m| item.contains_key(*m));
            assert!(
                has_operation,
                "path {path} has no get/post/put/delete/... operation — \
                 it is a stub entry, not a described operation"
            );

            for method in HTTP_METHODS {
                let Some(operation) = item.get(*method) else {
                    continue;
                };
                assert!(
                    operation
                        .get("responses")
                        .is_some_and(|r| r.is_object() && !r.as_object().unwrap().is_empty()),
                    "path {path} method {method} has no typed responses"
                );
            }
        }
    }

    /// The exact set of DTO names declared in [`ApiDoc`]'s `components(schemas(...))`
    /// list above — kept in sync with that list by hand, since `utoipa`'s derive macro
    /// gives no way to enumerate it at compile time. A name added to one list and not
    /// the other fails this test, in either direction.
    const DECLARED_SCHEMA_NAMES: &[&str] = &[
        "DocumentInput",
        "ProcessDocumentsRequest",
        "ScanTextRequest",
        "RedactTextRequest",
        "RevealTokenRequest",
        "MintTokenRequest",
        "MintTokenResponse",
        "KeyStatusDto",
        "KeyListResponse",
        "ReviewDecideRequest",
        "IssueKeyRequest",
        "EntityDto",
        "ScanTextResponse",
        "RedactTextResponse",
        "RevealTokenResponse",
        "DocumentResult",
        "ProcessDocumentsResponse",
        "AsyncJobResponse",
        "JobResponse",
        "JobListQuery",
        "JobListResponse",
        "JobResultResponse",
        "AuditEntryDto",
        "ReviewItemDto",
        "AuditResponse",
        "AuditScope",
        "NodeAuditEntryDto",
        "NodeAuditPage",
        "SegmentSealDto",
        "NodeSealsResponse",
        "AuditTipResponse",
        "VerifyFailureKind",
        "VerifyFailure",
        "VerifyResponse",
        "ReviewResponse",
        "ReviewDecideResponse",
        "ReviewDecisionWire",
        "ComplianceDpiaResponse",
        "ComplianceReportResponse",
        "GlossaryResponse",
        "IssueKeyResponse",
        "AuthConfigResponse",
        "WhoamiResponse",
        "PiiConfigResponse",
        "HealthResponse",
        "VersionResponse",
        "InfoResponse",
        "UpsertDocumentRequest",
        "UpsertDocumentResponse",
        "RagListQuery",
        "ListCollectionsResponse",
        "ListDocumentsResponse",
        "MigrateEmbeddingsRequest",
        "MigrateEmbeddingsResponse",
        "MigrateProgressDto",
        "MigrateStatusResponse",
        "AnswerRequest",
        "CreatePresetRequest",
        "PresetResponse",
        "PresetListResponse",
        "DocumentVersionSummaryDto",
        "DocumentVersionListResponse",
        "DocumentEnvelopeResponse",
        "DocumentDiffQuery",
        "DiffLineDto",
        "DocumentDiffResponse",
        "DiffJobAcceptedResponse",
        "DiffJobResultResponse",
        "PresignUploadRequest",
        "PresignUploadResponse",
        "ConfirmUploadRequest",
        "ConfirmUploadResponse",
        "UsageQuery",
        "UsageRecordDto",
        "UsageResponse",
    ];

    /// Every schema referenced by `components(schemas(...))` must actually appear
    /// under `components.schemas` in the built document — catches a `ToSchema` that
    /// resolves to nothing (e.g. a bare type alias with no schema of its own). Also
    /// catches the opposite drift: a schema present in the document but absent from
    /// [`DECLARED_SCHEMA_NAMES`], meaning this test's own list fell behind [`ApiDoc`].
    #[test]
    fn every_declared_schema_is_present_in_components() {
        let doc = build_openapi().expect("static ApiDoc metadata always serialises");
        let schemas = doc["components"]["schemas"]
            .as_object()
            .expect("components.schemas must be an object");

        for name in DECLARED_SCHEMA_NAMES {
            assert!(
                schemas.contains_key(*name),
                "declared schema {name} is missing from components.schemas — \
                 its ToSchema derive resolved to nothing, or it was removed from \
                 dto.rs without being removed from ApiDoc's components(schemas(...))"
            );
        }

        let declared: std::collections::HashSet<&str> =
            DECLARED_SCHEMA_NAMES.iter().copied().collect();
        for name in schemas.keys() {
            assert!(
                declared.contains(name.as_str()),
                "schema {name} is present in components.schemas but missing from \
                 this test's DECLARED_SCHEMA_NAMES — add it there to keep this test \
                 an exact set comparison, not just a one-directional check"
            );
        }
    }

    /// `/v1/audit/export` returns `Vec<u8>` at the Rust type level for all three
    /// `?format=` values, which on its own erases the distinction between them. The
    /// document must still describe each format's actual content type and body shape
    /// (`application/json`, `application/x-ndjson`, `text/csv`) rather than only the
    /// generic byte response — see the handler's own doc comment for why the formats
    /// are not interchangeable.
    #[test]
    fn audit_export_response_documents_every_format_content_type() {
        let doc = build_openapi().expect("static ApiDoc metadata always serialises");
        let content = doc["paths"]["/v1/audit/export"]["get"]["responses"]["200"]["content"]
            .as_object()
            .expect("/v1/audit/export's 200 response must declare a content map");

        // Exact set, not just "contains" — an extra or misspelled content type would
        // pass a `contains_key`-only check just as easily as a missing one.
        let actual: std::collections::BTreeSet<&str> = content.keys().map(String::as_str).collect();
        let expected: std::collections::BTreeSet<&str> =
            ["application/json", "application/x-ndjson", "text/csv"].into();
        assert_eq!(
            actual, expected,
            "/v1/audit/export's 200 response content types must be exactly the three \
             ?format= values, no more, no fewer"
        );

        // The shape matters as much as the presence: json is an array of entries,
        // jsonl/csv are opaque strings (see this handler's own doc comment on why
        // jsonl/csv have no structured schema).
        assert_eq!(content["application/json"]["schema"]["type"], "array");
        assert_eq!(
            content["application/json"]["schema"]["items"]["$ref"],
            "#/components/schemas/NodeAuditEntryDto"
        );
        assert_eq!(content["application/x-ndjson"]["schema"]["type"], "string");
        assert_eq!(content["text/csv"]["schema"]["type"], "string");
    }
}
