//! Backend-agnostic knowledge-graph contract for GraphRAG.
//!
//! The graph layer is deliberately independent from LanceDB and from any
//! particular extraction runtime.  This keeps the API and the in-memory test
//! seam stable while the optional Lance Graph adapter is validated separately.

use crate::error::{RagError, RagResult};
use crate::query::{RetrieveOutput, RetrieveQuery};
use crate::store::RagStore;
use crate::types::{ChunkId, DocumentId, DEFAULT_TENANT};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::RwLock;

/// Upper bound on entity IDs accepted by any graph expansion request. Request
/// bodies are caller-controlled, so every entry point — DTO validation, the
/// backend-neutral query, and both store implementations — enforces the same
/// cap before any full-collection load or BFS traversal.
pub const MAX_ENTITY_IDS: usize = 100;

/// Stable identifier for a graph entity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityId(pub String);

/// Closed node ontology for the legal/documentary MVP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntityLabel {
    Person,
    Organization,
    Document,
    Contract,
    Clause,
    Date,
    Location,
    Identifier,
    MonetaryAmount,
}

/// Closed relation ontology for the legal/documentary MVP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelationType {
    Mentions,
    PartyTo,
    IssuedBy,
    SignedBy,
    References,
    Amends,
    EffectiveOn,
    LocatedIn,
    HasClause,
    SameAs,
}

/// Evidence supporting an entity or relation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceSpan {
    pub document_id: DocumentId,
    pub chunk_id: Option<ChunkId>,
    pub start: u32,
    pub end: u32,
    pub text_hash: String,
}

impl EvidenceSpan {
    /// Validate offsets without retaining source text in the graph store.
    pub fn validate(&self) -> RagResult<()> {
        if self.start >= self.end {
            return Err(RagError::InvalidQuery(
                "evidence span must have start < end".to_owned(),
            ));
        }
        if self.text_hash.trim().is_empty() {
            return Err(RagError::InvalidQuery(
                "evidence span requires a non-empty text_hash".to_owned(),
            ));
        }
        Ok(())
    }
}

/// An extracted entity with model and source provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEntity {
    pub id: EntityId,
    pub label: EntityLabel,
    pub normalized_value: String,
    pub document_id: DocumentId,
    /// Owning tenant. `None` is the legacy/default tenant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    pub chunk_id: Option<ChunkId>,
    pub start: u32,
    pub end: u32,
    pub confidence: f32,
    pub model_version: String,
    #[serde(default)]
    pub properties: serde_json::Value,
}

/// An extracted, ontology-validated relation.
///
/// Tenant isolation is enforced through both endpoints (`GraphEntity::tenant_id`)
/// *and* this field, so a relation can never be read across tenants even if an
/// endpoint is later re-assigned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphRelation {
    pub subject: EntityId,
    pub predicate: RelationType,
    pub object: EntityId,
    pub evidence: EvidenceSpan,
    pub confidence: f32,
    pub ontology_version: String,
    /// Owning tenant. `None` is the legacy/default tenant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

pub(crate) fn effective_tenant(tenant_id: Option<&str>) -> &str {
    tenant_id.unwrap_or(DEFAULT_TENANT)
}

/// A bounded graph context returned to GraphRAG retrieval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GraphContext {
    pub entities: Vec<GraphEntity>,
    pub relations: Vec<GraphRelation>,
}

/// Operational counters for readiness and observability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphHealth {
    pub ready: bool,
    pub entity_count: usize,
    pub relation_count: usize,
}

/// Inputs for a bounded two-stage GraphRAG retrieval.
///
/// Entity linking is intentionally supplied by the caller for this first
/// slice.  A future linker (GLiNER or another extractor) can populate the
/// same contract without coupling the retrieval store to a model runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRagQuery {
    pub retrieval: RetrieveQuery,
    pub entity_ids: Vec<EntityId>,
    #[serde(default = "default_hops")]
    pub hops: u8,
    #[serde(default = "default_graph_limit")]
    pub graph_limit: usize,
    #[serde(default = "default_graph_min_confidence")]
    pub graph_min_confidence: f32,
}

impl GraphRagQuery {
    /// Validate graph-specific bounds before touching either backend.
    pub fn validate(&self) -> RagResult<()> {
        if self.entity_ids.is_empty() || self.entity_ids.len() > MAX_ENTITY_IDS {
            return Err(RagError::InvalidQuery(format!(
                "graph retrieval requires between 1 and {MAX_ENTITY_IDS} entity_ids"
            )));
        }
        if !(1..=2).contains(&self.hops) {
            return Err(RagError::InvalidQuery(
                "graph retrieval hops must be between 1 and 2".to_owned(),
            ));
        }
        if self.graph_limit == 0 || self.graph_limit > 500 {
            return Err(RagError::InvalidQuery(
                "graph retrieval graph_limit must be between 1 and 500".to_owned(),
            ));
        }
        if !self.graph_min_confidence.is_finite()
            || !(0.0..=1.0).contains(&self.graph_min_confidence)
        {
            return Err(RagError::InvalidQuery(
                "graph retrieval graph_min_confidence must be finite and between 0 and 1"
                    .to_owned(),
            ));
        }
        Ok(())
    }
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

/// The vector/full-text result and its bounded, tenant-scoped graph context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRagOutput {
    pub retrieval: RetrieveOutput,
    pub graph: GraphContext,
}

/// Execute retrieval and graph expansion as one backend-neutral operation.
///
/// The tenant is authoritative server-side: any tenant supplied in
/// `query.retrieval` is overwritten before the vector store sees it.
pub async fn retrieve_with_graph(
    rag: &dyn RagStore,
    graph: &dyn GraphStore,
    collection: &str,
    tenant_id: &str,
    query: &GraphRagQuery,
) -> RagResult<GraphRagOutput> {
    if tenant_id.trim().is_empty() {
        return Err(RagError::InvalidQuery(
            "graph retrieval requires a tenant_id".to_owned(),
        ));
    }
    query.validate()?;

    let mut retrieval_query = query.retrieval.clone();
    retrieval_query.tenant_id = Some(tenant_id.to_owned());
    let retrieval = rag.retrieve(collection, &retrieval_query).await?;
    let graph_context = graph
        .neighborhood(
            collection,
            tenant_id,
            &query.entity_ids,
            query.hops,
            query.graph_limit,
            query.graph_min_confidence,
        )
        .await?;

    Ok(GraphRagOutput {
        retrieval,
        graph: graph_context,
    })
}

/// Backend-neutral GraphRAG graph contract.
///
/// Every mutating or read method is scoped by `(collection, tenant_id)` so a
/// tenant can never read or write another tenant's graph, even indirectly.
#[async_trait]
pub trait GraphStore: Send + Sync + 'static {
    async fn upsert_entities(
        &self,
        collection: &str,
        tenant_id: &str,
        entities: &[GraphEntity],
    ) -> RagResult<()>;
    async fn upsert_relations(
        &self,
        collection: &str,
        tenant_id: &str,
        relations: &[GraphRelation],
    ) -> RagResult<()>;
    async fn neighborhood(
        &self,
        collection: &str,
        tenant_id: &str,
        entity_ids: &[EntityId],
        hops: u8,
        limit: usize,
        min_confidence: f32,
    ) -> RagResult<GraphContext>;
    async fn delete_document(
        &self,
        collection: &str,
        tenant_id: &str,
        document_id: &DocumentId,
    ) -> RagResult<u64>;
    async fn health(&self, tenant_id: &str) -> RagResult<GraphHealth>;
}

#[derive(Default)]
struct GraphData {
    entities: HashMap<EntityId, GraphEntity>,
    relations: HashMap<(EntityId, RelationType, EntityId, String), GraphRelation>,
}

/// Exact, deterministic graph backend used by tests and local development.
#[derive(Default)]
pub struct InMemoryGraphStore {
    collections: RwLock<HashMap<String, GraphData>>,
}

impl InMemoryGraphStore {
    fn validate_entity(entity: &GraphEntity) -> RagResult<()> {
        Self::validate_confidence(entity.confidence, "entity")?;
        if entity.start >= entity.end {
            return Err(RagError::InvalidQuery(
                "entity span must have start < end".to_owned(),
            ));
        }
        if entity.normalized_value.trim().is_empty() {
            return Err(RagError::InvalidQuery(
                "entity normalized_value must not be empty".to_owned(),
            ));
        }
        if entity.model_version.trim().is_empty() {
            return Err(RagError::InvalidQuery(
                "entity model_version must not be empty".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_confidence(value: f32, field: &str) -> RagResult<()> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(RagError::InvalidQuery(format!(
                "{field} confidence must be finite and between 0 and 1"
            )));
        }
        Ok(())
    }

    pub(crate) fn insert_entities(
        &self,
        collection: &str,
        entities: &[GraphEntity],
    ) -> RagResult<()> {
        for entity in entities {
            Self::validate_entity(entity)?;
        }
        let mut data = self
            .collections
            .write()
            .map_err(|_| RagError::InvalidQuery("graph store lock poisoned".to_owned()))?;
        let graph = data.entry(collection.to_owned()).or_default();
        for entity in entities {
            graph.entities.insert(entity.id.clone(), entity.clone());
        }
        Ok(())
    }

    pub(crate) fn insert_relations(
        &self,
        collection: &str,
        relations: &[GraphRelation],
    ) -> RagResult<()> {
        for relation in relations {
            relation.evidence.validate()?;
            Self::validate_confidence(relation.confidence, "relation")?;
            if relation.ontology_version.trim().is_empty() {
                return Err(RagError::InvalidQuery(
                    "relation ontology_version must not be empty".to_owned(),
                ));
            }
        }
        let mut data = self
            .collections
            .write()
            .map_err(|_| RagError::InvalidQuery("graph store lock poisoned".to_owned()))?;
        let graph = data.entry(collection.to_owned()).or_default();
        for relation in relations {
            let Some(subject) = graph.entities.get(&relation.subject) else {
                return Err(RagError::InvalidQuery(format!(
                    "relation subject not found: {}",
                    relation.subject.0
                )));
            };
            let Some(object) = graph.entities.get(&relation.object) else {
                return Err(RagError::InvalidQuery(format!(
                    "relation object not found: {}",
                    relation.object.0
                )));
            };
            if subject.document_id != relation.evidence.document_id
                || object.document_id != relation.evidence.document_id
            {
                return Err(RagError::InvalidQuery(
                    "relation evidence document does not match both endpoints".to_owned(),
                ));
            }
            if effective_tenant(subject.tenant_id.as_deref())
                != effective_tenant(object.tenant_id.as_deref())
            {
                return Err(RagError::InvalidQuery(
                    "relation endpoints belong to different tenants".to_owned(),
                ));
            }
        }
        for relation in relations {
            let key = (
                relation.subject.clone(),
                relation.predicate,
                relation.object.clone(),
                relation.evidence.text_hash.clone(),
            );
            graph.relations.insert(key, relation.clone());
        }
        Ok(())
    }

    #[cfg(feature = "sqlite-graph")]
    pub(crate) fn entity_tenant(
        &self,
        collection: &str,
        entity_id: &EntityId,
    ) -> RagResult<Option<String>> {
        let data = self
            .collections
            .read()
            .map_err(|_| RagError::InvalidQuery("graph store lock poisoned".to_owned()))?;
        Ok(data
            .get(collection)
            .and_then(|graph| graph.entities.get(entity_id))
            .map(|entity| effective_tenant(entity.tenant_id.as_deref()).to_owned()))
    }
}

#[async_trait]
impl GraphStore for InMemoryGraphStore {
    async fn upsert_entities(
        &self,
        collection: &str,
        tenant_id: &str,
        entities: &[GraphEntity],
    ) -> RagResult<()> {
        for entity in entities {
            Self::validate_entity(entity)?;
            if effective_tenant(entity.tenant_id.as_deref()) != tenant_id {
                return Err(RagError::InvalidQuery(format!(
                    "entity {} belongs to tenant {}, expected {}",
                    entity.id.0,
                    effective_tenant(entity.tenant_id.as_deref()),
                    tenant_id
                )));
            }
        }
        self.insert_entities(collection, entities)
    }

    async fn upsert_relations(
        &self,
        collection: &str,
        tenant_id: &str,
        relations: &[GraphRelation],
    ) -> RagResult<()> {
        for relation in relations {
            relation.evidence.validate()?;
            Self::validate_confidence(relation.confidence, "relation")?;
            if relation.ontology_version.trim().is_empty() {
                return Err(RagError::InvalidQuery(
                    "relation ontology_version must not be empty".to_owned(),
                ));
            }
            if effective_tenant(relation.tenant_id.as_deref()) != tenant_id {
                return Err(RagError::InvalidQuery(format!(
                    "relation {}->{} belongs to tenant {}, expected {}",
                    relation.subject.0,
                    relation.object.0,
                    effective_tenant(relation.tenant_id.as_deref()),
                    tenant_id
                )));
            }
        }
        self.insert_relations(collection, relations)
    }

    async fn neighborhood(
        &self,
        collection: &str,
        tenant_id: &str,
        entity_ids: &[EntityId],
        hops: u8,
        limit: usize,
        min_confidence: f32,
    ) -> RagResult<GraphContext> {
        if entity_ids.is_empty()
            || entity_ids.len() > MAX_ENTITY_IDS
            || limit == 0
            || !(1..=2).contains(&hops)
        {
            return Err(RagError::InvalidQuery(format!(
                "neighborhood requires 1..={MAX_ENTITY_IDS} ids, limit > 0 and hops in 1..=2"
            )));
        }
        Self::validate_confidence(min_confidence, "minimum")?;
        let data = self
            .collections
            .read()
            .map_err(|_| RagError::InvalidQuery("graph store lock poisoned".to_owned()))?;
        let Some(graph) = data.get(collection) else {
            return Ok(GraphContext::default());
        };
        let mut seen = HashSet::new();
        let mut queue: VecDeque<(EntityId, u8)> = entity_ids
            .iter()
            .filter(|id| {
                graph.entities.get(*id).is_some_and(|entity| {
                    effective_tenant(entity.tenant_id.as_deref()) == tenant_id
                })
            })
            .cloned()
            .map(|id| (id, 0))
            .collect();
        let mut relation_keys = HashSet::new();
        while let Some((current, depth)) = queue.pop_front() {
            if !seen.insert(current.clone()) || depth >= hops {
                continue;
            }
            for (key, relation) in &graph.relations {
                if relation.confidence < min_confidence {
                    continue;
                }
                let Some(subject) = graph.entities.get(&key.0) else {
                    continue;
                };
                let Some(object) = graph.entities.get(&key.2) else {
                    continue;
                };
                // Upserting an entity can change the tenant behind an existing
                // relation. Re-check both endpoints at read time so stale edges
                // cannot bridge tenants even if they were valid when inserted.
                if effective_tenant(subject.tenant_id.as_deref()) != tenant_id
                    || effective_tenant(object.tenant_id.as_deref()) != tenant_id
                    || subject.document_id != relation.evidence.document_id
                    || object.document_id != relation.evidence.document_id
                {
                    continue;
                }
                let next = if key.0 == current {
                    &key.2
                } else if key.2 == current {
                    &key.0
                } else {
                    continue;
                };
                relation_keys.insert(key.clone());
                if !seen.contains(next) {
                    queue.push_back((next.clone(), depth + 1));
                }
                if relation_keys.len() >= limit {
                    break;
                }
            }
            if relation_keys.len() >= limit {
                break;
            }
        }
        let mut relations = relation_keys
            .into_iter()
            .filter_map(|key| graph.relations.get(&key).cloned())
            .collect::<Vec<_>>();
        relations.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
        relations.truncate(limit);
        let mut entities = seen
            .into_iter()
            .filter_map(|id| graph.entities.get(&id))
            .filter(|entity| effective_tenant(entity.tenant_id.as_deref()) == tenant_id)
            .cloned()
            .collect::<Vec<_>>();
        entities.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        Ok(GraphContext {
            entities,
            relations,
        })
    }

    async fn delete_document(
        &self,
        collection: &str,
        tenant_id: &str,
        document_id: &DocumentId,
    ) -> RagResult<u64> {
        let mut data = self
            .collections
            .write()
            .map_err(|_| RagError::InvalidQuery("graph store lock poisoned".to_owned()))?;
        let Some(graph) = data.get_mut(collection) else {
            return Ok(0);
        };
        let entity_ids: HashSet<_> = graph
            .entities
            .iter()
            .filter(|(_, entity)| {
                &entity.document_id == document_id
                    && effective_tenant(entity.tenant_id.as_deref()) == tenant_id
            })
            .map(|(id, _)| id.clone())
            .collect();
        let removed_entities = entity_ids.len() as u64;
        graph.entities.retain(|id, _| !entity_ids.contains(id));
        graph.relations.retain(|key, relation| {
            !entity_ids.contains(&key.0)
                && !entity_ids.contains(&key.2)
                && relation.evidence.document_id != *document_id
        });
        Ok(removed_entities)
    }

    async fn health(&self, tenant_id: &str) -> RagResult<GraphHealth> {
        let data = self
            .collections
            .read()
            .map_err(|_| RagError::InvalidQuery("graph store lock poisoned".to_owned()))?;
        let (entity_count, relation_count) = data
            .values()
            .fold((0, 0), |(entities, relations), graph| {
                let tenant_entities = graph
                    .entities
                    .values()
                    .filter(|e| effective_tenant(e.tenant_id.as_deref()) == tenant_id)
                    .count();
                let tenant_relations = graph
                    .relations
                    .values()
                    .filter(|r| effective_tenant(r.tenant_id.as_deref()) == tenant_id)
                    .count();
                (
                    entities + tenant_entities,
                    relations + tenant_relations,
                )
            });
        Ok(GraphHealth {
            ready: true,
            entity_count,
            relation_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(id: &str, document_id: &str, start: u32) -> GraphEntity {
        GraphEntity {
            id: EntityId(id.to_owned()),
            label: EntityLabel::Organization,
            normalized_value: id.to_owned(),
            document_id: DocumentId(document_id.to_owned()),
            chunk_id: None,
            start,
            end: start + 3,
            confidence: 0.99,
            model_version: "gliner-2.5".to_owned(),
            tenant_id: None,
            properties: serde_json::Value::Null,
        }
    }

    fn relation(
        subject: &str,
        object: &str,
        document_id: &str,
        tenant_id: Option<&str>,
    ) -> GraphRelation {
        GraphRelation {
            subject: EntityId(subject.to_owned()),
            predicate: RelationType::References,
            object: EntityId(object.to_owned()),
            evidence: EvidenceSpan {
                document_id: DocumentId(document_id.to_owned()),
                chunk_id: None,
                start: 0,
                end: 10,
                text_hash: format!("hash-{subject}-{object}"),
            },
            confidence: 0.95,
            ontology_version: "legal-v1".to_owned(),
            tenant_id: tenant_id.map(|t| t.to_owned()),
        }
    }

    #[tokio::test]
    async fn accepts_valid_relations_and_returns_bounded_neighborhood() {
        let store = InMemoryGraphStore::default();
        store
            .upsert_entities("contracts", DEFAULT_TENANT, &[entity("a", "doc", 0), entity("b", "doc", 4)])
            .await
            .unwrap();
        store
            .upsert_relations("contracts", DEFAULT_TENANT, &[relation("a", "b", "doc", None)])
            .await
            .unwrap();

        let context = store
            .neighborhood(
                "contracts",
                DEFAULT_TENANT,
                &[EntityId("a".to_owned())],
                1,
                10,
                0.9,
            )
            .await
            .unwrap();
        assert_eq!(context.entities.len(), 2);
        assert_eq!(context.relations.len(), 1);
    }

    #[tokio::test]
    async fn rejects_dangling_relation_and_cross_document_evidence() {
        let store = InMemoryGraphStore::default();
        store
            .upsert_entities("contracts", DEFAULT_TENANT, &[entity("a", "doc-a", 0)])
            .await
            .unwrap();
        let dangling = store
            .upsert_relations("contracts", DEFAULT_TENANT, &[relation("a", "missing", "doc-a", None)])
            .await;
        assert!(matches!(dangling, Err(RagError::InvalidQuery(_))));

        store
            .upsert_entities("contracts", DEFAULT_TENANT, &[entity("b", "doc-b", 0)])
            .await
            .unwrap();
        let cross_doc = store
            .upsert_relations("contracts", DEFAULT_TENANT, &[relation("a", "b", "doc-a", None)])
            .await;
        assert!(matches!(cross_doc, Err(RagError::InvalidQuery(_))));
    }

    #[tokio::test]
    async fn batch_upserts_are_atomic_on_validation_failure() {
        let store = InMemoryGraphStore::default();
        let mut invalid = entity("invalid", "doc", 4);
        invalid.end = invalid.start;
        let result = store
            .upsert_entities("contracts", DEFAULT_TENANT, &[entity("valid", "doc", 0), invalid])
            .await;
        assert!(matches!(result, Err(RagError::InvalidQuery(_))));
        assert_eq!(store.health(DEFAULT_TENANT).await.unwrap().entity_count, 0);

        store
            .upsert_entities("contracts", DEFAULT_TENANT, &[entity("a", "doc", 0), entity("b", "doc", 4)])
            .await
            .unwrap();
        let result = store
            .upsert_relations(
                "contracts",
                DEFAULT_TENANT,
                &[relation("a", "b", "doc", None), relation("a", "missing", "doc", None)],
            )
            .await;
        assert!(matches!(result, Err(RagError::InvalidQuery(_))));
        assert_eq!(store.health(DEFAULT_TENANT).await.unwrap().relation_count, 0);
    }

    #[tokio::test]
    async fn deleting_document_removes_entities_and_relations() {
        let store = InMemoryGraphStore::default();
        store
            .upsert_entities("contracts", DEFAULT_TENANT, &[entity("a", "doc", 0), entity("b", "doc", 4)])
            .await
            .unwrap();
        store
            .upsert_relations("contracts", DEFAULT_TENANT, &[relation("a", "b", "doc", None)])
            .await
            .unwrap();
        assert_eq!(
            store
                .delete_document("contracts", DEFAULT_TENANT, &DocumentId("doc".to_owned()))
                .await
                .unwrap(),
            2
        );
        let health = store.health(DEFAULT_TENANT).await.unwrap();
        assert_eq!(health.entity_count, 0);
        assert_eq!(health.relation_count, 0);
    }

    #[tokio::test]
    async fn neighborhood_never_crosses_tenant_boundary() {
        let store = InMemoryGraphStore::default();
        let mut private = entity("private", "doc-private", 0);
        private.tenant_id = Some("acme".to_owned());
        let mut shared = entity("shared", "doc-shared", 4);
        shared.tenant_id = Some("globex".to_owned());
        store
            .upsert_entities("contracts", "acme", &[private])
            .await
            .unwrap();
        store
            .upsert_entities("contracts", "globex", &[shared])
            .await
            .unwrap();
        let cross_tenant = store
            .upsert_relations("contracts", "acme", &[relation("private", "shared", "doc-private", Some("acme"))])
            .await;
        assert!(matches!(cross_tenant, Err(RagError::InvalidQuery(_))));

        let context = store
            .neighborhood(
                "contracts",
                "globex",
                &[EntityId("private".to_owned())],
                1,
                10,
                0.9,
            )
            .await
            .unwrap();
        assert!(context.entities.is_empty());
        assert!(context.relations.is_empty());
    }

    #[tokio::test]
    async fn neighborhood_rechecks_tenant_after_endpoint_upsert() {
        let store = InMemoryGraphStore::default();
        let mut subject = entity("subject", "doc", 0);
        subject.tenant_id = Some("acme".to_owned());
        let mut object = entity("object", "doc", 4);
        object.tenant_id = Some("acme".to_owned());
        store
            .upsert_entities("contracts", "acme", &[subject, object.clone()])
            .await
            .unwrap();
        store
            .upsert_relations("contracts", "acme", &[relation("subject", "object", "doc", Some("acme"))])
            .await
            .unwrap();

        object.tenant_id = Some("globex".to_owned());
        store.upsert_entities("contracts", "globex", &[object]).await.unwrap();

        let context = store
            .neighborhood(
                "contracts",
                "acme",
                &[EntityId("subject".to_owned())],
                1,
                10,
                0.9,
            )
            .await
            .unwrap();
        assert_eq!(context.entities.len(), 1);
        assert_eq!(context.entities[0].id.0, "subject");
        assert!(context.relations.is_empty());
    }

    #[test]
    fn graph_rag_query_rejects_invalid_bounds_before_retrieval() {
        let query = GraphRagQuery {
            retrieval: RetrieveQuery::vector(1),
            entity_ids: vec![EntityId("e".to_owned())],
            hops: 3,
            graph_limit: 10,
            graph_min_confidence: 0.9,
        };
        assert!(matches!(
            query.validate(),
            Err(RagError::InvalidQuery(message)) if message.contains("hops")
        ));
    }

    #[test]
    fn graph_rag_query_caps_entity_ids_at_one_hundred() {
        let mut query = GraphRagQuery {
            retrieval: RetrieveQuery::vector(1),
            entity_ids: (0..MAX_ENTITY_IDS)
                .map(|i| EntityId(format!("e-{i}")))
                .collect(),
            hops: 1,
            graph_limit: 10,
            graph_min_confidence: 0.9,
        };
        assert!(query.validate().is_ok(), "exactly 100 ids must be accepted");

        query.entity_ids.push(EntityId("one-too-many".to_owned()));
        assert!(matches!(
            query.validate(),
            Err(RagError::InvalidQuery(message)) if message.contains("100")
        ));
    }

    #[tokio::test]
    async fn in_memory_neighborhood_caps_entity_ids_at_one_hundred() {
        let store = InMemoryGraphStore::default();
        let ids = (0..=MAX_ENTITY_IDS)
            .map(|i| EntityId(format!("e-{i}")))
            .collect::<Vec<_>>();
        let err = store
            .neighborhood("contracts", DEFAULT_TENANT, &ids, 1, 10, 0.9)
            .await
            .expect_err("101 ids must be refused before traversal");
        assert!(matches!(err, RagError::InvalidQuery(_)));
    }

    /// Concurrent upserts and neighborhood queries must not poison the lock
    /// or leak entities across tenants.
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_upsert_and_neighborhood_stays_tenant_isolated() {
        use std::sync::Arc;

        let store = Arc::new(InMemoryGraphStore::default());
        let mut handles = vec![];

        // Writer: upsert default-tenant entities/relations in a loop.
        for i in 0..20u32 {
            let s = Arc::clone(&store);
            handles.push(tokio::spawn(async move {
                let e1 = EntityId(format!("e-{i}"));
                let e2 = EntityId(format!("e-{i}-b"));
                s.upsert_entities(
                    "contracts",
                    DEFAULT_TENANT,
                    &[
                        GraphEntity {
                            id: e1.clone(),
                            label: EntityLabel::Organization,
                            normalized_value: format!("doc-{i}"),
                            document_id: DocumentId("doc-default".to_owned()),
                            chunk_id: None,
                            start: 0,
                            end: 5,
                            confidence: 0.9,
                            model_version: "gliner-2.5".to_owned(),
                            tenant_id: None,
                            properties: serde_json::Value::Null,
                        },
                        GraphEntity {
                            id: e2.clone(),
                            label: EntityLabel::Organization,
                            normalized_value: format!("doc-{i}-b"),
                            document_id: DocumentId("doc-default".to_owned()),
                            chunk_id: None,
                            start: 6,
                            end: 11,
                            confidence: 0.9,
                            model_version: "gliner-2.5".to_owned(),
                            tenant_id: None,
                            properties: serde_json::Value::Null,
                        },
                    ],
                )
                .await
                .unwrap();
                s.upsert_relations(
                    "contracts",
                    DEFAULT_TENANT,
                    &[GraphRelation {
                        subject: e1,
                        predicate: RelationType::References,
                        object: e2,
                        evidence: EvidenceSpan {
                            document_id: DocumentId("doc-default".to_owned()),
                            chunk_id: None,
                            start: 0,
                            end: 10,
                            text_hash: format!("hash-default-{i}"),
                        },
                        confidence: 0.95,
                        ontology_version: "legal-v1".to_owned(),
                        tenant_id: None,
                    }],
                )
                .await
                .unwrap();
            }));
        }

        // Reader: repeatedly query the default tenant's neighborhood while writers run.
        for _ in 0..20 {
            let s = Arc::clone(&store);
            handles.push(tokio::spawn(async move {
                for _ in 0..10 {
                    let _ = s
                        .neighborhood(
                            "contracts",
                            DEFAULT_TENANT,
                            &[EntityId("e-0".to_owned())],
                            1,
                            10,
                            0.9,
                        )
                        .await;
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        // After all writes, no panic/poisoning, and at least 40 entities are present.
        let ctx = store
            .neighborhood("contracts", DEFAULT_TENANT, &[EntityId("e-0".to_owned())], 1, 50, 0.9)
            .await
            .unwrap();
        for e in &ctx.entities {
            assert_eq!(
                effective_tenant(e.tenant_id.as_deref()),
                DEFAULT_TENANT,
                "no cross-tenant entity leak under concurrency"
            );
        }
        let health = store.health(DEFAULT_TENANT).await.unwrap();
        assert!(health.entity_count >= 40, "at least 40 default-tenant entities must be present");
    }
}
