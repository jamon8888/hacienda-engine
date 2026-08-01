//! Request extractors that fail into the [`ApiError`] envelope.
//!
//! # Why not `axum::Json`
//!
//! `axum::Json`'s rejection renders itself as `text/plain` and its message *quotes the
//! offending input*. A body of `{"documents":[{"mime_type":42,...}]}` yields
//!
//! ```text
//! Failed to deserialize the JSON body into the target type: documents[0].mime_type:
//! invalid type: integer `42`, expected a string at line 1 column 40
//! ```
//!
//! On a product whose claim is that document content never leaves the host, a rejection
//! that echoes a fragment of the request body back to the client — and, via
//! `tracing`, into the host's logs — is a disclosure path that no handler can close,
//! because it fires before any handler runs.
//!
//! [`Json`] wraps the axum extractor and discards the rejection's text entirely,
//! substituting a constant sentence chosen by the rejection's status code.

use axum::{
    extract::{
        rejection::{JsonRejection, QueryRejection},
        FromRequest, FromRequestParts, Request,
    },
    http::{request::Parts, StatusCode},
};
use serde::de::DeserializeOwned;

use crate::error::ApiError;

/// JSON body extractor whose rejections are [`ApiError`]s.
///
/// Drop-in for `axum::Json` in argument position.
#[derive(Debug, Clone, Copy, Default)]
pub struct Json<T>(pub T);

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let axum::Json(value) = axum::Json::<T>::from_request(req, state).await?;
        Ok(Self(value))
    }
}

/// Map an axum JSON rejection to the error envelope.
///
/// Only the rejection's **status** is consulted. Its `body_text()` is deliberately
/// dropped — see the module docs. It is not logged either: the whole point is that the
/// offending bytes are the client's document.
impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        match rejection.status() {
            StatusCode::PAYLOAD_TOO_LARGE => ApiError::payload_too_large(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE => ApiError::unsupported_media_type(),
            _ => ApiError::invalid_request(
                "The request body is not valid JSON for this endpoint. \
                 See /openapi.json for the expected schema.",
            ),
        }
    }
}

/// Query-string extractor whose rejections are [`ApiError`]s.
///
/// Drop-in for `axum::extract::Query` in argument position.
///
/// # Why this exists alongside [`Json`]
///
/// Same reason, one step earlier in the request: `axum::extract::Query`'s rejection
/// renders as `text/plain` and quotes the offending parameter value back at the caller. A
/// query string is attacker-controlled and lands in access logs; echoing it into a
/// response body is an injection surface and a disclosure path, and — like the JSON
/// rejection — it fires before any handler can intervene.
///
/// # Use it with `#[serde(deny_unknown_fields)]`
///
/// Every query type in this crate denies unknown fields. On an audit endpoint that is not
/// pedantry: a caller who asks for `?principal=alice` and silently receives the *unfiltered*
/// chain has been handed a superset of what they asked for and has no way to notice. For a
/// record whose purpose is to answer "who saw this value", quietly ignoring the filter is
/// worse than refusing the request.
#[derive(Debug, Clone, Copy, Default)]
pub struct Query<T>(pub T);

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let axum::extract::Query(value) =
            axum::extract::Query::<T>::from_request_parts(parts, state).await?;
        Ok(Self(value))
    }
}

/// Map an axum query rejection to the error envelope.
///
/// The rejection's text is dropped for the reason given above. The substituted sentence
/// points at the OpenAPI document rather than naming the offending parameter, because
/// naming it means quoting it.
impl From<QueryRejection> for ApiError {
    fn from(_rejection: QueryRejection) -> Self {
        ApiError::invalid_request(
            "The query string is not valid for this endpoint. Check the parameter names \
             and values against /openapi.json.",
        )
    }
}
