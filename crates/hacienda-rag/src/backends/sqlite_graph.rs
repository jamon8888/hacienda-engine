//! Durable SQLite [`GraphStore`](crate::GraphStore) for embedded deployments.
//!
//! The graph contract stays independent from SQLite. This adapter stores the
//! typed graph records as JSON plus indexed identity columns, which keeps the
//! schema migration surface small while preserving the exact provenance and
//! ontology payloads. It is intentionally feature-gated for Tauri/local use.
//!
//! Every operation runs in full on a blocking thread: the mutex is acquired,
//! the transaction is opened and committed there, and never on an async
//! runtime worker. Reads are scoped to one tenant and to the requested
//! neighborhood, so a one-seed query never reconstructs unrelated documents.

use crate::error::{RagError, RagResult};
use crate::graph::{
    entity_key, validate_graph_bounds, validate_neighborhood_bounds, EntityId, EntityKey,
    GraphContext, GraphEntity, GraphHealth, GraphRelation, GraphStore, InMemoryGraphStore,
};
use crate::types::DocumentId;
use async_trait::async_trait;
use rusqlite::{params, params_from_iter, Connection};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Keep every `IN (...)` well under SQLite's default 32,766 bind limit. Both
/// endpoint columns are bound, so a chunk never contributes more than 2× this.
const MAX_ID_BINDS: usize = 400;
/// Hard ceiling on relations decoded for one bounded neighborhood expansion.
const MAX_EXPANSION_RELATIONS: usize = 20_000;

fn backend(error: rusqlite::Error) -> RagError {
    RagError::Backend(Box::new(error))
}

/// Run the *whole* synchronous database operation off the async runtime.
async fn on_blocking_thread<T, F>(job: F) -> RagResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> RagResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|error| RagError::Backend(Box::new(error)))?
}

fn unique_ids(ids: &[EntityId]) -> Vec<EntityId> {
    let mut seen = HashSet::new();
    ids.iter()
        .filter(|id| seen.insert(id.0.clone()))
        .cloned()
        .collect()
}

/// Numbered placeholders for a variable-length `IN (...)` clause starting
/// after `fixed` parameters already bound in the statement.
fn placeholders(count: usize, fixed: usize) -> String {
    (1..=count)
        .map(|i| format!("?{}", fixed + i))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Entity payloads owned by `tenant`, optionally restricted to `ids`. The ids
/// are deduplicated and read in bounded chunks so no statement can exceed the
/// SQLite bind limit. Works on a connection or an open transaction.
fn entity_payloads_on(
    connection: &Connection,
    collection: &str,
    tenant: &str,
    ids: Option<&[EntityId]>,
) -> RagResult<Vec<String>> {
    let mut payloads = Vec::new();
    let base = "SELECT payload FROM graph_entities
                WHERE collection = ?1
                  AND (tenant_key = ?2 OR (tenant_id IS NULL AND ?2 = 'default'))";
    match ids {
        None => {
            let mut statement = connection.prepare(base).map_err(backend)?;
            let rows = statement
                .query_map(params![collection, tenant], |row| row.get::<_, String>(0))
                .map_err(backend)?;
            for row in rows {
                payloads.push(row.map_err(backend)?);
            }
        }
        Some(ids) => {
            let ids = unique_ids(ids);
            for chunk in ids.chunks(MAX_ID_BINDS) {
                let sql = format!("{base} AND entity_id IN ({})", placeholders(chunk.len(), 2));
                let mut binds: Vec<String> = vec![collection.to_owned(), tenant.to_owned()];
                binds.extend(chunk.iter().map(|id| id.0.clone()));
                let mut statement = connection.prepare(&sql).map_err(backend)?;
                let rows = statement
                    .query_map(params_from_iter(binds.iter()), |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(backend)?;
                for row in rows {
                    payloads.push(row.map_err(backend)?);
                }
            }
        }
    }
    Ok(payloads)
}

/// Relation payloads owned by `tenant` that touch any of `ids`, filtered by
/// confidence in SQL so low-scoring candidates are never decoded. Chunked and
/// limited so a fan-out cannot exhaust SQLite's bind limit or memory.
fn relation_payloads_on(
    connection: &Connection,
    collection: &str,
    tenant: &str,
    ids: &[EntityId],
    min_confidence: f32,
    max_rows: usize,
) -> RagResult<Vec<String>> {
    let ids = unique_ids(ids);
    let mut payloads = Vec::new();
    for chunk in ids.chunks(MAX_ID_BINDS) {
        if payloads.len() >= max_rows {
            break;
        }
        let subject_at = 3;
        let object_at = subject_at + chunk.len();
        let confidence_at = object_at + chunk.len();
        let limit_at = confidence_at + 1;
        let sql = format!(
            "SELECT payload FROM graph_relations
             WHERE collection = ?1 AND tenant_key = ?2
               AND (subject_id IN ({}) OR object_id IN ({}))
               AND json_extract(payload, '$.confidence') >= ?{confidence_at}
             LIMIT ?{limit_at}",
            placeholders(chunk.len(), 2),
            placeholders(chunk.len(), subject_at + chunk.len() - 1)
        );
        // Values must keep their SQLite types: a TEXT '0.9' compared to a
        // numeric json_extract result is always false under SQLite's type
        // ordering, which would silently drop every relation.
        let mut binds: Vec<rusqlite::types::Value> = vec![
            rusqlite::types::Value::Text(collection.to_owned()),
            rusqlite::types::Value::Text(tenant.to_owned()),
        ];
        binds.extend(
            chunk
                .iter()
                .map(|id| rusqlite::types::Value::Text(id.0.clone())),
        );
        binds.extend(
            chunk
                .iter()
                .map(|id| rusqlite::types::Value::Text(id.0.clone())),
        );
        binds.push(rusqlite::types::Value::Real(min_confidence as f64));
        let remaining = max_rows - payloads.len();
        binds.push(rusqlite::types::Value::Integer(remaining as i64));
        let mut statement = connection.prepare(&sql).map_err(backend)?;
        let rows = statement
            .query_map(params_from_iter(binds.iter()), |row| {
                row.get::<_, String>(0)
            })
            .map_err(backend)?;
        for row in rows {
            if payloads.len() >= max_rows {
                break;
            }
            payloads.push(row.map_err(backend)?);
        }
    }
    Ok(payloads)
}

/// SQLite-backed graph store. One connection is serialized behind a mutex;
/// SQLite remains the source of truth and callers may share this type across
/// async tasks safely.
#[derive(Clone)]
pub struct SqliteGraphStore {
    connection: Arc<Mutex<Connection>>,
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
                     tenant_key TEXT NOT NULL,
                     entity_id TEXT NOT NULL,
                     document_id TEXT NOT NULL,
                     tenant_id TEXT,
                     payload TEXT NOT NULL,
                     PRIMARY KEY (collection, tenant_key, entity_id)
                 );
                 CREATE INDEX IF NOT EXISTS graph_entities_scope
                     ON graph_entities (collection, tenant_key, document_id);
                 CREATE TABLE IF NOT EXISTS graph_relations (
                     collection TEXT NOT NULL,
                     tenant_key TEXT NOT NULL,
                     subject_id TEXT NOT NULL,
                     predicate TEXT NOT NULL,
                     object_id TEXT NOT NULL,
                     tenant_id TEXT NOT NULL,
                     evidence_hash TEXT NOT NULL,
                     payload TEXT NOT NULL,
                     PRIMARY KEY (collection, tenant_key, subject_id, predicate, object_id, evidence_hash)
                 );
                 CREATE INDEX IF NOT EXISTS graph_relations_scope
                     ON graph_relations (collection, tenant_key, subject_id, object_id);",
            )
            .map_err(backend)?;
        // A pre-release prototype of this table omitted `tenant_key`, which let
        // two tenants overwrite each other's rows. Refuse to use such a file
        // instead of silently reintroducing the collision.
        for table in ["graph_entities", "graph_relations"] {
            let mut statement = connection
                .prepare(&format!("PRAGMA table_info({table})"))
                .map_err(backend)?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(backend)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(backend)?;
            if !columns.iter().any(|name| name == "tenant_key") {
                return Err(RagError::Backend(Box::new(std::io::Error::other(format!(
                    "{table} lacks the tenant_key column; recreate the graph database"
                )))));
            }
        }
        // Host-linked SQLite is not guaranteed to include JSON1. The evidence
        // predicates in relation reads/deletes require it, so fail clearly at
        // open rather than at the first document deletion.
        let json1 = connection
            .query_row("SELECT json_extract(?1, '$.ok')", ["{\"ok\":1}"], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|error| {
                RagError::Backend(Box::new(std::io::Error::other(format!(
                    "sqlite-graph requires a host SQLite library with JSON1 enabled: {error}"
                ))))
            })?;
        if json1 != 1 {
            return Err(RagError::Backend(Box::new(std::io::Error::other(
                "sqlite-graph requires SQLite JSON1 support",
            ))));
        }
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    fn lock(&self) -> RagResult<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| RagError::InvalidQuery("sqlite graph lock poisoned".to_owned()))
    }

    /// Load only the tenant's seed entities and their bounded neighborhood.
    /// Endpoint lookups are tenant-qualified, so an identically named entity in
    /// another collection tenant is never deserialized or traversed.
    fn load_bounded_memory(
        &self,
        collection: &str,
        tenant: &str,
        seeds: &[EntityId],
        hops: u8,
        limit: usize,
        min_confidence: f32,
    ) -> RagResult<InMemoryGraphStore> {
        let connection = self.lock()?;
        let mut entities: HashMap<EntityKey, GraphEntity> = HashMap::new();
        let mut relation_payloads: HashSet<String> = HashSet::new();
        let mut visited: HashSet<EntityId> = HashSet::new();
        let mut frontier = seeds.to_vec();

        for depth in 0..=hops {
            let ids: Vec<EntityId> = frontier
                .drain(..)
                .filter(|id| visited.insert(id.clone()))
                .collect();
            if ids.is_empty() {
                break;
            }
            for payload in entity_payloads_on(&connection, collection, tenant, Some(&ids))? {
                let entity: GraphEntity = serde_json::from_str(&payload)
                    .map_err(|error| RagError::Backend(Box::new(error)))?;
                InMemoryGraphStore::validate_entity(&entity).map_err(|error| {
                    RagError::Backend(Box::new(std::io::Error::other(format!(
                        "invalid stored graph entity {}: {error}",
                        entity.id.0
                    ))))
                })?;
                entities.insert(entity_key(&entity), entity);
            }
            if depth == hops || relation_payloads.len() >= MAX_EXPANSION_RELATIONS {
                break;
            }
            let remaining = MAX_EXPANSION_RELATIONS - relation_payloads.len();
            let mut next = Vec::new();
            for payload in relation_payloads_on(
                &connection,
                collection,
                tenant,
                &ids,
                min_confidence,
                remaining,
            )? {
                if !relation_payloads.insert(payload.clone()) {
                    continue;
                }
                let relation: GraphRelation = serde_json::from_str(&payload)
                    .map_err(|error| RagError::Backend(Box::new(error)))?;
                InMemoryGraphStore::validate_relation_payload(&relation).map_err(|error| {
                    RagError::Backend(Box::new(std::io::Error::other(format!(
                        "invalid stored graph relation {}->{}: {error}",
                        relation.subject.0, relation.object.0
                    ))))
                })?;
                next.push(relation.subject);
                next.push(relation.object);
            }
            frontier = next;
        }

        // The loader has already validated every stored row. Re-inserting into
        // the in-memory store also re-checks endpoint ownership; a stale edge
        // whose endpoint was re-assigned is skipped, any other failure is a
        // real integrity error and must be reported.
        let memory = InMemoryGraphStore::default();
        for entity in entities.values() {
            memory.insert_entities(collection, std::slice::from_ref(entity))?;
        }
        for payload in &relation_payloads {
            let relation: GraphRelation = serde_json::from_str(payload)
                .map_err(|error| RagError::Backend(Box::new(error)))?;
            let tenant_key = crate::graph::effective_tenant(relation.tenant_id.as_deref());
            let stale = [&relation.subject, &relation.object].iter().any(|id| {
                entities
                    .get(&(tenant_key.to_owned(), (*id).clone()))
                    .is_none()
            });
            if stale {
                continue;
            }
            memory.insert_relations(collection, std::slice::from_ref(&relation))?;
        }
        Ok(memory)
    }

    fn insert_entities_sync(
        &self,
        collection: &str,
        tenant: &str,
        entities: &[GraphEntity],
    ) -> RagResult<()> {
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction().map_err(backend)?;
        for entity in entities {
            let payload = serde_json::to_string(entity)
                .map_err(|error| RagError::Backend(Box::new(error)))?;
            transaction
                .execute(
                    "INSERT INTO graph_entities
                     (collection, tenant_key, entity_id, document_id, tenant_id, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(collection, tenant_key, entity_id) DO UPDATE SET
                       document_id = excluded.document_id,
                       tenant_id = excluded.tenant_id,
                       payload = excluded.payload",
                    params![
                        collection,
                        tenant,
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

    fn insert_relations_sync(
        &self,
        collection: &str,
        tenant: &str,
        relations: &[GraphRelation],
    ) -> RagResult<()> {
        let connection = self.lock()?;
        // Validation and insertion share one transaction: a concurrent delete
        // cannot remove validated endpoints between the two steps.
        let transaction = connection.unchecked_transaction().map_err(backend)?;
        let ids = relations
            .iter()
            .flat_map(|relation| [&relation.subject, &relation.object])
            .cloned()
            .collect::<Vec<_>>();
        let memory = InMemoryGraphStore::default();
        for payload in entity_payloads_on(&transaction, collection, tenant, Some(&ids))? {
            let entity: GraphEntity = serde_json::from_str(&payload)
                .map_err(|error| RagError::Backend(Box::new(error)))?;
            memory.insert_entities(collection, std::slice::from_ref(&entity))?;
        }
        memory.insert_relations(collection, relations)?;

        for relation in relations {
            let payload = serde_json::to_string(relation)
                .map_err(|error| RagError::Backend(Box::new(error)))?;
            transaction
                .execute(
                    "INSERT INTO graph_relations
                     (collection, tenant_key, subject_id, predicate, object_id, tenant_id, evidence_hash, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(collection, tenant_key, subject_id, predicate, object_id, evidence_hash)
                     DO UPDATE SET payload = excluded.payload",
                    params![
                        collection,
                        tenant,
                        &relation.subject.0,
                        serde_json::to_string(&relation.predicate)
                            .map_err(|error| RagError::Backend(Box::new(error)))?,
                        &relation.object.0,
                        tenant,
                        &relation.evidence.text_hash,
                        payload
                    ],
                )
                .map_err(backend)?;
        }
        transaction.commit().map_err(backend)
    }

    fn delete_document_sync(
        &self,
        collection: &str,
        tenant: &str,
        document_id: &DocumentId,
    ) -> RagResult<u64> {
        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction().map_err(backend)?;
        // Delete edges first, using a tenant-scoped subquery for the entity ids
        // instead of binding potentially thousands of ids into the statement.
        transaction
            .execute(
                "DELETE FROM graph_relations
                 WHERE collection = ?1 AND tenant_key = ?2
                   AND (
                     json_extract(payload, '$.evidence.document_id') = ?3
                     OR subject_id IN (
                       SELECT entity_id FROM graph_entities
                       WHERE collection = ?1 AND tenant_key = ?2 AND document_id = ?3)
                     OR object_id IN (
                       SELECT entity_id FROM graph_entities
                       WHERE collection = ?1 AND tenant_key = ?2 AND document_id = ?3)
                   )",
                params![collection, tenant, &document_id.0],
            )
            .map_err(backend)?;
        let removed = transaction
            .execute(
                "DELETE FROM graph_entities
                 WHERE collection = ?1 AND tenant_key = ?2 AND document_id = ?3",
                params![collection, tenant, &document_id.0],
            )
            .map_err(backend)?;
        transaction.commit().map_err(backend)?;
        Ok(removed as u64)
    }

    fn health_sync(&self, tenant: &str) -> RagResult<GraphHealth> {
        let connection = self.lock()?;
        let entities = connection
            .query_row(
                "SELECT COUNT(*) FROM graph_entities WHERE tenant_key = ?1",
                params![tenant],
                |row| row.get::<_, i64>(0),
            )
            .map_err(backend)?;
        let relations = connection
            .query_row(
                "SELECT COUNT(*) FROM graph_relations WHERE tenant_key = ?1",
                params![tenant],
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

#[async_trait]
impl GraphStore for SqliteGraphStore {
    async fn upsert_entities(
        &self,
        collection: &str,
        tenant_id: &str,
        entities: &[GraphEntity],
    ) -> RagResult<()> {
        for entity in entities {
            if crate::graph::effective_tenant(entity.tenant_id.as_deref()) != tenant_id {
                return Err(RagError::InvalidQuery(format!(
                    "entity {} belongs to tenant {}, expected {}",
                    entity.id.0,
                    crate::graph::effective_tenant(entity.tenant_id.as_deref()),
                    tenant_id
                )));
            }
            InMemoryGraphStore::validate_entity(entity)?;
        }
        let store = self.clone();
        let collection = collection.to_owned();
        let tenant = tenant_id.to_owned();
        let entities = entities.to_vec();
        on_blocking_thread(move || store.insert_entities_sync(&collection, &tenant, &entities))
            .await
    }

    async fn upsert_relations(
        &self,
        collection: &str,
        tenant_id: &str,
        relations: &[GraphRelation],
    ) -> RagResult<()> {
        for relation in relations {
            if crate::graph::effective_tenant(relation.tenant_id.as_deref()) != tenant_id {
                return Err(RagError::InvalidQuery(format!(
                    "relation {}->{} belongs to tenant {}, expected {}",
                    relation.subject.0,
                    relation.object.0,
                    crate::graph::effective_tenant(relation.tenant_id.as_deref()),
                    tenant_id
                )));
            }
            InMemoryGraphStore::validate_relation_payload(relation)?;
        }
        let store = self.clone();
        let collection = collection.to_owned();
        let tenant = tenant_id.to_owned();
        let relations = relations.to_vec();
        on_blocking_thread(move || store.insert_relations_sync(&collection, &tenant, &relations))
            .await
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
        // Every bound is checked before any SQL is issued.
        validate_neighborhood_bounds(entity_ids, hops, limit, min_confidence)?;
        let store = self.clone();
        let collection_name = collection.to_owned();
        let tenant = tenant_id.to_owned();
        let ids = entity_ids.to_vec();
        let memory = on_blocking_thread(move || {
            store.load_bounded_memory(&collection_name, &tenant, &ids, hops, limit, min_confidence)
        })
        .await?;
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
        let store = self.clone();
        let collection = collection.to_owned();
        let tenant = tenant_id.to_owned();
        let document_id = document_id.clone();
        on_blocking_thread(move || store.delete_document_sync(&collection, &tenant, &document_id))
            .await
    }

    async fn health(&self, tenant_id: &str) -> RagResult<GraphHealth> {
        let store = self.clone();
        let tenant = tenant_id.to_owned();
        on_blocking_thread(move || store.health_sync(&tenant)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EntityLabel, EvidenceSpan, RelationType};
    use crate::types::DEFAULT_TENANT;

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

    fn relation(subject: &str, object: &str, hash: &str) -> GraphRelation {
        GraphRelation {
            subject: EntityId(subject.to_owned()),
            predicate: RelationType::References,
            object: EntityId(object.to_owned()),
            evidence: EvidenceSpan {
                document_id: DocumentId("doc-1".to_owned()),
                chunk_id: None,
                start: 0,
                end: 4,
                text_hash: hash.to_owned(),
            },
            confidence: 0.95,
            ontology_version: "legal-v1".to_owned(),
            tenant_id: Some("acme".to_owned()),
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
            .upsert_relations("contracts", "acme", &[relation("a", "b", "hash-1")])
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

    /// Two tenants using the same EntityId in one collection must keep their
    /// own rows, including after reopening a file-backed database.
    #[tokio::test]
    async fn tenant_qualified_identity_survives_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("graph.sqlite");
        let store = SqliteGraphStore::open(&path).unwrap();
        store
            .upsert_entities("contracts", "acme", &[entity("shared")])
            .await
            .unwrap();
        let mut other = entity("shared");
        other.tenant_id = Some("globex".to_owned());
        other.normalized_value = "globex-copy".to_owned();
        store
            .upsert_entities("contracts", "globex", &[other])
            .await
            .unwrap();
        drop(store);

        let reopened = SqliteGraphStore::open(&path).unwrap();
        let acme = reopened
            .neighborhood(
                "contracts",
                "acme",
                &[EntityId("shared".to_owned())],
                1,
                10,
                0.9,
            )
            .await
            .unwrap();
        let globex = reopened
            .neighborhood(
                "contracts",
                "globex",
                &[EntityId("shared".to_owned())],
                1,
                10,
                0.9,
            )
            .await
            .unwrap();
        assert_eq!(acme.entities[0].normalized_value, "shared");
        assert_eq!(globex.entities[0].normalized_value, "globex-copy");
    }

    /// Invalid entities are rejected at write time, so a malformed row can
    /// never poison later tenant-scoped reads.
    #[tokio::test]
    async fn rejects_invalid_entity_before_commit() {
        let store = SqliteGraphStore::open(":memory:").unwrap();
        let mut invalid = entity("bad");
        invalid.end = invalid.start;
        let result = store
            .upsert_entities("contracts", "acme", &[entity("good"), invalid])
            .await;
        assert!(matches!(result, Err(RagError::InvalidQuery(_))));
        assert_eq!(store.health("acme").await.unwrap().entity_count, 0);
    }

    /// A malformed row written outside this adapter must produce a contextual
    /// error instead of masquerading as an empty neighborhood.
    #[tokio::test]
    async fn reports_invalid_stored_entity_payload() {
        let store = SqliteGraphStore::open(":memory:").unwrap();
        store
            .upsert_entities("contracts", "acme", &[entity("a")])
            .await
            .unwrap();
        let mut malformed = entity("broken");
        malformed.end = malformed.start;
        let payload = serde_json::to_string(&malformed).unwrap();
        let connection = store.lock().unwrap();
        connection
            .execute(
                "INSERT INTO graph_entities
                 (collection, tenant_key, entity_id, document_id, tenant_id, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "contracts",
                    "acme",
                    "broken",
                    "doc-1",
                    Some("acme"),
                    payload
                ],
            )
            .unwrap();
        drop(connection);

        let error = store
            .neighborhood(
                "contracts",
                "acme",
                &[EntityId("broken".to_owned())],
                1,
                10,
                0.9,
            )
            .await
            .expect_err("invalid stored entity must be reported");
        assert!(
            matches!(error, RagError::Backend(_)),
            "unexpected error: {error}"
        );
    }

    /// A one-seed read must not load another tenant's rows even when both use
    /// the same collection name and entity ids.
    #[tokio::test]
    async fn neighborhood_reads_are_tenant_and_seed_scoped() {
        let store = SqliteGraphStore::open(":memory:").unwrap();
        let mut acme_a = entity("a");
        acme_a.document_id = DocumentId("doc-a".to_owned());
        let mut acme_b = entity("b");
        acme_b.document_id = DocumentId("doc-a".to_owned());
        store
            .upsert_entities("contracts", "acme", &[acme_a, acme_b])
            .await
            .unwrap();
        let mut globex_c = entity("c");
        globex_c.tenant_id = Some("globex".to_owned());
        globex_c.document_id = DocumentId("doc-g".to_owned());
        store
            .upsert_entities("contracts", "globex", &[globex_c])
            .await
            .unwrap();
        // A relation reachable only from the other tenant's seed.
        let mut cross = relation("c", "a", "cross-hash");
        cross.tenant_id = Some("globex".to_owned());
        cross.evidence.document_id = DocumentId("doc-g".to_owned());
        let mut globex_a = entity("a");
        globex_a.tenant_id = Some("globex".to_owned());
        globex_a.document_id = DocumentId("doc-g".to_owned());
        store
            .upsert_entities("contracts", "globex", &[globex_a])
            .await
            .unwrap();
        store
            .upsert_relations("contracts", "globex", &[cross])
            .await
            .unwrap();

        let acme = store
            .neighborhood("contracts", "acme", &[EntityId("a".to_owned())], 2, 50, 0.9)
            .await
            .unwrap();
        assert_eq!(
            acme.entities.len(),
            1,
            "only the seeded entity loads; b has no edge from a"
        );
        assert!(acme
            .entities
            .iter()
            .all(|e| e.tenant_id.as_deref() == Some("acme")));
    }

    #[tokio::test]
    async fn delete_document_only_removes_callers_edges() {
        let store = SqliteGraphStore::open(":memory:").unwrap();
        store
            .upsert_entities("contracts", "acme", &[entity("a"), entity("b")])
            .await
            .unwrap();
        store
            .upsert_relations("contracts", "acme", &[relation("a", "b", "acme-hash")])
            .await
            .unwrap();
        let mut foreign = relation("a", "b", "foreign-hash");
        foreign.tenant_id = Some("globex".to_owned());
        let mut foreign_a = entity("a");
        foreign_a.tenant_id = Some("globex".to_owned());
        let mut foreign_b = entity("b");
        foreign_b.tenant_id = Some("globex".to_owned());
        store
            .upsert_entities("contracts", "globex", &[foreign_a, foreign_b])
            .await
            .unwrap();
        store
            .upsert_relations("contracts", "globex", &[foreign])
            .await
            .unwrap();

        assert_eq!(
            store
                .delete_document("contracts", "acme", &DocumentId("doc-1".to_owned()))
                .await
                .unwrap(),
            2
        );
        assert_eq!(store.health("acme").await.unwrap().relation_count, 0);
        assert_eq!(store.health("globex").await.unwrap().relation_count, 1);
    }

    /// Every bound is validated before `load_bounded_memory` touches SQLite.
    #[tokio::test]
    async fn neighborhood_rejects_invalid_bounds_before_loading() {
        let store = SqliteGraphStore::open(":memory:").unwrap();
        store
            .upsert_entities("contracts", "acme", &[entity("a"), entity("b")])
            .await
            .unwrap();
        let ids = (0..=crate::graph::MAX_ENTITY_IDS)
            .map(|i| EntityId(format!("e-{i}")))
            .collect::<Vec<_>>();
        for (ids, hops, limit, confidence) in [
            (ids, 1, 10, 0.9),
            (vec![EntityId("a".to_owned())], 3, 10, 0.9),
            (vec![EntityId("a".to_owned())], 1, 0, 0.9),
            (vec![EntityId("a".to_owned())], 1, 10, f32::NAN),
        ] {
            let err = store
                .neighborhood("contracts", "acme", &ids, hops, limit, confidence)
                .await
                .expect_err("invalid bounds must be refused before any SQL");
            assert!(matches!(err, RagError::InvalidQuery(_)));
        }
        // The default tenant has no rows even though acme does.
        let health = store.health(DEFAULT_TENANT).await.unwrap();
        assert_eq!(health.entity_count, 0);
    }

    #[test]
    fn graph_bounds_are_shared_and_field_specific() {
        let error = validate_graph_bounds(1, 3, 10, 0.9).unwrap_err();
        assert_eq!(error.field, "hops");
        assert!(error.to_string().contains("between 1 and 2"));
        assert!(validate_graph_bounds(1, 1, 10, 0.9).is_ok());
    }
}
