//! Handlers for `POST /v1/uploads/presign` and `POST /v1/uploads/confirm`.
//!
//! Presigned uploads reopen an SSRF decision already settled in favor of allowing this
//! shape: the client PUTs bytes directly to hacienda's own bucket via a presigned URL
//! (out-of-band, never through this server — xberg's own pattern, per the Dart SDK's
//! `presignUpload`/`confirmUpload` pair), and `confirm_upload` fetches metadata from that
//! same bucket by a server-issued key (`upload_id`) — never a client-supplied URL. See
//! spec §5 ("Presigned Uploads Reopen an SSRF Decision") for the full analysis; this is
//! not the shape the "no wire-supplied URL" rule in `hacienda-api-cli-surface` was
//! written to block.
//!
//! `ObjectStore` has no in-memory backend (S3-compatible only) — like `PresetStore` and
//! `DocumentVersionStore`, these routes 400 when `ApiState::object_store` is `None`.

use axum::{extract::State, Json};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use hacienda_core::store::object::ObjectStore;

use crate::{
    dto::{
        ConfirmUploadRequest, ConfirmUploadResponse, PresignUploadRequest, PresignUploadResponse,
    },
    error::ApiError,
    extract::Json as SafeJson,
    state::ApiState,
};

/// How long a presigned PUT URL remains valid. Generous enough for a slow client
/// connection to complete a large upload, short enough that a leaked URL isn't usable
/// indefinitely.
const PRESIGN_PUT_EXPIRY: Duration = Duration::from_secs(15 * 60);

/// Prefix applied to every upload's storage key, namespacing this feature's objects
/// separately from anything else that might later share the same bucket.
const UPLOAD_KEY_PREFIX: &str = "uploads/";

fn require_store(state: &ApiState) -> Result<&Arc<dyn ObjectStore>, ApiError> {
    state
        .object_store
        .as_ref()
        .ok_or_else(|| ApiError::invalid_request("Uploads are not enabled on this server."))
}

/// The storage key for an upload is derived solely from the server-issued `upload_id`,
/// never from the client-supplied `filename` — so a crafted filename can't steer or
/// collide with another upload's path.
fn object_key(upload_id: Uuid) -> String {
    format!("{UPLOAD_KEY_PREFIX}{upload_id}")
}

/// `POST /v1/uploads/presign` — issue a presigned PUT URL for a new upload.
pub async fn presign_upload(
    State(state): State<ApiState>,
    SafeJson(body): SafeJson<PresignUploadRequest>,
) -> Result<Json<PresignUploadResponse>, ApiError> {
    let store = require_store(&state)?;
    let upload_id = Uuid::new_v4();
    let key = object_key(upload_id);

    let presigned = store.presign_put(&key, &body.mime_type, PRESIGN_PUT_EXPIRY);

    Ok(Json(PresignUploadResponse {
        upload_id,
        upload_url: presigned.url,
        method: "PUT".to_string(),
        required_headers: presigned.required_headers,
        expires_at: presigned.expires_at,
    }))
}

/// `POST /v1/uploads/confirm` — verify a presigned upload actually completed.
///
/// Issues a `HEAD` against hacienda's own bucket for `upload_id`'s key — never a
/// client-supplied URL (spec §5). 404s if the object doesn't exist yet, distinguishing
/// "not uploaded" (a client-triggerable condition: called before the `PUT` finished, or
/// with a stale/wrong `upload_id`) from a genuine backend failure (500, via
/// `ObjectStoreError`'s `From<ObjectStoreError> for ApiError` impl).
pub async fn confirm_upload(
    State(state): State<ApiState>,
    SafeJson(body): SafeJson<ConfirmUploadRequest>,
) -> Result<Json<ConfirmUploadResponse>, ApiError> {
    let store = require_store(&state)?;
    let key = object_key(body.upload_id);

    let metadata = store
        .head(&key)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;

    Ok(Json(ConfirmUploadResponse {
        upload_id: body.upload_id,
        size_bytes: metadata.size_bytes,
        content_type: metadata.content_type,
    }))
}
