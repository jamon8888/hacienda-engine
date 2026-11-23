//! GraphRAG graph-context endpoints.
//!
//! The endpoint exposes only bounded, provenance-bearing graph context. It
//! never accepts raw document text and remains client-neutral for Tauri and
//! other API consumers.

use axum::{
    extract::{Path, State},
    http::request::Parts,
    Json,
};
use hacienda_rag::{EntityId, GraphContext, GraphHealth, GraphStore};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    error::ApiError,
    extract::Json as SafeJson,
    handlers::{
        caller_from_arc, extract_auth_context,
        rag::{
            redact_retrieval_json, redact_retrieval_text, scope_collection_name,
            validate_collection_name,
        },
    },
    state::ApiState,
};

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NeighborhoodRequest {
    #[schema(value_type = Vec<String>)]
    pub entity_ids: Vec<EntityId>,
    #[serde(default = "default_hops")]
    pub hops: u8,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f32,
}

fn default_hops() -> u8 {
    1
}

fn default_limit() -> usize {
    50
}

fn default_min_confidence() -> f32 {
    0.9
}

impl NeighborhoodRequest {
    fn validate(&self) -> Result<(), ApiError> {
        if self.entity_ids.is_empty()
            || self.entity_ids.len() > hacienda_rag::MAX_ENTITY_IDS
            || !(1..=2).contains(&self.hops)
            || self.limit == 0
            || self.limit > 500
            || !self.min_confidence.is_finite()
            || !(0.0..=1.0).contains(&self.min_confidence)
        {
            return Err(ApiError::invalid_request(
                "invalid graph neighborhood bounds",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct GraphHealthResponse {
    #[schema(value_type = serde_json::Value)]
    pub graph: GraphHealth,
}

pub(super) fn require_graph_store(
    state: &ApiState,
) -> Result<&std::sync::Arc<dyn GraphStore>, ApiError> {
    state
        .graph_store
        .as_ref()
        .ok_or_else(|| ApiError::invalid_request("GraphRAG is not enabled on this server."))
}

#[utoipa::path(
    post,
    path = "/v1/rag/collections/{name}/graph/neighborhood",
    tag = "rag",
    operation_id = "graph_neighborhood",
    security(("bearerAuth" = [])),
    params(("name" = String, Path, description = "Collection name")),
    request_body = NeighborhoodRequest,
    responses(
        (status = 200, description = "Bounded tenant-scoped graph neighborhood", body = serde_json::Value),
        (status = 400, description = "Invalid graph query"),
        (status = 404, description = "No such collection")
    )
)]
pub async fn neighborhood(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    parts: Parts,
    SafeJson(body): SafeJson<NeighborhoodRequest>,
) -> Result<Json<GraphContext>, ApiError> {
    let store = require_graph_store(&state)?;
    body.validate()?;
    validate_collection_name(&name)?;
    let ctx = extract_auth_context(&parts);
    let tenant = caller_from_arc(&ctx).tenant_ctx();
    let collection = scope_collection_name(&tenant, &name);
    let mut context = store
        .neighborhood(
            &collection,
            tenant.tenant.as_str(),
            &body.entity_ids,
            body.hops,
            body.limit,
            body.min_confidence,
        )
        .await
        .map_err(ApiError::from)?;
    redact_graph_context(&state.facade, caller_from_arc(&ctx), &mut context).await?;
    Ok(Json(context))
}

pub(super) async fn redact_graph_context(
    facade: &hacienda_core::HaciendaFacade,
    caller: hacienda_core::auth::Caller<'_>,
    context: &mut GraphContext,
) -> Result<(), ApiError> {
    for entity in &mut context.entities {
        entity.normalized_value =
            redact_retrieval_text(facade, caller, &entity.normalized_value).await?;
        redact_retrieval_json(facade, caller, &mut entity.properties).await?;
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/v1/rag/graph/health",
    tag = "rag",
    operation_id = "graph_health",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Graph backend health", body = GraphHealthResponse),
        (status = 400, description = "GraphRAG is disabled")
    )
)]
pub async fn health(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<GraphHealthResponse>, ApiError> {
    let store = require_graph_store(&state)?;
    let ctx = extract_auth_context(&parts);
    let tenant = caller_from_arc(&ctx).tenant_ctx();
    Ok(Json(GraphHealthResponse {
        graph: store
            .health(tenant.tenant.as_str())
            .await
            .map_err(ApiError::from)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacienda_core::auth::{
        authn::DevTokenResolver, AuthContext, AuthExtension, AuthState, Caller, Capability,
        CapabilitySet,
    };
    use hacienda_core::jobs::InMemoryJobStore;
    use hacienda_core::pii::PipelineConfig;
    use hacienda_core::tenancy::TenantId;
    use hacienda_core::{HaciendaConfig, HaciendaFacade};
    use hacienda_rag::{DocumentId, EntityLabel, GraphEntity, InMemoryGraphStore, DEFAULT_TENANT};
    use std::sync::Arc;

    const EMAIL_A: &str = "alice@example.com";
    const EMAIL_B: &str = "bob@example.com";

    fn graph_entity(id: &str, tenant: Option<&str>) -> GraphEntity {
        GraphEntity {
            id: EntityId(id.to_owned()),
            label: EntityLabel::Person,
            normalized_value: EMAIL_A.to_owned(),
            document_id: DocumentId("doc-1".to_owned()),
            tenant_id: tenant.map(str::to_owned),
            chunk_id: None,
            start: 0,
            end: 4,
            confidence: 0.99,
            model_version: "test-extractor".to_owned(),
            properties: serde_json::json!({"contact": EMAIL_B}),
        }
    }

    fn state_with_graph() -> ApiState {
        let config = HaciendaConfig::default().with_pii(PipelineConfig::default());
        let facade = Arc::new(HaciendaFacade::new(config).expect("facade"));
        let jobs = InMemoryJobStore::new().into_arc();
        let auth = AuthState::new(Arc::new(DevTokenResolver)).with_enabled(false);
        let graph: Arc<dyn GraphStore> = Arc::new(InMemoryGraphStore::default());
        ApiState::new(facade, jobs, auth, crate::state::ApiLimits::default())
            .with_graph_store(graph)
    }

    fn parts_for(tenant: &str) -> Parts {
        let ctx = AuthContext::with_tenant(
            "test-principal",
            TenantId::new(tenant),
            CapabilitySet::new([Capability::DocumentsProcess]),
        );
        let mut parts = axum::http::Request::new(()).into_parts().0;
        parts.extensions.insert(AuthExtension(Arc::new(ctx)));
        parts
    }

    /// Graph values are extracted text and `properties` is free-form JSON: both
    /// must go through Hacienda before a `GraphContext` is serialized to a caller.
    #[tokio::test]
    async fn redact_graph_context_removes_pii_from_values_and_properties() {
        let config = HaciendaConfig::default().with_pii(PipelineConfig::default());
        let facade = HaciendaFacade::new(config).expect("facade");
        let mut context = GraphContext {
            entities: vec![graph_entity("e1", None)],
            relations: Vec::new(),
        };

        redact_graph_context(&facade, Caller::Trusted, &mut context)
            .await
            .expect("redaction must succeed");

        let serialized = serde_json::to_string(&context).expect("serialize graph context");
        assert!(
            !serialized.contains(EMAIL_A),
            "entity value PII leaked: {serialized}"
        );
        assert!(
            !serialized.contains(EMAIL_B),
            "entity properties PII leaked: {serialized}"
        );
        assert!(
            serialized.contains("[EMAIL]"),
            "redaction marker missing: {serialized}"
        );
    }

    /// A missing PII pipeline must abort the response, not return raw graph values.
    #[tokio::test]
    async fn redact_graph_context_fails_closed_without_a_pii_pipeline() {
        let facade = HaciendaFacade::new(HaciendaConfig::default()).expect("facade");
        let mut context = GraphContext {
            entities: vec![graph_entity("e1", None)],
            relations: Vec::new(),
        };

        let err = redact_graph_context(&facade, Caller::Trusted, &mut context)
            .await
            .expect_err("raw graph values must be refused without Hacienda redaction");
        assert_eq!(
            err.code,
            crate::error::ApiErrorCode::InvalidRequest,
            "expected a 400-class error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn neighborhood_returns_only_the_callers_tenant_entities() {
        let state = state_with_graph();
        let store = state.graph_store.as_ref().expect("graph store");
        store
            .upsert_entities(
                "acme:contracts",
                "acme",
                &[graph_entity("acme-e", Some("acme"))],
            )
            .await
            .expect("upsert acme entity");
        store
            .upsert_entities(
                "acme:contracts",
                "globex",
                &[graph_entity("globex-e", Some("globex"))],
            )
            .await
            .expect("upsert globex entity");

        let response = neighborhood(
            State(state),
            Path("contracts".to_owned()),
            parts_for("acme"),
            SafeJson(NeighborhoodRequest {
                entity_ids: vec![EntityId("acme-e".to_owned())],
                hops: 1,
                limit: 10,
                min_confidence: 0.9,
            }),
        )
        .await
        .expect("neighborhood must succeed");
        let context = response.0;

        assert_eq!(context.entities.len(), 1);
        assert_eq!(context.entities[0].id.0, "acme-e");
        assert_eq!(context.entities[0].tenant_id.as_deref(), Some("acme"));
    }

    #[test]
    fn neighborhood_rejects_out_of_bounds_graph_expansion() {
        let base = || NeighborhoodRequest {
            entity_ids: vec![EntityId("e".to_owned())],
            hops: 1,
            limit: 10,
            min_confidence: 0.9,
        };

        let mut empty_ids = base();
        empty_ids.entity_ids.clear();
        assert!(empty_ids.validate().is_err());

        let mut oversized = base();
        oversized.limit = 501;
        assert!(oversized.validate().is_err());

        let mut too_many_ids = base();
        too_many_ids.entity_ids = (0..=hacienda_rag::MAX_ENTITY_IDS)
            .map(|i| EntityId(format!("e-{i}")))
            .collect();
        assert!(too_many_ids.validate().is_err());

        let mut too_many_hops = base();
        too_many_hops.hops = 3;
        assert!(too_many_hops.validate().is_err());

        let mut bad_confidence = base();
        bad_confidence.min_confidence = f32::NAN;
        assert!(bad_confidence.validate().is_err());

        assert!(base().validate().is_ok());
    }

    /// The legacy/default tenant must still resolve unprefixed collection names.
    #[tokio::test]
    async fn neighborhood_works_for_the_default_tenant() {
        let state = state_with_graph();
        let store = state.graph_store.as_ref().expect("graph store");
        store
            .upsert_entities(
                "contracts",
                DEFAULT_TENANT,
                &[graph_entity("default-e", Some(DEFAULT_TENANT))],
            )
            .await
            .expect("upsert entities");

        let response = neighborhood(
            State(state),
            Path("contracts".to_owned()),
            parts_for(DEFAULT_TENANT),
            SafeJson(NeighborhoodRequest {
                entity_ids: vec![EntityId("default-e".to_owned())],
                hops: 1,
                limit: 10,
                min_confidence: 0.9,
            }),
        )
        .await
        .expect("neighborhood must succeed for the default tenant");
        assert_eq!(response.0.entities.len(), 1);
    }
}
