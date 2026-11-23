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
/// Shared graph request bounds and wire defaults. Keeping them in the contract
/// prevents the streaming and retrieval endpoints from accepting different
/// requests. All numeric ranges below are inclusive.
/// Smallest accepted number of graph traversal hops.
pub const MIN_HOPS: u8 = 1;
/// Largest accepted number of graph traversal hops.
pub const MAX_HOPS: u8 = 2;
/// Smallest accepted neighborhood relation limit.
pub const MIN_GRAPH_LIMIT: usize = 1;
/// Largest accepted neighborhood relation limit.
pub const MAX_GRAPH_LIMIT: usize = 500;
/// Default hop count when a request omits it (a default, not a limit).
pub const DEFAULT_HOPS: u8 = 1;
/// Default relation limit when a request omits it (a default, not a limit).
pub const DEFAULT_GRAPH_LIMIT: usize = 50;
/// Default minimum confidence when a request omits it (a default, not a limit).
pub const DEFAULT_GRAPH_MIN_CONFIDENCE: f32 = 0.9;

/// A rejected graph request bound, with enough context to tell the caller
/// which field was wrong and what it accepts (never the entity values).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphBoundsError {
    /// Name of the rejected request field (e.g. `hops`), never its values.
    pub field: &'static str,
    /// The rejected count/value, rendered without any entity content.
    pub rejected: String,
    /// Human-readable accepted range or condition for that field.
    pub accepted: String,
}

impl std::fmt::Display for GraphBoundsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid graph bound '{}': expected {}, got {}",
            self.field, self.accepted, self.rejected
        )
    }
}

impl std::error::Error for GraphBoundsError {}

/// Validate every graph request bound. Callers translate this into their own
/// error type (RagError / ApiError) without duplicating the numeric limits.
pub fn validate_graph_bounds(
    entity_id_count: usize,
    hops: u8,
    limit: usize,
    min_confidence: f32,
) -> Result<(), GraphBoundsError> {
    if entity_id_count == 0 || entity_id_count > MAX_ENTITY_IDS {
        return Err(GraphBoundsError {
            field: "entity_ids",
            rejected: entity_id_count.to_string(),
            accepted: format!("between 1 and {MAX_ENTITY_IDS} ids"),
        });
    }
    if !(MIN_HOPS..=MAX_HOPS).contains(&hops) {
        return Err(GraphBoundsError {
            field: "hops",
            rejected: hops.to_string(),
            accepted: format!("between {MIN_HOPS} and {MAX_HOPS}"),
        });
    }
    if !(MIN_GRAPH_LIMIT..=MAX_GRAPH_LIMIT).contains(&limit) {
        return Err(GraphBoundsError {
            field: "limit",
            rejected: limit.to_string(),
            accepted: format!("between {MIN_GRAPH_LIMIT} and {MAX_GRAPH_LIMIT}"),
        });
    }
    if !min_confidence.is_finite() || !(0.0..=1.0).contains(&min_confidence) {
        return Err(GraphBoundsError {
            field: "min_confidence",
            rejected: min_confidence.to_string(),
            accepted: "a finite value between 0 and 1".to_owned(),
        });
    }
    Ok(())
}

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

/// Validate every caller-controlled neighborhood bound before a backend issues
/// SQL or traverses an in-memory graph. Both stores call this same helper so a
/// new backend cannot silently skip one of the checks.
pub(crate) fn validate_neighborhood_bounds(
    entity_ids: &[EntityId],
    hops: u8,
    limit: usize,
    min_confidence: f32,
) -> RagResult<()> {
    validate_graph_bounds(entity_ids.len(), hops, limit, min_confidence)
        .map_err(|error| RagError::InvalidQuery(format!("neighborhood {error}")))
}

/// Identity of an entity inside one collection. The tenant is part of the key:
/// two tenants may use the same caller-supplied `EntityId` independently.
pub(crate) type EntityKey = (String, EntityId);

pub(crate) fn entity_key(entity: &GraphEntity) -> EntityKey {
    (
        effective_tenant(entity.tenant_id.as_deref()).to_owned(),
        entity.id.clone(),
    )
}

fn relation_key(relation: &GraphRelation) -> (String, EntityId, RelationType, EntityId, String) {
    (
        effective_tenant(relation.tenant_id.as_deref()).to_owned(),
        relation.subject.clone(),
        relation.predicate,
        relation.object.clone(),
        relation.evidence.text_hash.clone(),
    )
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
        validate_graph_bounds(
            self.entity_ids.len(),
            self.hops,
            self.graph_limit,
            self.graph_min_confidence,
        )
        .map_err(|error| RagError::InvalidQuery(format!("graph retrieval {error}")))
    }
}

fn default_hops() -> u8 {
    DEFAULT_HOPS
}

fn default_graph_limit() -> usize {
    DEFAULT_GRAPH_LIMIT
}

fn default_graph_min_confidence() -> f32 {
    DEFAULT_GRAPH_MIN_CONFIDENCE
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

type RelationKey = (String, EntityId, RelationType, EntityId, String);

#[derive(Default)]
struct GraphData {
    entities: HashMap<EntityKey, GraphEntity>,
    relations: HashMap<RelationKey, GraphRelation>,
    /// Tenant-qualified endpoint -> incident relation keys. Maintained under
    /// the same write lock as `relations`, so a seed with no incident edges
    /// never scans unrelated edges.
    adjacency: HashMap<EntityKey, HashSet<RelationKey>>,
}

fn index_relation(graph: &mut GraphData, key: &RelationKey, relation: &GraphRelation) {
    let tenant = effective_tenant(relation.tenant_id.as_deref()).to_owned();
    graph
        .adjacency
        .entry((tenant.clone(), relation.subject.clone()))
        .or_default()
        .insert(key.clone());
    graph
        .adjacency
        .entry((tenant, relation.object.clone()))
        .or_default()
        .insert(key.clone());
}

fn unindex_relation(graph: &mut GraphData, key: &RelationKey, relation: &GraphRelation) {
    let tenant = effective_tenant(relation.tenant_id.as_deref()).to_owned();
    for endpoint in [&relation.subject, &relation.object] {
        let entity_key = (tenant.clone(), endpoint.clone());
        if let Some(keys) = graph.adjacency.get_mut(&entity_key) {
            keys.remove(key);
            if keys.is_empty() {
                graph.adjacency.remove(&entity_key);
            }
        }
    }
}

/// Exact, deterministic graph backend used by tests and local development.
#[derive(Default)]
pub struct InMemoryGraphStore {
    collections: RwLock<HashMap<String, GraphData>>,
}

impl InMemoryGraphStore {
    /// Validate a relation row itself, independently of its endpoints. Used by
    /// durable backends to distinguish corrupt stored rows from a stale edge
    /// whose endpoint was legitimately re-assigned to another tenant.
    pub(crate) fn validate_relation_payload(relation: &GraphRelation) -> RagResult<()> {
        relation.evidence.validate()?;
        Self::validate_confidence(relation.confidence, "relation")?;
        if relation.ontology_version.trim().is_empty() {
            return Err(RagError::InvalidQuery(
                "relation ontology_version must not be empty".to_owned(),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_entity(entity: &GraphEntity) -> RagResult<()> {
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
            graph.entities.insert(entity_key(entity), entity.clone());
        }
        Ok(())
    }

    pub(crate) fn insert_relations(
        &self,
        collection: &str,
        relations: &[GraphRelation],
    ) -> RagResult<()> {
        for relation in relations {
            Self::validate_relation_payload(relation)?;
        }
        let mut data = self
            .collections
            .write()
            .map_err(|_| RagError::InvalidQuery("graph store lock poisoned".to_owned()))?;
        let graph = data.entry(collection.to_owned()).or_default();
        for relation in relations {
            let tenant = effective_tenant(relation.tenant_id.as_deref());
            let Some(subject) = graph
                .entities
                .get(&(tenant.to_owned(), relation.subject.clone()))
            else {
                return Err(RagError::InvalidQuery(format!(
                    "relation subject not found: {}",
                    relation.subject.0
                )));
            };
            let Some(object) = graph
                .entities
                .get(&(tenant.to_owned(), relation.object.clone()))
            else {
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
            // All three tenants must agree. Checking only the two endpoints
            // would let tenant A write an edge it does not own over B's nodes.
            if effective_tenant(subject.tenant_id.as_deref()) != tenant
                || effective_tenant(object.tenant_id.as_deref()) != tenant
            {
                return Err(RagError::InvalidQuery(
                    "relation owner and both endpoints must belong to the same tenant".to_owned(),
                ));
            }
        }
        for relation in relations {
            let key = relation_key(relation);
            if let Some(previous) = graph.relations.insert(key.clone(), relation.clone()) {
                unindex_relation(graph, &key, &previous);
            }
            index_relation(graph, &key, relation);
        }
        Ok(())
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
            Self::validate_relation_payload(relation)?;
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
        validate_neighborhood_bounds(entity_ids, hops, limit, min_confidence)?;
        let data = self
            .collections
            .read()
            .map_err(|_| RagError::InvalidQuery("graph store lock poisoned".to_owned()))?;
        let Some(graph) = data.get(collection) else {
            return Ok(GraphContext::default());
        };
        let mut seen: HashSet<EntityKey> = HashSet::new();
        let mut queue: VecDeque<(EntityKey, u8)> = entity_ids
            .iter()
            .map(|id| (tenant_id.to_owned(), id.clone()))
            .filter(|key| graph.entities.contains_key(key))
            .map(|key| (key, 0))
            .collect();
        let mut relation_keys = HashSet::new();
        while let Some((current, depth)) = queue.pop_front() {
            if !seen.insert(current.clone()) || depth >= hops {
                continue;
            }
            // Borrow the index: cloning the whole incident set for every seed
            // would defeat the point of having it. Only selected keys are
            // cloned into `relation_keys` below.
            let incident = graph.adjacency.get(&current);
            for key in incident.into_iter().flatten() {
                let Some(relation) = graph.relations.get(key) else {
                    continue;
                };
                // Relation identity and owner are tenant-qualified: never follow
                // an edge belonging to a different tenant, even if its ids match.
                if key.0 != tenant_id
                    || effective_tenant(relation.tenant_id.as_deref()) != tenant_id
                    || relation.confidence < min_confidence
                {
                    continue;
                }
                let Some(subject) = graph.entities.get(&(tenant_id.to_owned(), key.1.clone()))
                else {
                    continue;
                };
                let Some(object) = graph.entities.get(&(tenant_id.to_owned(), key.3.clone()))
                else {
                    continue;
                };
                // An endpoint can later be upserted with a different tenant or
                // document. Re-check at read time so a stale edge cannot bridge
                // tenants or sources.
                if effective_tenant(subject.tenant_id.as_deref()) != tenant_id
                    || effective_tenant(object.tenant_id.as_deref()) != tenant_id
                    || subject.document_id != relation.evidence.document_id
                    || object.document_id != relation.evidence.document_id
                {
                    continue;
                }
                let next = if key.1 == current.1 {
                    (tenant_id.to_owned(), key.3.clone())
                } else if key.3 == current.1 {
                    (tenant_id.to_owned(), key.1.clone())
                } else {
                    continue;
                };
                relation_keys.insert(key.clone());
                if !seen.contains(&next) {
                    queue.push_back((next, depth + 1));
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
            .filter(|key| key.0 == tenant_id)
            .filter_map(|key| graph.entities.get(&key))
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
        let entity_ids: HashSet<EntityKey> = graph
            .entities
            .iter()
            .filter(|(key, entity)| {
                key.0 == tenant_id
                    && &entity.document_id == document_id
                    && effective_tenant(entity.tenant_id.as_deref()) == tenant_id
            })
            .map(|(key, _)| key.clone())
            .collect();
        let removed_entities = entity_ids.len() as u64;
        graph.entities.retain(|key, _| !entity_ids.contains(key));
        let removed_relations = graph
            .relations
            .iter()
            .filter(|(key, relation)| {
                // An edge owned by another tenant is never removed by this
                // tenant's document deletion, even when its evidence names the
                // same document.
                if key.0 != tenant_id
                    || effective_tenant(relation.tenant_id.as_deref()) != tenant_id
                {
                    return false;
                }
                let endpoint_removed = entity_ids.contains(&(tenant_id.to_owned(), key.1.clone()))
                    || entity_ids.contains(&(tenant_id.to_owned(), key.3.clone()));
                endpoint_removed || relation.evidence.document_id == *document_id
            })
            .map(|(key, relation)| (key.clone(), relation.clone()))
            .collect::<Vec<_>>();
        for (key, _) in &removed_relations {
            graph.relations.remove(key);
        }
        for (key, relation) in &removed_relations {
            unindex_relation(graph, key, relation);
        }
        Ok(removed_entities)
    }

    async fn health(&self, tenant_id: &str) -> RagResult<GraphHealth> {
        let data = self
            .collections
            .read()
            .map_err(|_| RagError::InvalidQuery("graph store lock poisoned".to_owned()))?;
        let (entity_count, relation_count) =
            data.values().fold((0, 0), |(entities, relations), graph| {
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
                (entities + tenant_entities, relations + tenant_relations)
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
            .upsert_entities(
                "contracts",
                DEFAULT_TENANT,
                &[entity("a", "doc", 0), entity("b", "doc", 4)],
            )
            .await
            .unwrap();
        store
            .upsert_relations(
                "contracts",
                DEFAULT_TENANT,
                &[relation("a", "b", "doc", None)],
            )
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
            .upsert_relations(
                "contracts",
                DEFAULT_TENANT,
                &[relation("a", "missing", "doc-a", None)],
            )
            .await;
        assert!(matches!(dangling, Err(RagError::InvalidQuery(_))));

        store
            .upsert_entities("contracts", DEFAULT_TENANT, &[entity("b", "doc-b", 0)])
            .await
            .unwrap();
        let cross_doc = store
            .upsert_relations(
                "contracts",
                DEFAULT_TENANT,
                &[relation("a", "b", "doc-a", None)],
            )
            .await;
        assert!(matches!(cross_doc, Err(RagError::InvalidQuery(_))));
    }

    #[tokio::test]
    async fn batch_upserts_are_atomic_on_validation_failure() {
        let store = InMemoryGraphStore::default();
        let mut invalid = entity("invalid", "doc", 4);
        invalid.end = invalid.start;
        let result = store
            .upsert_entities(
                "contracts",
                DEFAULT_TENANT,
                &[entity("valid", "doc", 0), invalid],
            )
            .await;
        assert!(matches!(result, Err(RagError::InvalidQuery(_))));
        assert_eq!(store.health(DEFAULT_TENANT).await.unwrap().entity_count, 0);

        store
            .upsert_entities(
                "contracts",
                DEFAULT_TENANT,
                &[entity("a", "doc", 0), entity("b", "doc", 4)],
            )
            .await
            .unwrap();
        let result = store
            .upsert_relations(
                "contracts",
                DEFAULT_TENANT,
                &[
                    relation("a", "b", "doc", None),
                    relation("a", "missing", "doc", None),
                ],
            )
            .await;
        assert!(matches!(result, Err(RagError::InvalidQuery(_))));
        assert_eq!(
            store.health(DEFAULT_TENANT).await.unwrap().relation_count,
            0
        );
    }

    #[tokio::test]
    async fn deleting_document_removes_entities_and_relations() {
        let store = InMemoryGraphStore::default();
        store
            .upsert_entities(
                "contracts",
                DEFAULT_TENANT,
                &[entity("a", "doc", 0), entity("b", "doc", 4)],
            )
            .await
            .unwrap();
        store
            .upsert_relations(
                "contracts",
                DEFAULT_TENANT,
                &[relation("a", "b", "doc", None)],
            )
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
            .upsert_relations(
                "contracts",
                "acme",
                &[relation("private", "shared", "doc-private", Some("acme"))],
            )
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
    async fn tenant_qualified_identities_stay_independent() {
        let store = InMemoryGraphStore::default();
        let mut acme_subject = entity("subject", "doc-acme", 0);
        acme_subject.tenant_id = Some("acme".to_owned());
        let mut acme_object = entity("object", "doc-acme", 4);
        acme_object.tenant_id = Some("acme".to_owned());
        let mut globex_object = entity("object", "doc-globex", 4);
        globex_object.tenant_id = Some("globex".to_owned());
        globex_object.normalized_value = "globex-copy".to_owned();

        store
            .upsert_entities("contracts", "acme", &[acme_subject, acme_object])
            .await
            .unwrap();
        // The same EntityId under another tenant must not overwrite acme's row.
        store
            .upsert_entities("contracts", "globex", &[globex_object])
            .await
            .unwrap();
        store
            .upsert_relations(
                "contracts",
                "acme",
                &[relation("subject", "object", "doc-acme", Some("acme"))],
            )
            .await
            .unwrap();

        let acme = store
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
        assert_eq!(acme.entities.len(), 2);
        assert_eq!(acme.relations.len(), 1);
        assert!(acme
            .entities
            .iter()
            .any(|e| e.id.0 == "object" && e.normalized_value == "object"));

        let globex = store
            .neighborhood(
                "contracts",
                "globex",
                &[EntityId("object".to_owned())],
                1,
                10,
                0.9,
            )
            .await
            .unwrap();
        assert_eq!(globex.entities.len(), 1);
        assert_eq!(globex.entities[0].normalized_value, "globex-copy");
        assert!(globex.relations.is_empty(), "acme's edge must not leak");
    }

    #[tokio::test]
    async fn relation_owner_must_match_both_endpoints() {
        let store = InMemoryGraphStore::default();
        let mut a = entity("a", "doc", 0);
        a.tenant_id = Some("acme".to_owned());
        let mut b = entity("b", "doc", 4);
        b.tenant_id = Some("acme".to_owned());
        store
            .upsert_entities("contracts", "acme", &[a, b])
            .await
            .unwrap();
        let forged = relation("a", "b", "doc", Some("globex"));
        let result = store
            .upsert_relations("contracts", "globex", &[forged])
            .await;
        assert!(
            matches!(result, Err(RagError::InvalidQuery(_))),
            "a foreign owner must not write an edge between acme endpoints"
        );
        assert_eq!(store.health("acme").await.unwrap().relation_count, 0);
    }

    #[tokio::test]
    async fn delete_document_only_removes_the_callers_edges() {
        let store = InMemoryGraphStore::default();
        let mut acme_a = entity("a", "doc", 0);
        acme_a.tenant_id = Some("acme".to_owned());
        let mut acme_b = entity("b", "doc", 4);
        acme_b.tenant_id = Some("acme".to_owned());
        let mut globex_edge = relation("a", "b", "doc", Some("globex"));
        globex_edge.evidence.text_hash = "globex-hash".to_owned();
        store
            .upsert_entities("contracts", "acme", &[acme_a, acme_b.clone()])
            .await
            .unwrap();
        let mut globex_a = acme_b.clone();
        globex_a.tenant_id = Some("globex".to_owned());
        let mut globex_b = acme_b.clone();
        globex_b.id = EntityId("b2".to_owned());
        globex_b.tenant_id = Some("globex".to_owned());
        store
            .upsert_entities("contracts", "globex", &[globex_a, globex_b])
            .await
            .unwrap();
        globex_edge.subject = EntityId("b".to_owned());
        globex_edge.object = EntityId("b2".to_owned());
        store
            .upsert_relations("contracts", "globex", &[globex_edge])
            .await
            .unwrap();

        store
            .delete_document("contracts", "acme", &DocumentId("doc".to_owned()))
            .await
            .unwrap();
        assert_eq!(store.health("acme").await.unwrap().relation_count, 0);
        assert_eq!(store.health("globex").await.unwrap().relation_count, 1);
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
            .neighborhood(
                "contracts",
                DEFAULT_TENANT,
                &[EntityId("e-0".to_owned())],
                1,
                50,
                0.9,
            )
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
        assert!(
            health.entity_count >= 40,
            "at least 40 default-tenant entities must be present"
        );
    }
}
