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
    dto::{JobListQuery, JobListResponse, JobResponse, JobResultResponse},
    error::ApiError,
    extract::Query,
    handlers::{caller_from_arc, extract_auth_context},
    state::ApiState,
};

/// Default page size for `GET /v1/jobs` when `limit` is omitted.
const DEFAULT_JOB_LIST_LIMIT: usize = 50;

/// Upper bound on `limit` for `GET /v1/jobs`, regardless of what the caller requests.
const MAX_JOB_LIST_LIMIT: usize = 200;

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

/// `GET /v1/jobs`
///
/// Tenant-scoped: an authenticated principal sees only jobs it owns. A trusted in-process
/// caller sees every job, including owner-less ones — see the module docs for why a
/// mismatch or an owner-less job is invisible to an authenticated principal.
///
/// Pagination is applied in-process over the full filtered result, not pushed into the
/// store query: [`hacienda_core::jobs::JobStore::list`] has no `limit`/`offset` parameters,
/// and extending it is a bigger change than this route needs. `total` is reported before
/// `limit`/`offset` slicing so a client can tell whether more pages remain.
pub async fn list_jobs(
    State(state): State<ApiState>,
    parts: Parts,
    Query(query): Query<JobListQuery>,
) -> Result<Json<JobListResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let caller_id = caller.principal_id();

    let jobs = state.jobs.list(query.status).await.map_err(|e| {
        tracing::error!(error = %e, "job store error");
        ApiError::internal()
    })?;

    let visible: Vec<_> = jobs
        .into_iter()
        .filter(|job| match (caller_id, &job.owner) {
            (None, _) => true,
            (Some(caller_id), Some(owner_id)) => caller_id == owner_id,
            (Some(_), None) => false,
        })
        .collect();

    let total = visible.len();
    let offset = query.offset.unwrap_or(0);
    let limit = query
        .limit
        .unwrap_or(DEFAULT_JOB_LIST_LIMIT)
        .min(MAX_JOB_LIST_LIMIT);

    let page: Vec<JobResponse> = visible
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(JobResponse::from)
        .collect();

    Ok(Json(JobListResponse { jobs: page, total }))
}

/// `GET /v1/jobs/{id}/result`
///
/// Distinct from [`get_job`]: a polling loop that only cares whether a job is done should
/// not pay to deserialize a potentially large `result` payload on every poll. This endpoint
/// always returns 200 regardless of job state — querying the result of a queued or running
/// job is a legitimate operation (its `result`/`error` are simply absent), not an error.
///
/// Ownership check mirrors [`get_job`]: 404, never 403, for the same membership-oracle
/// reason documented at the top of this module.
pub async fn get_job_result(
    State(state): State<ApiState>,
    parts: Parts,
    Path(id): Path<String>,
) -> Result<Json<JobResultResponse>, ApiError> {
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

    match (caller.principal_id(), &job.owner) {
        (None, _) => {}
        (Some(caller_id), Some(owner_id)) if caller_id == owner_id => {}
        _ => {
            return Err(ApiError::not_found());
        }
    }

    let result = job
        .result_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    Ok(Json(JobResultResponse {
        status: job.status,
        result,
        error: job.error,
    }))
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

    async fn list_jobs_as(app: &axum::Router, secret: &str, query: &str) -> serde_json::Value {
        let uri = if query.is_empty() {
            "/v1/jobs".to_string()
        } else {
            format!("/v1/jobs?{query}")
        };
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .header("authorization", format!("Bearer {secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn get_job_result_as(
        app: &axum::Router,
        secret: &str,
        id: &str,
    ) -> (u16, serde_json::Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/jobs/{id}/result"))
                    .header("authorization", format!("Bearer {secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let value = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, value)
    }

    /// A principal must only see jobs it owns — an unfiltered list would leak the
    /// existence of another tenant's jobs (OWASP A01, IDOR).
    #[tokio::test]
    async fn should_only_list_jobs_owned_by_the_caller() {
        let app = two_tenant_app();
        create_job_as(&app, "alice-secret").await;
        create_job_as(&app, "alice-secret").await;
        create_job_as(&app, "bob-secret").await;

        let alice_view = list_jobs_as(&app, "alice-secret", "").await;
        assert_eq!(
            alice_view["total"], 2,
            "alice must see only her own two jobs"
        );
        assert_eq!(alice_view["jobs"].as_array().unwrap().len(), 2);

        let bob_view = list_jobs_as(&app, "bob-secret", "").await;
        assert_eq!(bob_view["total"], 1, "bob must see only his own one job");
    }

    /// `limit`/`offset` must slice the caller's visible set, and `total` must reflect the
    /// count before slicing so a client can detect further pages.
    #[tokio::test]
    async fn should_paginate_the_callers_job_list() {
        let app = two_tenant_app();
        for _ in 0..3 {
            create_job_as(&app, "alice-secret").await;
        }

        let page = list_jobs_as(&app, "alice-secret", "limit=2&offset=0").await;
        assert_eq!(page["total"], 3);
        assert_eq!(page["jobs"].as_array().unwrap().len(), 2);

        let second_page = list_jobs_as(&app, "alice-secret", "limit=2&offset=2").await;
        assert_eq!(second_page["total"], 3);
        assert_eq!(second_page["jobs"].as_array().unwrap().len(), 1);
    }

    /// A malformed query string must fail into the `{"error": {...}}` envelope, not axum's
    /// default plain-text rejection — see `extract::Query`.
    #[tokio::test]
    async fn should_reject_a_malformed_limit_with_the_error_envelope() {
        let app = two_tenant_app();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/jobs?limit=not-a-number")
                    .header("authorization", "Bearer alice-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 400);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            value.get("error").is_some(),
            "rejection must use the {{\"error\": {{...}}}} envelope, got: {value}"
        );
    }

    /// `GET /v1/jobs/{id}/result` must apply the same 404-not-403 ownership check as
    /// `GET /v1/jobs/{id}` — see the module docs.
    #[tokio::test]
    async fn should_return_404_for_a_foreign_jobs_result() {
        let app = two_tenant_app();
        let job_id = create_job_as(&app, "alice-secret").await;

        let (status, _) = get_job_result_as(&app, "bob-secret", &job_id).await;
        assert_eq!(status, 404);
    }

    /// Querying the result of a job that has not finished is not an error — it always
    /// returns 200, with `result`/`error` simply absent until the job settles.
    ///
    /// This does not pin the job to `queued` specifically: the job runs in a detached
    /// `tokio::spawn` task (see `handlers/documents.rs`) that a single-threaded test
    /// runtime may schedule before this assertion runs, so the job could already be
    /// `succeeded` by the time we ask. What must hold in every state is the *shape*:
    /// `result` is present only when `succeeded`, `error` only when `failed`.
    #[tokio::test]
    async fn should_return_200_with_a_result_shape_matching_job_status() {
        let app = two_tenant_app();
        let job_id = create_job_as(&app, "alice-secret").await;

        let (status, body) = get_job_result_as(&app, "alice-secret", &job_id).await;
        assert_eq!(status, 200);
        match body["status"].as_str().unwrap() {
            "succeeded" => assert!(
                body.get("result").is_some(),
                "succeeded job must carry a result"
            ),
            "failed" => assert!(
                body.get("error").is_some(),
                "failed job must carry an error"
            ),
            "queued" | "running" => {
                assert!(
                    body.get("result").is_none(),
                    "unfinished job must not carry a result"
                );
                assert!(
                    body.get("error").is_none(),
                    "unfinished job must not carry an error"
                );
            }
            other => panic!("unexpected job status: {other}"),
        }
    }
}
