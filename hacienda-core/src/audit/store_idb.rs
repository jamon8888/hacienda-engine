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
use std::sync::Mutex;

use super::cursor::{page_from, AuditCursor, AuditPage};
use super::entry::{AuditEntry, AuditEntryInput};
use super::error::AuditError;
use super::segment::{verify_seal_chain, NodeId, Segment, SegmentSeal};
use super::store::{verify_open_entries, verify_sealed_entries, State};

const OBJECT_STORE: &str = "hacienda_audit_snapshot";
const SNAPSHOT_KEY: &str = "current";
const SCHEMA_VERSION: u32 = 1;

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
pub struct IndexedDbAuditStore {
    db: SendWrapper<Database>,
    state: Mutex<State>,
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

        let snapshot: Option<Snapshot> = {
            let tx = db.transaction(OBJECT_STORE).build().map_err(backend_err)?;
            let store = tx.object_store(OBJECT_STORE).map_err(backend_err)?;
            store
                .get(SNAPSHOT_KEY)
                .serde()
                .map_err(backend_err)?
                .await
                .map_err(backend_err)?
        };

        let state = match snapshot {
            Some(snapshot) => rehydrate(snapshot, &node_id, &config_hash)?,
            None => State {
                open: Some(Segment::open(node_id.clone(), config_hash.clone(), None)),
                sealed: Vec::new(),
                closed_seal: None,
            },
        };

        Ok(Self {
            db: SendWrapper::new(db),
            state: Mutex::new(state),
            node_id,
            config_hash,
        })
    }

    /// Take the state lock, recovering from poisoning — same rationale as
    /// [`InMemoryAuditStore::state`](super::store::InMemoryAuditStore).
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Serialize the current state and write it to IndexedDB. Called after every
    /// mutating method, lock already released — mirrors the file backend's "build the
    /// buffer under the lock, write after" discipline (`store_file.rs`), just with
    /// IndexedDB instead of `spawn_blocking`.
    async fn persist(&self, snapshot: Snapshot) -> Result<(), AuditError> {
        let tx = self
            .db
            .transaction(OBJECT_STORE)
            .with_mode(TransactionMode::Readwrite)
            .build()
            .map_err(backend_err)?;
        let store = tx.object_store(OBJECT_STORE).map_err(backend_err)?;
        store
            .put(snapshot)
            .with_key(SNAPSHOT_KEY)
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
                crate::tenancy::TenantId::default_tenant(),
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
    async fn append(&self, inputs: Vec<AuditEntryInput>) -> Result<Vec<AuditEntry>, AuditError> {
        let fut = async move {
            let (result, snapshot) = {
                let mut state = self.state();
                let segment = state.open.as_mut().ok_or(AuditError::StoreClosed {
                    operation: "append",
                })?;
                let result: Vec<AuditEntry> = inputs
                    .into_iter()
                    .map(|input| segment.push(input).clone())
                    .collect();
                (result, snapshot_of(&state))
            };
            self.persist(snapshot).await?;
            Ok(result)
        };
        SendWrapper::new(fut).await
    }

    async fn entries(&self) -> Result<Vec<AuditEntry>, AuditError> {
        let state = self.state();
        Ok(state
            .open
            .as_ref()
            .map(|segment| segment.entries().to_vec())
            .unwrap_or_default())
    }

    /// Identical to [`InMemoryAuditStore`](super::InMemoryAuditStore)'s, and for the same
    /// reason: this backend shares [`State`], so the whole history — sealed entries
    /// included — is already in memory. IndexedDB is the durability step behind the
    /// mutating methods, not a store this one has to page through; reading it here would
    /// re-fetch a snapshot of exactly what the guard is already holding.
    async fn history(
        &self,
        after: Option<&AuditCursor>,
        limit: usize,
    ) -> Result<AuditPage, AuditError> {
        let state = self.state();

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
    }

    async fn tip(&self) -> Result<String, AuditError> {
        let state = self.state();
        match state.open.as_ref() {
            Some(segment) if !segment.is_empty() => Ok(segment.tip().to_owned()),
            _ => Ok(state
                .sealed
                .last()
                .map(|(seal, _)| seal.sealed_tip.clone())
                .unwrap_or_else(|| crate::audit::GENESIS_HASH.to_owned())),
        }
    }

    async fn seals(&self) -> Result<Vec<SegmentSeal>, AuditError> {
        let state = self.state();
        Ok(state.sealed.iter().map(|(seal, _)| seal.clone()).collect())
    }

    async fn verify(&self) -> Result<(), AuditError> {
        let (sealed, open_entries, open_tip) = {
            let state = self.state();
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
    }

    async fn rotate(&self) -> Result<SegmentSeal, AuditError> {
        let fut = async move {
            let (seal, snapshot) = {
                let mut state = self.state();
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

                (seal, snapshot_of(&state))
            };
            self.persist(snapshot).await?;
            Ok(seal)
        };
        SendWrapper::new(fut).await
    }

    async fn close(&self) -> Result<SegmentSeal, AuditError> {
        let fut = async move {
            let (seal, snapshot) = {
                let mut state = self.state();

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

                (seal, snapshot_of(&state))
            };
            self.persist(snapshot).await?;
            Ok(seal)
        };
        SendWrapper::new(fut).await
    }
}
