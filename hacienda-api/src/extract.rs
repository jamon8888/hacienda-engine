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
    extract::{rejection::JsonRejection, FromRequest, Request},
    http::StatusCode,
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
