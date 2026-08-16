//! An [`AuditStore`] backed by IndexedDB — Track L5.
//!
//! Same in-memory bookkeeping as [`InMemoryAuditStore`](super::InMemoryAuditStore) (this
//! module reuses [`super::store::State`] rather than re-deriving its transition logic),
//! plus a durability step: after every mutating call, the current state is snapshotted
//! and written to a single IndexedDB record. [`IndexedDbAuditStore::open`] reads that
//! record back and replays it through [`Segment::append`] (the same path
//! `FileAuditStore`'s recovery uses), so a store opened against a database an earlier
//! instance wrote to picks up exactly where that instance left off — the persistence
//! this module exists to add, "reload the page" in Track L5's check.
//!
//! # Why this needs `SendWrapper`
//!
//! [`AuditStore`] uses plain `#[async_trait]`, so every method returns a `Send`-bounded
//! future — deliberately, so a database backend can run its I/O off whatever executor
//! thread drives it. `indexed_db_futures`'s `Database`/`Transaction`/`ObjectStore` wrap
//! `web_sys`/`js_sys` handles, which are `!Send` unconditionally on wasm32-unknown-unknown
//! (`wasm-bindgen` does not special-case the non-`atomics`, single-threaded case this
//! crate actually targets). [`send_wrapper::SendWrapper`] is the standard way to satisfy a
//! `Send` bound around a JS handle when the target is genuinely single-threaded: it just
//! moves the "never actually crosses a thread" invariant from the type system to a
//! same-thread runtime assertion that can never fire here.
//!
//! # The C3 sub-question (still open)
//!
//! An IndexedDB chain dies with a cleared browser profile. If Studio's output must be
//! legally defensible, the chain has to be *exported into the vault* (Track I2), not
//! merely retained here — this module makes the chain durable across a reload, not
//! durable in the sense a compliance record needs. That product decision is unmade;
//! nothing in this module resolves it.

use async_trait::async_trait;
use indexed_db_futures::database::Database;
use indexed_db_futures::prelude::*;
use indexed_db_futures::transaction::TransactionMode;
use send_wrapper::SendWrapper;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

use super::cursor::{page_from, AuditCursor, AuditPage};
use super::entry::{AuditEntry, AuditEntryInput};
use super::error::AuditError;
use super::segment::{verify_seal_chain, NodeId, Segment, SegmentSeal};
use super::store::{verify_open_entries, verify_sealed_entries, State};
use crate::tenancy::TenantId;

const OBJECT_STORE: &str = "hacienda_audit_snapshot";
const SCHEMA_VERSION: u32 = 1;

/// The pre-S1b record key: one global snapshot, no tenant concept at all. Every chain
/// ever written under this key was, in effect, the default tenant's — see
/// [`IndexedDbAuditStore::ensure_tenant_loaded`]'s fallback for why this constant still
/// exists after S1b introduced per-tenant keys.
const LEGACY_CURRENT_KEY: &str = "current";

/// The IndexedDB record key holding `tenant`'s snapshot. One key per tenant in the same
/// object store (see the module doc's C3 note — S1b requires this backend to isolate
/// tenants exactly like the other two, even though a browser profile is normally
/// single-tenant in practice).
fn snapshot_key(tenant: &TenantId) -> String {
    format!("current:{}", tenant.as_str())
}

fn backend_err(err: impl std::fmt::Display) -> AuditError {
    AuditError::Backend(err.to_string())
}

/// One IndexedDB record: everything needed to rebuild [`State`] on the next `open`.
#[derive(Debug, Serialize, Deserialize)]
struct Snapshot {
    open: Option<OpenSnapshot>,
    sealed: Vec<(SegmentSeal, Vec<AuditEntry>)>,
    closed_seal: Option<SegmentSeal>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenSnapshot {
    segment_id: String,
    entries: Vec<AuditEntry>,
}

/// An [`AuditStore`] persisted to IndexedDB. See the module docs for the concurrency
/// story and the C3 caveat.
///
/// One [`State`] per tenant, same as [`super::store::InMemoryAuditStore`] — see that
/// type's doc for why a tenant is a per-call parameter rather than a separate store
/// instance (decision D-S1-1, `tenancy.rs`).
pub struct IndexedDbAuditStore {
    db: SendWrapper<Database>,
    state: Mutex<HashMap<TenantId, State>>,
    node_id: NodeId,
    config_hash: String,
}

impl std::fmt::Debug for IndexedDbAuditStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexedDbAuditStore")
            .field("node_id", &self.node_id)
            .field("config_hash", &self.config_hash)
            .finish_non_exhaustive()
    }
}

impl IndexedDbAuditStore {
    /// Open (creating if necessary) the IndexedDB database `db_name` and rehydrate the
    /// store from whatever an earlier instance against the same `db_name` last wrote —
    /// or start fresh if nothing was ever written.
    ///
    /// `db_name` is the whole identity for persistence purposes: two stores opened with
    /// the same name resume the same chain, matching a file backend keyed by directory.
    pub async fn open(
        db_name: impl Into<String>,
        node_id: NodeId,
        config_hash: impl Into<String>,
    ) -> Result<Self, AuditError> {
        let db_name = db_name.into();
        let config_hash = config_hash.into();

        let db = Database::open(db_name)
            .with_version(SCHEMA_VERSION)
            .with_on_upgrade_needed(|_event, db| {
                db.create_object_store(OBJECT_STORE).build()?;
                Ok(())
            })
            .await
            .map_err(backend_err)?;

        // Tenants are no longer rehydrated eagerly here — there is no single "current"
        // record anymore, one per tenant instead (S1b), and which tenants exist is not
        // known until a caller names one. Each tenant's own state is lazily rehydrated
        // by `ensure_tenant_loaded` on that tenant's first call.
        Ok(Self {
            db: SendWrapper::new(db),
            state: Mutex::new(HashMap::new()),
            node_id,
            config_hash,
        })
    }

    /// Take the state lock, recovering from poisoning — same rationale as
    /// [`InMemoryAuditStore::state`](super::store::InMemoryAuditStore).
    fn state(&self) -> std::sync::MutexGuard<'_, HashMap<TenantId, State>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Rehydrate `tenant`'s state from IndexedDB into the in-memory map, if it is not
    /// already cached there. A no-op — no DB round trip — once a tenant has been touched
    /// once, since every mutation keeps the cached copy in sync (see `persist`).
    ///
    /// # Legacy database fallback
    ///
    /// A database written before S1b has no per-tenant keys at all — every chain lived
    /// under one bare [`LEGACY_CURRENT_KEY`] record, since there was no tenant concept
    /// to key by. `SCHEMA_VERSION` was not bumped and no migration runs in
    /// `with_on_upgrade_needed`, so without this fallback the default tenant's first
    /// load after upgrading to this version would find nothing under its new
    /// [`snapshot_key`] and silently start a fresh genesis chain — the pre-existing
    /// history would still be sitting in the database, just unreachable. So: only for
    /// the default tenant, and only when the new key has nothing, fall back to the
    /// legacy key. The next mutation persists under the new key as normal (`persist`),
    /// so this fallback is needed at most once per default-tenant chain.
    async fn ensure_tenant_loaded(&self, tenant: &TenantId) -> Result<(), AuditError> {
        {
            let map = self.state();
            if map.contains_key(tenant) {
                return Ok(());
            }
        }

        let mut snapshot: Option<Snapshot> = {
            let tx = self
                .db
                .transaction(OBJECT_STORE)
                .build()
                .map_err(backend_err)?;
            let store = tx.object_store(OBJECT_STORE).map_err(backend_err)?;
            store
                .get(snapshot_key(tenant))
                .serde()
                .map_err(backend_err)?
                .await
                .map_err(backend_err)?
        };

        if snapshot.is_none() && *tenant == TenantId::default_tenant() {
            let tx = self
                .db
                .transaction(OBJECT_STORE)
                .build()
                .map_err(backend_err)?;
            let store = tx.object_store(OBJECT_STORE).map_err(backend_err)?;
            snapshot = store
                .get(LEGACY_CURRENT_KEY.to_string())
                .serde()
                .map_err(backend_err)?
                .await
                .map_err(backend_err)?;
        }

        let state = match snapshot {
            Some(snapshot) => rehydrate(snapshot, &self.node_id, &self.config_hash)?,
            None => State {
                open: Some(Segment::open(
                    self.node_id.clone(),
                    self.config_hash.clone(),
                    None,
                )),
                sealed: Vec::new(),
                closed_seal: None,
            },
        };

        // `or_insert` rather than unconditional insert: two concurrent first-touches of
        // the same tenant may both reach here, and the loser must not clobber whatever
        // the winner (or a mutation racing it) already installed.
        self.state().entry(tenant.clone()).or_insert(state);
        Ok(())
    }

    /// Serialize `tenant`'s current state and write it to its own IndexedDB record.
    /// Called after every mutating method, lock already released — mirrors the file
    /// backend's "build the buffer under the lock, write after" discipline
    /// (`store_file.rs`), just with IndexedDB instead of `spawn_blocking`.
    async fn persist(&self, tenant: &TenantId, snapshot: Snapshot) -> Result<(), AuditError> {
        let tx = self
            .db
            .transaction(OBJECT_STORE)
            .with_mode(TransactionMode::Readwrite)
            .build()
            .map_err(backend_err)?;
        let store = tx.object_store(OBJECT_STORE).map_err(backend_err)?;
        store
            .put(snapshot)
            .with_key(snapshot_key(tenant))
            .with_key_type::<String>()
            .serde()
            .map_err(backend_err)?
            .await
            .map_err(backend_err)?;
        tx.commit().await.map_err(backend_err)?;
        Ok(())
    }
}

fn snapshot_of(state: &State) -> Snapshot {
    Snapshot {
        open: state.open.as_ref().map(|segment| OpenSnapshot {
            segment_id: segment.id().to_owned(),
            entries: segment.entries().to_vec(),
        }),
        sealed: state.sealed.clone(),
        closed_seal: state.closed_seal.clone(),
    }
}

/// Rebuild [`State`] from a persisted [`Snapshot`].
///
/// The open segment's `prev_seal_hash` is not stored in the snapshot — it is always
/// derivable as the most recent sealed segment's `seal_hash` (the same invariant
/// [`super::store::InMemoryAuditStore::rotate`] establishes when it opens a successor),
/// so storing it separately would just be a second place for it to go stale.
///
/// Entries are replayed through [`Segment::append`] rather than trusted as-is, so a
/// snapshot tampered with between sessions is caught here — the same guarantee
/// `FileAuditStore`'s `replay_segment` gives a `.jsonl` file.
fn rehydrate(snapshot: Snapshot, node_id: &NodeId, config_hash: &str) -> Result<State, AuditError> {
    let prev_seal_hash = snapshot
        .sealed
        .last()
        .map(|(seal, _)| seal.seal_hash.clone());

    let open = match snapshot.open {
        Some(open) => {
            let mut segment = Segment::open_with_id(
                open.segment_id,
                node_id.clone(),
                config_hash.to_owned(),
                prev_seal_hash,
            );
            for entry in open.entries {
                segment.append(entry)?;
            }
            Some(segment)
        }
        None => None,
    };

    Ok(State {
        open,
        sealed: snapshot.sealed,
        closed_seal: snapshot.closed_seal,
    })
}

#[async_trait]
impl super::AuditStore for IndexedDbAuditStore {
    async fn append(
        &self,
        tenant: &TenantId,
        inputs: Vec<AuditEntryInput>,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        let tenant = tenant.clone();
        let fut = async move {
            self.ensure_tenant_loaded(&tenant).await?;
            let (result, snapshot) = {
                let mut map = self.state();
                let state = map.get_mut(&tenant).ok_or_else(|| {
                    AuditError::Internal {
                        operation: "append",
                        tenant: tenant.as_str().to_string(),
                        cause: "tenant map entry disappeared after ensure_tenant_loaded".to_string(),
                    }
                })?;
                let segment = state.open.as_mut().ok_or(AuditError::StoreClosed {
                    operation: "append",
                })?;
                let result: Vec<AuditEntry> = inputs
                    .into_iter()
                    .map(|input| segment.push(input).clone())
                    .collect();
                (result, snapshot_of(state))
            };
            self.persist(&tenant, snapshot).await?;
            Ok(result)
        };
        SendWrapper::new(fut).await
    }

    async fn entries(&self, tenant: &TenantId) -> Result<Vec<AuditEntry>, AuditError> {
        let tenant = tenant.clone();
        let fut = async move {
            self.ensure_tenant_loaded(&tenant).await?;
            let map = self.state();
            let state = map.get(&tenant).ok_or_else(|| {
                AuditError::Internal {
                    operation: "entries",
                    tenant: tenant.as_str().to_string(),
                    cause: "tenant map entry disappeared after ensure_tenant_loaded".to_string(),
                }
            })?;
            Ok(state
                .open
                .as_ref()
                .map(|segment| segment.entries().to_vec())
                .unwrap_or_default())
        };
        SendWrapper::new(fut).await
    }

    /// Identical to [`InMemoryAuditStore`](super::InMemoryAuditStore)'s, and for the same
    /// reason: this backend shares [`State`], so the whole history — sealed entries
    /// included — is already in memory. IndexedDB is the durability step behind the
    /// mutating methods, not a store this one has to page through; reading it here would
    /// re-fetch a snapshot of exactly what the guard is already holding.
    async fn history(
        &self,
        tenant: &TenantId,
        after: Option<&AuditCursor>,
        limit: usize,
    ) -> Result<AuditPage, AuditError> {
        let tenant = tenant.clone();
        let fut = async move {
            self.ensure_tenant_loaded(&tenant).await?;
            let map = self.state();
            let state = map.get(&tenant).ok_or_else(|| {
                AuditError::Internal {
                    operation: "history",
                    tenant: tenant.as_str().to_string(),
                    cause: "tenant map entry disappeared after ensure_tenant_loaded".to_string(),
                }
            })?;

            let mut extents: Vec<(&str, u64)> = state
                .sealed
                .iter()
                .map(|(seal, _)| (seal.segment_id.as_str(), seal.entry_count))
                .collect();
            if let Some(segment) = state.open.as_ref() {
                extents.push((segment.id(), segment.len() as u64));
            }

            page_from(&extents, after, limit, |position| {
                Ok(match state.sealed.get(position) {
                    Some((_, entries)) => entries.clone(),
                    // Past the sealed run, so this is the open segment.
                    None => state
                        .open
                        .as_ref()
                        .map(|segment| segment.entries().to_vec())
                        .unwrap_or_default(),
                })
            })
        };
        SendWrapper::new(fut).await
    }

    async fn tip(&self, tenant: &TenantId) -> Result<String, AuditError> {
        let tenant = tenant.clone();
        let fut = async move {
            self.ensure_tenant_loaded(&tenant).await?;
            let map = self.state();
            let state = map.get(&tenant).ok_or_else(|| {
                AuditError::Internal {
                    operation: "tip",
                    tenant: tenant.as_str().to_string(),
                    cause: "tenant map entry disappeared after ensure_tenant_loaded".to_string(),
                }
            })?;
            match state.open.as_ref() {
                Some(segment) if !segment.is_empty() => Ok(segment.tip().to_owned()),
                _ => Ok(state
                    .sealed
                    .last()
                    .map(|(seal, _)| seal.sealed_tip.clone())
                    .unwrap_or_else(|| crate::audit::GENESIS_HASH.to_owned())),
            }
        };
        SendWrapper::new(fut).await
    }

    async fn seals(&self, tenant: &TenantId) -> Result<Vec<SegmentSeal>, AuditError> {
        let tenant = tenant.clone();
        let fut = async move {
            self.ensure_tenant_loaded(&tenant).await?;
            let map = self.state();
            let state = map.get(&tenant).ok_or_else(|| {
                AuditError::Internal {
                    operation: "seals",
                    tenant: tenant.as_str().to_string(),
                    cause: "tenant map entry disappeared after ensure_tenant_loaded".to_string(),
                }
            })?;
            Ok(state.sealed.iter().map(|(seal, _)| seal.clone()).collect())
        };
        SendWrapper::new(fut).await
    }

    async fn verify(&self, tenant: &TenantId) -> Result<(), AuditError> {
        let tenant = tenant.clone();
        let fut = async move {
            self.ensure_tenant_loaded(&tenant).await?;
            let (sealed, open_entries, open_tip) = {
                let map = self.state();
                let state = map.get(&tenant).ok_or_else(|| {
                    AuditError::Internal {
                        operation: "verify",
                        tenant: tenant.as_str().to_string(),
                        cause: "tenant map entry disappeared after ensure_tenant_loaded".to_string(),
                    }
                })?;
                let (entries, tip) = match state.open.as_ref() {
                    Some(segment) => (segment.entries().to_vec(), segment.tip().to_owned()),
                    None => (Vec::new(), crate::audit::GENESIS_HASH.to_owned()),
                };
                (state.sealed.clone(), entries, tip)
            };

            let seals: Vec<SegmentSeal> = sealed.iter().map(|(seal, _)| seal.clone()).collect();
            verify_seal_chain(&seals)?;

            for (seal, entries) in &sealed {
                verify_sealed_entries(entries, seal)?;
            }

            verify_open_entries(&open_entries, &open_tip, &self.config_hash)
        };
        SendWrapper::new(fut).await
    }

    async fn rotate(&self, tenant: &TenantId) -> Result<SegmentSeal, AuditError> {
        let tenant = tenant.clone();
        let fut = async move {
            self.ensure_tenant_loaded(&tenant).await?;
            let (seal, snapshot) = {
                let mut map = self.state();
                let state = map.get_mut(&tenant).ok_or_else(|| {
                    AuditError::Internal {
                        operation: "rotate",
                        tenant: tenant.as_str().to_string(),
                        cause: "tenant map entry disappeared after ensure_tenant_loaded".to_string(),
                    }
                })?;
                let old = state.open.take().ok_or(AuditError::StoreClosed {
                    operation: "rotate",
                })?;
                let entries = old.entries().to_vec();
                let seal = old.seal();

                state.open = Some(Segment::open(
                    self.node_id.clone(),
                    self.config_hash.clone(),
                    Some(seal.seal_hash.clone()),
                ));
                state.sealed.push((seal.clone(), entries));

                (seal, snapshot_of(state))
            };
            self.persist(&tenant, snapshot).await?;
            Ok(seal)
        };
        SendWrapper::new(fut).await
    }

    async fn close(&self, tenant: &TenantId) -> Result<SegmentSeal, AuditError> {
        let tenant = tenant.clone();
        let fut = async move {
            self.ensure_tenant_loaded(&tenant).await?;
            let (seal, snapshot) = {
                let mut map = self.state();
                let state = map.get_mut(&tenant).ok_or_else(|| {
                    AuditError::Internal {
                        operation: "close",
                        tenant: tenant.as_str().to_string(),
                        cause: "tenant map entry disappeared after ensure_tenant_loaded".to_string(),
                    }
                })?;

                if let Some(seal) = &state.closed_seal {
                    return Ok(seal.clone());
                }

                let segment = state
                    .open
                    .take()
                    .ok_or(AuditError::StoreClosed { operation: "close" })?;
                let entries = segment.entries().to_vec();
                let seal = segment.seal();

                state.sealed.push((seal.clone(), entries));
                state.closed_seal = Some(seal.clone());

                (seal, snapshot_of(state))
            };
            self.persist(&tenant, snapshot).await?;
            Ok(seal)
        };
        SendWrapper::new(fut).await
    }
}
