//! Durable SQLite [`GraphStore`](crate::GraphStore) for embedded deployments.
//!
//! The graph contract stays independent from SQLite. This adapter stores the
//! typed graph records as JSON plus indexed identity columns, which keeps the
//! schema migration surface small while preserving the exact provenance and
//! ontology payloads. It is intentionally feature-gated for Tauri/local use.

use crate::error::{RagError, RagResult};
use crate::graph::{
    EntityId, GraphContext, GraphEntity, GraphHealth, GraphRelation, GraphStore,
    InMemoryGraphStore, MAX_ENTITY_IDS, effective_tenant,
};
use crate::types::DocumentId;
use async_trait::async_trait;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

fn backend(error: rusqlite::Error) -> RagError {
    RagError::Backend(Box::new(error))
}

fn relation_tenant(
    memory: &InMemoryGraphStore,
    collection: &str,
    subject: &EntityId,
) -> RagResult<String> {
    memory
        .entity_tenant(collection, subject)?
        .ok_or_else(|| RagError::InvalidQuery("relation subject not found".to_owned()))
}

/// SQLite-backed graph store. One connection is serialized behind a mutex;
/// SQLite remains the source of truth and callers may share this type across
/// async tasks safely.
pub struct SqliteGraphStore {
    connection: Mutex<Connection>,
}

impl SqliteGraphStore {
    /// Open or create a graph database at `path`. Use `":memory:"` in tests.
    pub fn open(path: impl AsRef<Path>) -> RagResult<Self> {
        let connection = Connection::open(path).map_err(backend)?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS graph_entities (
                     collection TEXT NOT NULL,
                     entity_id TEXT NOT NULL,
                     document_id TEXT NOT NULL,
                     tenant_id TEXT,
                     payload TEXT NOT NULL,
                     PRIMARY KEY (collection, entity_id)
                 );
                 CREATE INDEX IF NOT EXISTS graph_entities_scope
                     ON graph_entities (collection, tenant_id, document_id);
                 CREATE TABLE IF NOT EXISTS graph_relations (
                     collection TEXT NOT NULL,
                     subject_id TEXT NOT NULL,
                     predicate TEXT NOT NULL,
                     object_id TEXT NOT NULL,
                     tenant_id TEXT NOT NULL,
                     evidence_hash TEXT NOT NULL,
                     payload TEXT NOT NULL,
                     PRIMARY KEY (collection, subject_id, predicate, object_id, evidence_hash)
                 );
                 CREATE INDEX IF NOT EXISTS graph_relations_collection
                     ON graph_relations (collection);",
            )
            .map_err(backend)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn lock(&self) -> RagResult<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| RagError::InvalidQuery("sqlite graph lock poisoned".to_owned()))
    }

    fn load_memory(&self, collection: &str) -> RagResult<InMemoryGraphStore> {
        let connection = self.lock()?;
        let memory = InMemoryGraphStore::default();
        {
            let mut statement = connection
                .prepare("SELECT payload FROM graph_entities WHERE collection = ?1")
                .map_err(backend)?;
            let rows = statement
                .query_map([collection], |row| row.get::<_, String>(0))
                .map_err(backend)?;
            let entities = rows
                .map(|row| {
                    let payload = row.map_err(backend)?;
                    serde_json::from_str::<GraphEntity>(&payload)
                        .map_err(|error| RagError::Backend(Box::new(error)))
                })
                .collect::<RagResult<Vec<_>>>()?;
            memory.insert_entities(collection, &entities)?;
        }
        {
            let mut statement = connection
                .prepare("SELECT payload FROM graph_relations WHERE collection = ?1")
                .map_err(backend)?;
            let rows = statement
                .query_map([collection], |row| row.get::<_, String>(0))
                .map_err(backend)?;
            let relations = rows
                .map(|row| {
                    let payload = row.map_err(backend)?;
                    serde_json::from_str::<GraphRelation>(&payload)
                        .map_err(|error| RagError::Backend(Box::new(error)))
                })
                .collect::<RagResult<Vec<_>>>()?;
            // A relation can become stale when an endpoint is later upserted
            // with a different tenant/document, or deleted: `SqliteGraphStore`
            // updates the entity payload in place but keeps the old edge.
            // `insert_relations` is the strict gate for writes; on this read
            // path skip such edges instead of failing the whole collection —
            // the in-memory `neighborhood` applies the same re-check at read
            // time so a stale edge can never bridge tenants.
            for relation in relations {
                if memory.insert_relations(collection, &[relation]).is_err() {
                    continue;
                }
            }
        }
        Ok(memory)
    }
}

#[async_trait]
impl GraphStore for SqliteGraphStore {
    async fn upsert_entities(
        &self,
        collection: &str,
        tenant_id: &str,
        entities: &[GraphEntity],
    ) -> RagResult<()> {
        for entity in entities {
            if effective_tenant(entity.tenant_id.as_deref()) != tenant_id {
                return Err(RagError::InvalidQuery(format!(
                    "entity {} belongs to tenant {}, expected {}",
                    entity.id.0,
                    effective_tenant(entity.tenant_id.as_deref()),
                    tenant_id
                )));
            }
        }
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction().map_err(backend)?;
        for entity in entities {
            let payload = serde_json::to_string(entity)
                .map_err(|error| RagError::Backend(Box::new(error)))?;
            transaction
                .execute(
                    "INSERT INTO graph_entities
                     (collection, entity_id, document_id, tenant_id, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(collection, entity_id) DO UPDATE SET
                       document_id = excluded.document_id,
                       tenant_id = excluded.tenant_id,
                       payload = excluded.payload",
                    params![
                        collection,
                        &entity.id.0,
                        &entity.document_id.0,
                        &entity.tenant_id,
                        payload
                    ],
                )
                .map_err(backend)?;
        }
        transaction.commit().map_err(backend)
    }

    async fn upsert_relations(
        &self,
        collection: &str,
        tenant_id: &str,
        relations: &[GraphRelation],
    ) -> RagResult<()> {
        for relation in relations {
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
        let memory = self.load_memory(collection)?;
        memory.insert_relations(collection, relations)?;
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction().map_err(backend)?;
        for relation in relations {
            let payload = serde_json::to_string(relation)
                .map_err(|error| RagError::Backend(Box::new(error)))?;
            transaction
                .execute(
                    "INSERT INTO graph_relations
                     (collection, subject_id, predicate, object_id, tenant_id, evidence_hash, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(collection, subject_id, predicate, object_id, evidence_hash)
                     DO UPDATE SET payload = excluded.payload",
                    params![
                        collection,
                        &relation.subject.0,
                        serde_json::to_string(&relation.predicate)
                            .map_err(|error| RagError::Backend(Box::new(error)))?,
                        &relation.object.0,
                        relation_tenant(&memory, collection, &relation.subject)?,
                        &relation.evidence.text_hash,
                        payload
                    ],
                )
                .map_err(backend)?;
        }
        transaction.commit().map_err(backend)
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
        // Validate the caller-controlled seed set before `load_memory` copies
        // the whole collection into RAM. The in-memory backend re-checks the
        // same bound on its own; this guard keeps the expensive load gated.
        if entity_ids.is_empty() || entity_ids.len() > MAX_ENTITY_IDS {
            return Err(RagError::InvalidQuery(format!(
                "neighborhood requires between 1 and {MAX_ENTITY_IDS} entity_ids"
            )));
        }
        let memory = self.load_memory(collection)?;
        memory
            .neighborhood(
                collection,
                tenant_id,
                entity_ids,
                hops,
                limit,
                min_confidence,
            )
            .await
    }

    async fn delete_document(
        &self,
        collection: &str,
        tenant_id: &str,
        document_id: &DocumentId,
    ) -> RagResult<u64> {
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction().map_err(backend)?;
        let removed = transaction
            .execute(
                "DELETE FROM graph_entities
                 WHERE collection = ?1 AND document_id = ?2
                   AND (tenant_id = ?3 OR (tenant_id IS NULL AND ?3 = 'default'))",
                params![collection, &document_id.0, tenant_id],
            )
            .map_err(backend)?;
        transaction
            .execute(
                "DELETE FROM graph_relations
                 WHERE collection = ?1
                   AND tenant_id = ?2
                   AND json_extract(payload, '$.evidence.document_id') = ?3",
                params![collection, tenant_id, &document_id.0],
            )
            .map_err(backend)?;
        transaction.commit().map_err(backend)?;
        Ok(removed as u64)
    }

    async fn health(&self, tenant_id: &str) -> RagResult<GraphHealth> {
        let connection = self.lock()?;
        let effective = effective_tenant(Some(tenant_id));
        let entities = connection
            .query_row(
                "SELECT COUNT(*) FROM graph_entities WHERE tenant_id = ?1 OR (tenant_id IS NULL AND ?2 = 'default')",
                params![effective, effective],
                |row| row.get::<_, i64>(0),
            )
            .map_err(backend)?;
        let relations = connection
            .query_row(
                "SELECT COUNT(*) FROM graph_relations WHERE tenant_id = ?1 OR (tenant_id IS NULL AND ?2 = 'default')",
                params![effective, effective],
                |row| row.get::<_, i64>(0),
            )
            .map_err(backend)?;
        Ok(GraphHealth {
            ready: true,
            entity_count: entities as usize,
            relation_count: relations as usize,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EntityLabel, EvidenceSpan, RelationType};

    fn entity(id: &str) -> GraphEntity {
        GraphEntity {
            id: EntityId(id.to_owned()),
            label: EntityLabel::Organization,
            normalized_value: id.to_owned(),
            document_id: DocumentId("doc-1".to_owned()),
            tenant_id: Some("acme".to_owned()),
            chunk_id: None,
            start: 0,
            end: 4,
            confidence: 0.99,
            model_version: "gliner-2.5".to_owned(),
            properties: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn persists_entities_and_relations_across_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("graph.sqlite");
        let store = SqliteGraphStore::open(&path).unwrap();
        store
            .upsert_entities("contracts", "acme", &[entity("a"), entity("b")])
            .await
            .unwrap();
        store
            .upsert_relations(
                "contracts",
                "acme",
                &[GraphRelation {
                    subject: EntityId("a".to_owned()),
                    predicate: RelationType::References,
                    object: EntityId("b".to_owned()),
                    evidence: EvidenceSpan {
                        document_id: DocumentId("doc-1".to_owned()),
                        chunk_id: None,
                        start: 0,
                        end: 4,
                        text_hash: "hash-1".to_owned(),
                    },
                    confidence: 0.95,
                    ontology_version: "legal-v1".to_owned(),
                    tenant_id: Some("acme".to_owned()),
                }],
            )
            .await
            .unwrap();
        drop(store);

        let reopened = SqliteGraphStore::open(&path).unwrap();
        let context = reopened
            .neighborhood("contracts", "acme", &[EntityId("a".to_owned())], 1, 10, 0.9)
            .await
            .unwrap();
        assert_eq!(context.entities.len(), 2);
        assert_eq!(context.relations.len(), 1);
    }

    /// An endpoint upserted to a different tenant leaves the persisted edge
    /// stale. The read path must skip that edge, not fail the whole collection.
    #[tokio::test]
    async fn neighborhood_skips_stale_edge_after_endpoint_tenant_upsert() {
        let store = SqliteGraphStore::open(":memory:").unwrap();
        store
            .upsert_entities("contracts", "acme", &[entity("subject"), entity("object")])
            .await
            .unwrap();
        store
            .upsert_relations(
                "contracts",
                "acme",
                &[GraphRelation {
                    subject: EntityId("subject".to_owned()),
                    predicate: RelationType::References,
                    object: EntityId("object".to_owned()),
                    evidence: EvidenceSpan {
                        document_id: DocumentId("doc-1".to_owned()),
                        chunk_id: None,
                        start: 0,
                        end: 4,
                        text_hash: "hash-1".to_owned(),
                    },
                    confidence: 0.95,
                    ontology_version: "legal-v1".to_owned(),
                    tenant_id: Some("acme".to_owned()),
                }],
            )
            .await
            .unwrap();

        let mut moved = entity("object");
        moved.tenant_id = Some("globex".to_owned());
        store.upsert_entities("contracts", "globex", &[moved]).await.unwrap();

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
            .expect("a stale edge must not make the collection unqueryable");
        assert_eq!(context.entities.len(), 1);
        assert_eq!(context.entities[0].id.0, "subject");
        assert!(context.relations.is_empty());
    }

    /// The cap is enforced before `load_memory`, so an oversized seed set
    /// cannot force a full-collection copy.
    #[tokio::test]
    async fn neighborhood_rejects_oversized_seed_set_before_loading() {
        let store = SqliteGraphStore::open(":memory:").unwrap();
        store
            .upsert_entities("contracts", "acme", &[entity("a"), entity("b")])
            .await
            .unwrap();
        let ids = (0..=MAX_ENTITY_IDS)
            .map(|i| EntityId(format!("e-{i}")))
            .collect::<Vec<_>>();
        let err = store
            .neighborhood("contracts", "acme", &ids, 1, 10, 0.9)
            .await
            .expect_err("101 ids must be refused before any collection load");
        assert!(matches!(err, RagError::InvalidQuery(_)));
    }
}
