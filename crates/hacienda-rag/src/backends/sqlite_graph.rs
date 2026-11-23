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
    entity_key, validate_neighborhood_bounds, EntityId, EntityKey, GraphContext, GraphEntity,
    GraphHealth, GraphRelation, GraphStore, InMemoryGraphStore,
};
use crate::types::DocumentId;
use async_trait::async_trait;
use rusqlite::{params, params_from_iter, Connection};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

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
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    fn lock(&self) -> RagResult<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| RagError::InvalidQuery("sqlite graph lock poisoned".to_owned()))
    }

    /// Numbered placeholders for a variable-length `IN (...)` clause starting
    /// after the fixed parameters already added to `binds`.
    fn placeholders(binds: &[String], count: usize) -> String {
        (1..=count)
            .map(|i| format!("?{}", binds.len() + i))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Entity payloads owned by `tenant`, optionally restricted to `ids`.
    fn entity_payloads(
        &self,
        collection: &str,
        tenant: &str,
        ids: Option<&[EntityId]>,
    ) -> RagResult<Vec<String>> {
        let connection = self.lock()?;
        let mut sql = String::from(
            "SELECT payload FROM graph_entities
             WHERE collection = ?1
               AND (tenant_key = ?2 OR (tenant_id IS NULL AND ?2 = 'default'))",
        );
        let mut binds = vec![collection.to_owned(), tenant.to_owned()];
        if let Some(ids) = ids {
            sql.push_str(&format!(
                " AND entity_id IN ({})",
                Self::placeholders(&binds, ids.len())
            ));
            binds.extend(ids.iter().map(|id| id.0.clone()));
        }
        let mut statement = connection.prepare(&sql).map_err(backend)?;
        let payloads = statement
            .query_map(params_from_iter(binds.iter()), |row| {
                row.get::<_, String>(0)
            })
            .map_err(backend)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(backend);
        payloads
    }

    /// Relation payloads owned by `tenant` that touch any of `ids`.
    fn relation_payloads(
        &self,
        collection: &str,
        tenant: &str,
        ids: &[EntityId],
    ) -> RagResult<Vec<String>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.lock()?;
        let first = 3;
        let subject_in = (first..first + ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let object_in = (first + ids.len()..first + 2 * ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT payload FROM graph_relations
             WHERE collection = ?1 AND tenant_key = ?2
               AND (subject_id IN ({subject_in}) OR object_id IN ({object_in}))"
        );
        let mut binds: Vec<String> = vec![collection.to_owned(), tenant.to_owned()];
        binds.extend(ids.iter().map(|id| id.0.clone()));
        binds.extend(ids.iter().map(|id| id.0.clone()));
        let mut statement = connection.prepare(&sql).map_err(backend)?;
        let payloads = statement
            .query_map(params_from_iter(binds.iter()), |row| {
                row.get::<_, String>(0)
            })
            .map_err(backend)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(backend);
        payloads
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
    ) -> RagResult<InMemoryGraphStore> {
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
            for payload in self.entity_payloads(collection, tenant, Some(&ids))? {
                let entity: GraphEntity = serde_json::from_str(&payload)
                    .map_err(|error| RagError::Backend(Box::new(error)))?;
                entities.insert(entity_key(&entity), entity);
            }
            if depth == hops {
                break;
            }
            let mut next = Vec::new();
            for payload in self.relation_payloads(collection, tenant, &ids)? {
                if !relation_payloads.insert(payload.clone()) {
                    continue;
                }
                let relation: GraphRelation = serde_json::from_str(&payload)
                    .map_err(|error| RagError::Backend(Box::new(error)))?;
                next.push(relation.subject);
                next.push(relation.object);
            }
            frontier = next;
        }

        let memory = InMemoryGraphStore::default();
        for entity in entities.values() {
            // Payloads written by this adapter are validated before commit;
            // skip a legacy malformed row rather than failing another tenant's
            // query. Finding such a row is still visible in its own tenant only.
            if InMemoryGraphStore::validate_entity(entity).is_ok() {
                memory.insert_entities(collection, std::slice::from_ref(entity))?;
            }
        }
        for payload in &relation_payloads {
            let relation: GraphRelation = serde_json::from_str(payload)
                .map_err(|error| RagError::Backend(Box::new(error)))?;
            if memory
                .insert_relations(collection, std::slice::from_ref(&relation))
                .is_err()
            {
                continue;
            }
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
        // Validate only the endpoints of this write, not the whole collection.
        let ids = relations
            .iter()
            .flat_map(|relation| [&relation.subject, &relation.object])
            .cloned()
            .collect::<Vec<_>>();
        let memory = InMemoryGraphStore::default();
        for payload in self.entity_payloads(collection, tenant, Some(&ids))? {
            let entity: GraphEntity = serde_json::from_str(&payload)
                .map_err(|error| RagError::Backend(Box::new(error)))?;
            memory.insert_entities(collection, std::slice::from_ref(&entity))?;
        }
        memory.insert_relations(collection, relations)?;

        let connection = self.lock()?;
        let transaction = connection.unchecked_transaction().map_err(backend)?;
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
        let ids = transaction
            .prepare(
                "SELECT entity_id FROM graph_entities
                 WHERE collection = ?1 AND tenant_key = ?2 AND document_id = ?3",
            )
            .map_err(backend)?
            .query_map(params![collection, tenant, &document_id.0], |row| {
                row.get::<_, String>(0)
            })
            .map_err(backend)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(backend)?;

        let removed = transaction
            .execute(
                "DELETE FROM graph_entities
                 WHERE collection = ?1 AND tenant_key = ?2 AND document_id = ?3",
                params![collection, tenant, &document_id.0],
            )
            .map_err(backend)?;

        // Remove this tenant's edges for the document, including edges whose
        // endpoints are being removed. Foreign tenants' edges are untouched.
        let mut sql = String::from(
            "DELETE FROM graph_relations
             WHERE collection = ?1 AND tenant_key = ?2
               AND (json_extract(payload, '$.evidence.document_id') = ?3",
        );
        let mut binds: Vec<String> = vec![
            collection.to_owned(),
            tenant.to_owned(),
            document_id.0.clone(),
        ];
        if !ids.is_empty() {
            let first = 4;
            let subject_in = (first..first + ids.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let object_in = (first + ids.len()..first + 2 * ids.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            sql.push_str(&format!(
                " OR subject_id IN ({subject_in}) OR object_id IN ({object_in})"
            ));
            binds.extend(ids.iter().cloned());
            binds.extend(ids.iter().cloned());
        }
        sql.push(')');
        transaction
            .execute(&sql, params_from_iter(binds.iter()))
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
            store.load_bounded_memory(&collection_name, &tenant, &ids, hops)
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
    /// own rows, including after a reopen.
    #[tokio::test]
    async fn tenant_qualified_identity_survives_reopen() {
        let store = SqliteGraphStore::open(":memory:").unwrap();
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

        let acme = store.health("acme").await.unwrap();
        let globex = store.health("globex").await.unwrap();
        assert_eq!((acme.entity_count, globex.entity_count), (1, 1));
        let context = store
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
        assert_eq!(context.entities[0].normalized_value, "globex-copy");
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
        assert_eq!(
            store
                .neighborhood(
                    "contracts",
                    "acme",
                    &[EntityId("good".to_owned())],
                    1,
                    10,
                    0.9
                )
                .await
                .unwrap()
                .entities
                .len(),
            0
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
}
