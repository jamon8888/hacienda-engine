//! Two-stage GraphRAG retrieval endpoint.

use axum::{
    extract::{Path, State},
    http::request::Parts,
    Json,
};
use hacienda_rag::{retrieve_with_graph, EntityId, GraphRagOutput, GraphRagQuery};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::{
    error::ApiError,
    extract::Json as SafeJson,
    handlers::{
        caller_from_arc, extract_auth_context,
        graph::redact_graph_context,
        rag::{
            enforce_post_retrieval_tenant, merge_with_tenant_filter, redact_retrieval_output,
            require_store, scope_collection_name, tenant_authorization_filter,
            validate_collection_name,
        },
    },
    state::ApiState,
};

/// `POST /v1/rag/collections/{name}/graph/retrieve`
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GraphRetrieveRequest {
    #[schema(value_type = serde_json::Value)]
    pub retrieval: hacienda_rag::RetrieveQuery,
    #[schema(value_type = Vec<String>)]
    pub entity_ids: Vec<EntityId>,
    #[serde(default = "default_hops")]
    pub hops: u8,
    #[serde(default = "default_graph_limit")]
    pub graph_limit: usize,
    #[serde(default = "default_graph_min_confidence")]
    pub graph_min_confidence: f32,
}

fn default_hops() -> u8 {
    1
}
fn default_graph_limit() -> usize {
    50
}
fn default_graph_min_confidence() -> f32 {
    0.9
}

impl From<GraphRetrieveRequest> for GraphRagQuery {
    fn from(value: GraphRetrieveRequest) -> Self {
        Self {
            retrieval: value.retrieval,
            entity_ids: value.entity_ids,
            hops: value.hops,
            graph_limit: value.graph_limit,
            graph_min_confidence: value.graph_min_confidence,
        }
    }
}

#[utoipa::path(
    post,
    path = "/v1/rag/collections/{name}/graph/retrieve",
    tag = "rag",
    operation_id = "graph_retrieve",
    security(("bearerAuth" = [])),
    params(("name" = String, Path, description = "Collection name")),
    request_body = GraphRetrieveRequest,
    responses(
        (status = 200, description = "Vector retrieval plus bounded tenant-scoped graph context", body = serde_json::Value),
        (status = 400, description = "Invalid retrieval or graph query"),
        (status = 404, description = "No such collection")
    )
)]
pub async fn retrieve(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    parts: Parts,
    SafeJson(body): SafeJson<GraphRetrieveRequest>,
) -> Result<Json<GraphRagOutput>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant = caller.tenant_ctx();
    validate_collection_name(&name)?;
    let collection = scope_collection_name(&tenant, &name);
    let rag = require_store(&state)?;
    let graph = state
        .graph_store
        .as_ref()
        .ok_or_else(|| ApiError::invalid_request("GraphRAG is not enabled"))?;

    let mut query: GraphRagQuery = body.into();
    query.retrieval.tenant_id = Some(tenant.tenant.as_str().to_owned());
    query.retrieval.filter = merge_with_tenant_filter(
        tenant_authorization_filter(&tenant),
        query.retrieval.filter.take(),
    );
    let mut output = retrieve_with_graph(
        rag.as_ref(),
        graph.as_ref(),
        &collection,
        tenant.tenant.as_str(),
        &query,
    )
    .await
    .map_err(ApiError::from)?;

    enforce_post_retrieval_tenant(&tenant, &mut output.retrieval.chunks);
    output.retrieval = redact_retrieval_output(&state.facade, caller, output.retrieval).await?;
    redact_graph_context(&state.facade, caller, &mut output.graph).await?;

    Ok(Json(output))
}
