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
            enforce_post_retrieval_tenant, redact_retrieval_output, require_store,
            scope_collection_name, validate_collection_name,
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

/// Stamp the authenticated tenant into the backend-neutral query. The tenant
/// boundary stays in `tenant_id` (owner-or-ACL) — deliberately *not* in the
/// field filter, which is evaluated as a required conjunct and would remove
/// documents another tenant shared with us via its ACL.
fn build_graph_query(tenant: &str, body: GraphRetrieveRequest) -> GraphRagQuery {
    let mut query: GraphRagQuery = body.into();
    query.retrieval.tenant_id = Some(tenant.to_owned());
    query
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

    let query = build_graph_query(tenant.tenant.as_str(), body);
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

#[cfg(test)]
mod tests {
    use super::*;
    use hacienda_rag::{
        ChunkRecord, CollectionSpec, DocumentRecord, Filter, FilterField, InMemoryVectorStore,
        RagStore, RetrieveQuery,
    };

    #[tokio::test]
    async fn acl_shared_document_survives_graph_retrieval_tenant_boundary() {
        let store = InMemoryVectorStore::new("test");
        store
            .ensure_collection(&CollectionSpec::new("contracts", 2))
            .await
            .unwrap();
        let document = DocumentRecord {
            full_text: "shared contract".to_owned(),
            tenant_id: Some("globex".to_owned()),
            acl: Some(serde_json::json!({"tenants": ["acme"]})),
            ..DocumentRecord::default()
        };
        let chunk = ChunkRecord {
            external_id: None,
            ordinal: 0,
            content: "shared contract".to_owned(),
            embedding: vec![1.0, 0.0],
            sparse_embedding: None,
            multi_vector: None,
            chunk_metadata: serde_json::Value::Null,
            dossier_id: None,
            version_ref: None,
            tenant_id: Some("globex".to_owned()),
            acl: Some(serde_json::json!({"tenants": ["acme"]})),
        };
        store
            .upsert_document("contracts", &document, &[chunk])
            .await
            .unwrap();

        let mut retrieval = RetrieveQuery::vector(5);
        retrieval.query_vector = Some(vec![1.0, 0.0]);
        // A caller-supplied filter must be preserved, but no owner-only
        // equality is injected on top of the tenant boundary.
        retrieval.filter = Some(Filter::Eq {
            field: FilterField("doc.full_text".to_owned()),
            value: serde_json::json!("shared contract"),
        });
        let query = build_graph_query(
            "acme",
            GraphRetrieveRequest {
                retrieval,
                entity_ids: vec![EntityId("e".to_owned())],
                hops: 1,
                graph_limit: 10,
                graph_min_confidence: 0.9,
            },
        );
        assert_eq!(query.retrieval.tenant_id.as_deref(), Some("acme"));
        assert!(query.retrieval.filter.is_some());

        let output = store.retrieve("contracts", &query.retrieval).await.unwrap();
        assert_eq!(
            output.chunks.len(),
            1,
            "an ACL-granted row must remain eligible for graph retrieval"
        );
    }
}
