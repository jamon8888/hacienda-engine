//! Handler for `GET /v1/jobs/{id}`.
//!
//! # Membership oracle prevention
//!
//! A caller who is not the job's owner receives 404, **not** 403. Returning 403 would
//! confirm that the job id exists — making the job store a membership oracle and enabling
//! IDOR (OWASP A01). 404 reveals nothing about whether the id is valid.
//!
//! A job with `owner: None` was created by a trusted in-process caller. It is not
//! visible to any authenticated principal — only `Caller::Trusted` can see it (i.e., the
//! server itself via internal tooling, not via this HTTP endpoint).

use axum::{
    extract::{Path, State},
    http::request::Parts,
    Json,
};

use crate::{
    dto::JobResponse,
    error::ApiError,
    handlers::{caller_from_arc, extract_auth_context},
    state::ApiState,
};

/// `GET /v1/jobs/{id}`
pub async fn get_job(
    State(state): State<ApiState>,
    parts: Parts,
    Path(id): Path<String>,
) -> Result<Json<JobResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let job = state
        .jobs
        .get(&id)
        .await
        .map_err(|e| {
            tracing::error!(job_id = %id, error = %e, "job store error");
            ApiError::internal()
        })?
        .ok_or_else(ApiError::not_found)?;

    // Ownership check: see module-level doc for the 404-not-403 rationale.
    match (caller.principal_id(), &job.owner) {
        // Trusted in-process caller can see all jobs.
        (None, _) => {}
        // Authenticated principal must match the job's owner.
        (Some(caller_id), Some(owner_id)) if caller_id == owner_id => {}
        // Mismatch OR the job has no owner (trusted-only) — return 404.
        _ => {
            return Err(ApiError::not_found());
        }
    }

    Ok(Json(JobResponse::from(job)))
}

#[cfg(test)]
mod tests {
    use crate::{
        routes::build_router,
        state::{ApiLimits, ApiState},
    };
    use axum::{body::Body, http::Request};
    use hacienda_core::{
        auth::{
            authn::{InMemoryTokenStore, Token},
            AuthState, Capability, CapabilitySet,
        },
        jobs::InMemoryJobStore,
        HaciendaConfig, HaciendaFacade,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Two principals. `DevTokenResolver` cannot be used here: it maps every token to the
    /// principal `dev-user`, so an ownership test built on it would compare a principal
    /// against itself and pass no matter what the handler does.
    fn two_tenant_app() -> axum::Router {
        let mut store = InMemoryTokenStore::new();
        let caps = CapabilitySet::new([Capability::DocumentsProcess]);
        store.insert("alice-secret", Token::new("t-alice", "alice", caps.clone()));
        store.insert("bob-secret", Token::new("t-bob", "bob", caps));

        let facade = Arc::new(HaciendaFacade::new(HaciendaConfig::default()).unwrap());
        let jobs = InMemoryJobStore::new().into_arc();
        let auth = AuthState::new(Arc::new(store));
        build_router(ApiState::new(facade, jobs, auth, ApiLimits::default()))
    }

    async fn create_job_as(app: &axum::Router, secret: &str) -> String {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/documents/async")
                    .header("authorization", format!("Bearer {secret}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"documents":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 202, "job creation must succeed");

        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["job_id"].as_str().unwrap().to_string()
    }

    async fn get_job_as(app: &axum::Router, secret: &str, id: &str) -> u16 {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/jobs/{id}"))
                    .header("authorization", format!("Bearer {secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    /// A principal must not read another principal's job (OWASP A01, IDOR).
    #[tokio::test]
    async fn should_return_404_when_another_principal_requests_a_job() {
        let app = two_tenant_app();
        let job_id = create_job_as(&app, "alice-secret").await;

        assert_eq!(
            get_job_as(&app, "bob-secret", &job_id).await,
            404,
            "a non-owner must not learn that the job id exists"
        );
    }

    /// 404 for a non-owner is only non-disclosing if it is *indistinguishable* from 404
    /// for an id that does not exist. A 403 for the first and 404 for the second would
    /// turn the endpoint into a membership oracle for job ids.
    #[tokio::test]
    async fn should_not_distinguish_a_foreign_job_from_a_missing_one() {
        let app = two_tenant_app();
        let job_id = create_job_as(&app, "alice-secret").await;

        let foreign = get_job_as(&app, "bob-secret", &job_id).await;
        let missing = get_job_as(&app, "bob-secret", "00000000-dead-beef-0000-000000000000").await;

        assert_eq!(
            foreign, missing,
            "an existing job owned by someone else must be indistinguishable from a \
             job id that was never issued"
        );
    }

    /// The owner must still be able to read their own job — otherwise the test above
    /// would pass with a handler that 404s unconditionally.
    #[tokio::test]
    async fn should_return_the_job_to_its_owner() {
        let app = two_tenant_app();
        let job_id = create_job_as(&app, "alice-secret").await;

        assert_eq!(get_job_as(&app, "alice-secret", &job_id).await, 200);
    }
}
