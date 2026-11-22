//! LocalFull native email boundary coverage (P0.5).
//!
//! Exercises the real path on a checked-in Maildir fixture:
//!
//!   register account -> token-only message list -> RAG ingestion -> retrieve
//!   -> message-bound capability -> reveal + open the ORIGINAL in the native
//!   viewer.
//!
//! The assertions pin the renderer boundary: every serialized DTO is free of
//! clear subject/address/display-name/SSN text and of absolute paths. A token
//! minted under a second tenant is rejected by the first tenant's capability,
//! and a replayed capability is rejected too.

use hacienda_core::audit::{AuditStore, InMemoryAuditStore};
use hacienda_core::redaction::pseudonym::{
    KeyId, PseudonymKey, Pseudonymiser, KEY_BYTES,
};
use hacienda_core::redaction::{KeyResolver, PseudonymError};
use hacienda_core::tenancy::TenantId;
use hacienda_core::HaciendaConfig;
use hacienda_email::native::{
    EmailSessionService, NativeEmailError, NativeMessageViewer, RegisterAccountRequest,
};
use hacienda_email::pii::PiiEngine;
use hacienda_rag::{CollectionSpec, InMemoryVectorStore, RagStore};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

const CLEAR_SUBJECT: &str = "Jane Doe SSN 123-45-6789 quarterly review";
const CLEAR_ADDRESS: &str = "jane.doe@acme.example";
const CLEAR_NAME: &str = "Jane Doe";
const CLEAR_SSN: &str = "123-45-6789";

/// Mirrors the LocalFull OS-keychain resolver: one shared key for every tenant,
/// so cross-tenant separation must come from the token domain + capability
/// boundary rather than from a second key.
#[derive(Debug)]
struct SharedKeyResolver(Zeroizing<[u8; KEY_BYTES]>);

impl KeyResolver for SharedKeyResolver {
    fn active(
        &self,
        _tenant: &TenantId,
    ) -> std::result::Result<PseudonymKey, PseudonymError> {
        PseudonymKey::from_bytes(KeyId::new("local")?, self.0.as_ref())
    }

    fn resolve(
        &self,
        _tenant: &TenantId,
        _id: &KeyId,
    ) -> std::result::Result<PseudonymKey, PseudonymError> {
        self.active(_tenant)
    }
}

fn resolver() -> Arc<dyn KeyResolver> {
    Arc::new(SharedKeyResolver(Zeroizing::new([7_u8; KEY_BYTES])))
}

#[derive(Default)]
struct RecordingViewer {
    opened: Mutex<Vec<(PathBuf, Vec<String>)>>,
}

impl NativeMessageViewer for RecordingViewer {
    fn open_message(
        &self,
        path: &Path,
        revealed: &[(String, Zeroizing<String>)],
    ) -> std::result::Result<(), ()> {
        self.opened.lock().map_err(|_| ())?.push((
            path.to_path_buf(),
            revealed.iter().map(|(_, value)| value.to_string()).collect(),
        ));
        Ok(())
    }
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/maildir")
}

fn assert_content_free(serialized: &str, label: &str) {
    for clear in [
        CLEAR_SUBJECT,
        CLEAR_ADDRESS,
        CLEAR_NAME,
        CLEAR_SSN,
        "richard.roe@acme.example",
        "Richard Roe",
    ] {
        assert!(
            !serialized.contains(clear),
            "clear {clear:?} crossed the renderer boundary in {label}: {serialized}"
        );
    }
    assert!(
        !serialized.contains(fixture_root().to_string_lossy().as_ref()),
        "absolute Maildir path crossed the renderer boundary in {label}: {serialized}"
    );
}

#[tokio::test]
async fn maildir_ingest_retrieve_and_message_bound_reveal() {
    let store = InMemoryVectorStore::new("native-maildir-e2e");
    store
        .ensure_collection(&CollectionSpec::new("email-corpus", 0))
        .await
        .unwrap();
    let audit = Arc::new(InMemoryAuditStore::new("native-maildir-audit"));
    let viewer = Arc::new(RecordingViewer::default());
    let pii = PiiEngine::for_local_use(
        HaciendaConfig::default().with_pii(hacienda_core::pii::PipelineConfig::default()),
        resolver(),
    )
    .unwrap();
    let store: Arc<dyn RagStore> = Arc::new(store);
    let service = EmailSessionService::new(pii, store, audit.clone(), viewer.clone());

    // 1. Native account registration (path never comes from the renderer).
    let account = service
        .register_account(RegisterAccountRequest {
            tenant: "tenant_a".to_owned(),
            principal: "alice".to_owned(),
            maildir_root: fixture_root(),
            label: "work".to_owned(),
        })
        .await
        .unwrap();
    assert_content_free(&serde_json::to_string(&account).unwrap(), "account");
    assert_eq!(account.mailbox_count, 1);

    // 2. Token-only mailbox + message projections.
    let mailboxes = service.list_mailboxes(&account.account_ref).await.unwrap();
    assert_content_free(&serde_json::to_string(&mailboxes).unwrap(), "mailboxes");
    let mailbox_ref = &mailboxes[0].mailbox_ref;
    let messages = service
        .list_messages(&account.account_ref, mailbox_ref)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    let serialized_messages = serde_json::to_string(&messages).unwrap();
    assert_content_free(&serialized_messages, "messages");
    let message_ref = &messages[0].message_ref;
    let subject_token = messages[0].envelope_token_only["subject"]
        .as_str()
        .expect("subject token")
        .to_owned();
    assert!(subject_token.starts_with("[EMAIL.TENANT_A.SUBJECT:"));

    // 3. Ingestion into the shared RAG store, with content-free job counters.
    let job = service
        .ingest(&account.account_ref, mailbox_ref)
        .await
        .unwrap();
    assert_eq!(job.status, "completed");
    assert_eq!(job.errors, 0);
    assert!(job.chunks_persisted > 0, "job must persist redacted chunks");
    assert_content_free(&serde_json::to_string(&job).unwrap(), "job");

    // 4. RAG retrieve -> token-only hit mapped back to the opaque message ref.
    let hits = service
        .search(&account.account_ref, CLEAR_ADDRESS)
        .await
        .unwrap();
    assert!(!hits.is_empty(), "retrieval must find the ingested email");
    assert_content_free(&serde_json::to_string(&hits).unwrap(), "search hits");
    assert_eq!(hits[0].message_ref.as_deref(), Some(message_ref.as_str()));

    // 5. Message-bound capability, then reveal + open inside the native viewer.
    let viewer_label = "email-viewer-e2e";
    let capability = service
        .authorize_message_view(message_ref, viewer_label)
        .await
        .unwrap();
    assert_content_free(&serde_json::to_string(&capability).unwrap(), "capability");
    let opened = service
        .reveal_and_open(
            &capability.capability_ref,
            message_ref,
            viewer_label,
            &subject_token,
        )
        .await
        .unwrap();
    assert!(opened.opened);

    let captured = viewer.opened.lock().unwrap();
    assert_eq!(captured.len(), 1);
    let (opened_path, revealed) = &captured[0];
    assert!(
        opened_path.starts_with(fixture_root().join("cur"))
            || opened_path.starts_with(fixture_root().join("new")),
        "the opened original must live inside the provisioned Maildir"
    );
    assert!(opened_path.is_file(), "the ORIGINAL RFC822 must be opened");
    let original = std::fs::read_to_string(opened_path).unwrap();
    assert!(original.contains(CLEAR_SUBJECT), "fixture is the clear original");
    // Reversible pseudonyms normalize case (NFKC + lowercase) before encryption,
    // so the revealed value is the normalized cleartext.
    assert_eq!(revealed.len(), 1);
    assert_eq!(revealed[0].to_lowercase(), CLEAR_SUBJECT.to_lowercase());
    drop(captured);

    // A replayed capability cannot reveal again.
    assert_eq!(
        service
            .reveal_and_open(
                &capability.capability_ref,
                message_ref,
                viewer_label,
                &subject_token,
            )
            .await,
        Err(NativeEmailError::Replayed)
    );

    // 6. Email tokens are accepted by the viewer's reveal path: the same
    //    resolver/key (not a second key) decrypts the email-minted token.
    let viewer_pseudonymiser =
        Pseudonymiser::new(resolver().as_ref(), &TenantId::default_tenant(), &[]).unwrap();
    assert_eq!(
        viewer_pseudonymiser
            .reveal(&subject_token)
            .unwrap()
            .to_lowercase(),
        CLEAR_SUBJECT.to_lowercase()
    );

    // 7. A token minted for another tenant is rejected by this message's
    //    capability (different token value + different category AAD).
    let other = service
        .register_account(RegisterAccountRequest {
            tenant: "tenant_b".to_owned(),
            principal: "bob".to_owned(),
            maildir_root: fixture_root(),
            label: "work".to_owned(),
        })
        .await
        .unwrap();
    let other_mailboxes = service.list_mailboxes(&other.account_ref).await.unwrap();
    let other_messages = service
        .list_messages(&other.account_ref, &other_mailboxes[0].mailbox_ref)
        .await
        .unwrap();
    let other_token = other_messages[0].envelope_token_only["subject"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(other_token, subject_token);
    assert!(other_token.starts_with("[EMAIL.TENANT_B.SUBJECT:"));
    let capability_b = service
        .authorize_message_view(&other_messages[0].message_ref, viewer_label)
        .await
        .unwrap();
    assert_eq!(
        service
            .reveal_and_open(
                &capability_b.capability_ref,
                &other_messages[0].message_ref,
                viewer_label,
                &subject_token,
            )
            .await,
        Err(NativeEmailError::TokenNotBound)
    );

    // Both reveals/authorizations are audited, content-free.
    let entries = audit.entries(&TenantId::new("tenant_a")).await.unwrap();
    assert!(!entries.is_empty());
    for entry in &entries {
        let serialized = serde_json::to_string(entry).unwrap();
        assert_content_free(&serialized, "audit entry");
    }
}
