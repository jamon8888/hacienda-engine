//! API error envelope and mapping from core errors.
//!
//! Every handler returns `Result<impl IntoResponse, ApiError>`. `ApiError` serialises to
//!
//! ```json
//! {"error": {"code": "...", "message": "..."}}
//! ```
//!
//! The `message` field **must never** contain document content, a detected span, a
//! filename from the request, or a filesystem path. That detail belongs in
//! `tracing::error!` on the host side. The conversion from `HaciendaError` is the only
//! way a core error reaches the client, and it deliberately discards the internal
//! message, so a new core variant cannot accidentally reach the wire verbatim.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use hacienda_core::error::HaciendaError;
use serde::Serialize;
use thiserror::Error;

/// Machine-readable error code sent in the `code` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    InvalidRequest,
    Unauthenticated,
    Forbidden,
    NotFound,
    PayloadTooLarge,
    UnsupportedMediaType,
    Internal,
}

impl ApiErrorCode {
    fn status(self) -> StatusCode {
        match self {
            Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// The API error type returned by every handler.
///
/// Construct via the factory methods; the `message` is the client-visible sentence.
/// Never put document content or span text in `message`.
#[derive(Debug, Error)]
#[error("{code:?}: {message}")]
pub struct ApiError {
    pub code: ApiErrorCode,
    /// Short, generic, client-safe sentence. No PII, no paths, no spans.
    pub message: String,
}

impl ApiError {
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: ApiErrorCode::InvalidRequest,
            message: message.into(),
        }
    }

    pub fn unauthenticated() -> Self {
        Self {
            code: ApiErrorCode::Unauthenticated,
            message: "Authentication is required.".into(),
        }
    }

    pub fn forbidden() -> Self {
        Self {
            code: ApiErrorCode::Forbidden,
            message: "Insufficient permissions.".into(),
        }
    }

    pub fn not_found() -> Self {
        Self {
            code: ApiErrorCode::NotFound,
            message: "The requested resource was not found.".into(),
        }
    }

    pub fn payload_too_large() -> Self {
        Self {
            code: ApiErrorCode::PayloadTooLarge,
            message: "The request body exceeds the maximum allowed size.".into(),
        }
    }

    pub fn unsupported_media_type() -> Self {
        Self {
            code: ApiErrorCode::UnsupportedMediaType,
            message: "Expected a request with `content-type: application/json`.".into(),
        }
    }

    pub fn internal() -> Self {
        Self {
            code: ApiErrorCode::Internal,
            message: "An internal error occurred. Check host logs for details.".into(),
        }
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: ApiErrorCode,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.code.status();
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code,
                message: self.message,
            },
        };
        (status, Json(body)).into_response()
    }
}

/// Convert a core error into an API error.
///
/// This is the *only* path from `HaciendaError` to a client-visible response.
/// The internal error is logged here so the host can diagnose it; the client
/// receives a generic code-and-sentence pair that reveals nothing about the document.
impl From<HaciendaError> for ApiError {
    fn from(source: HaciendaError) -> Self {
        match &source {
            HaciendaError::Authz(authz_err) => {
                use hacienda_core::auth::AuthzError;
                match authz_err {
                    AuthzError::Unauthenticated => ApiError::unauthenticated(),
                    AuthzError::TokenExpired | AuthzError::InvalidToken(_) => {
                        ApiError::unauthenticated()
                    }
                    AuthzError::MissingCapability { .. } | AuthzError::UntrustedIssuer(_) => {
                        ApiError::forbidden()
                    }
                }
            }
            HaciendaError::PiiDisabled => {
                ApiError::invalid_request("PII detection is not enabled on this server.")
            }
            HaciendaError::Pseudonym(pseudonym_err) => {
                // Direct PseudonymError → HaciendaError::Pseudonym conversion.
                // All token errors are client faults → 400.
                use hacienda_core::redaction::PseudonymError;
                match pseudonym_err {
                    PseudonymError::MalformedToken
                    | PseudonymError::MalformedPadding
                    | PseudonymError::InvalidKeyId { .. }
                    | PseudonymError::KeyNotFound { .. }
                    | PseudonymError::NoActiveKey
                    | PseudonymError::MalformedKeyMaterial { .. }
                    | PseudonymError::WrongKeyLength { .. }
                    | PseudonymError::UnreadableToken
                    | PseudonymError::UnsupportedCategory { .. } => {
                        ApiError::invalid_request("Invalid pseudonym token.")
                    }
                }
            }
            HaciendaError::Pii(pii_err) => {
                // Map PII errors to appropriate client-visible codes.
                // Pseudonym errors (malformed token, unreadable, key not found) are
                // client faults → 400. Other PII errors are internal → 500.
                use hacienda_core::pii::PiiError;
                use hacienda_core::redaction::RedactionError;
                match pii_err {
                    PiiError::Redaction(RedactionError::Pseudonym(pseudonym_err)) => {
                        use hacienda_core::redaction::PseudonymError;
                        match pseudonym_err {
                            PseudonymError::MalformedToken
                            | PseudonymError::MalformedPadding
                            | PseudonymError::InvalidKeyId { .. }
                            | PseudonymError::KeyNotFound { .. }
                            | PseudonymError::NoActiveKey
                            | PseudonymError::MalformedKeyMaterial { .. }
                            | PseudonymError::WrongKeyLength { .. }
                            | PseudonymError::UnreadableToken
                            | PseudonymError::UnsupportedCategory { .. } => {
                                ApiError::invalid_request("Invalid pseudonym token.")
                            }
                        }
                    }
                    _ => {
                        tracing::error!(error = %source, "pii error processing request");
                        ApiError::internal()
                    }
                }
            }
            HaciendaError::Extraction(inner) => {
                // Split extraction failures by fault. A corrupt PDF is the *client's*
                // input, and answering 500 both misleads the caller into retrying and
                // pages an operator for a document that was never going to parse.
                // Host-side faults (a missing OCR binary, a poisoned lock) stay 500 so
                // they keep showing up in error-rate alerts.
                //
                // The inner `message` is never forwarded: for `Parsing` and `Validation`
                // it routinely contains the filename and a fragment of the document.
                use xberg::XbergError;
                match inner.as_ref() {
                    XbergError::Parsing { .. } | XbergError::Validation { .. } => {
                        ApiError::invalid_request(
                            "The document could not be parsed. Check that the bytes and \
                             the declared MIME type match.",
                        )
                    }
                    XbergError::UnsupportedFormat(_) => ApiError::invalid_request(
                        "The declared MIME type is not supported by this server.",
                    ),
                    XbergError::Security { .. } => ApiError::invalid_request(
                        "The document was refused by the extraction safety limits.",
                    ),
                    _ => {
                        tracing::error!(error = %source, "extraction failed");
                        ApiError::internal()
                    }
                }
            }
            _ => {
                // Log the full error on the host; the client gets a generic sentence.
                tracing::error!(error = %source, "internal error processing request");
                ApiError::internal()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;

    /// A rejected request body must never be echoed back to the client.
    ///
    /// This drives the **full router**, not a hand-built `ApiError`. Asserting that
    /// `ApiError::internal()` does not contain a secret proves nothing — by construction
    /// it cannot. The disclosure risk lives in axum's own extractor rejections, which
    /// quote the offending input; the request below is shaped to trigger exactly that
    /// path (a well-formed JSON body whose field has the wrong type), with a
    /// recognisable client name and IBAN in the offending position.
    #[tokio::test]
    async fn rejected_body_is_not_echoed_back_to_the_client() {
        use crate::routes::build_router;
        use crate::routes::tests::test_state_no_auth;
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;

        let secret = "Jean-Pierre Dupont, IBAN FR7630006000011234567890189";
        // `documents` must be an array; giving it a string makes serde report the
        // offending value verbatim in its own rejection text.
        let body = format!(r#"{{"documents":"{secret}"}}"#);

        let response = build_router(test_state_no_auth())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/documents")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "a malformed body must be a client error"
        );

        let body_bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let body_text = std::str::from_utf8(&body_bytes).unwrap();

        assert!(
            !body_text.contains("Jean-Pierre"),
            "error response echoed request content back to the client: {body_text}"
        );
        assert!(
            !body_text.contains("FR7630006000011234567890189"),
            "error response echoed request content back to the client: {body_text}"
        );
        assert!(
            body_text.contains("\"code\":\"invalid_request\""),
            "rejection must use the error envelope, got: {body_text}"
        );
    }

    /// A rejection that arrives as `text/plain` has bypassed the envelope, which means it
    /// also bypassed the message sanitisation above. Assert the content type explicitly.
    #[tokio::test]
    async fn extractor_rejections_use_the_json_envelope() {
        use crate::routes::build_router;
        use crate::routes::tests::test_state_no_auth;
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;

        let response = build_router(test_state_no_auth())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pii/redact")
                    .header("content-type", "application/json")
                    .body(Body::from("{not json"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(
            content_type.starts_with("application/json"),
            "rejections must render as the JSON error envelope, got {content_type}"
        );
    }

    /// A body missing `content-type: application/json` must be 415, not 400 — and must
    /// still use the envelope.
    #[tokio::test]
    async fn missing_json_content_type_yields_415() {
        use crate::routes::build_router;
        use crate::routes::tests::test_state_no_auth;
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;

        let response = build_router(test_state_no_auth())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pii/redact")
                    .body(Body::from(r#"{"text":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn invalid_request_yields_400() {
        let err = ApiError::invalid_request("batch too large");
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn forbidden_yields_403() {
        let err = ApiError::forbidden();
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn payload_too_large_yields_413() {
        let err = ApiError::payload_too_large();
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn hacienda_authz_maps_to_403() {
        use hacienda_core::auth::{AuthzError, Capability};
        let source = HaciendaError::Authz(AuthzError::MissingCapability {
            required: Capability::PiiReveal,
            principal: "user-1".into(),
        });
        let api_err = ApiError::from(source);
        assert_eq!(api_err.code, ApiErrorCode::Forbidden);
    }

    #[tokio::test]
    async fn hacienda_unauthenticated_maps_to_401() {
        use hacienda_core::auth::AuthzError;
        let source = HaciendaError::Authz(AuthzError::Unauthenticated);
        let api_err = ApiError::from(source);
        assert_eq!(api_err.code, ApiErrorCode::Unauthenticated);
    }

    #[tokio::test]
    async fn hacienda_pii_pseudonym_maps_to_400() {
        use hacienda_core::pii::PiiError;
        use hacienda_core::redaction::{PseudonymError, RedactionError};
        let source = HaciendaError::Pii(PiiError::Redaction(RedactionError::Pseudonym(
            PseudonymError::MalformedToken,
        )));
        let api_err = ApiError::from(source);
        assert_eq!(api_err.code, ApiErrorCode::InvalidRequest);
    }
}
