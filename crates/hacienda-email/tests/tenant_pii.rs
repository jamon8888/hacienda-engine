//! Multi-tenant P0.4/P0.5 regression suite for the email backend.
//!
//! A and B are two tenants with **different pseudonym keys** on the same host
//! facade. The assertions pin the security contract:
//!
//! * a metadata token minted for A is not transferable to B — B's key cannot
//!   reveal it, and B mints a different token for the same clear value;
//! * all email PII work goes through the **shared** facade, so A's audit chain
//!   records the redaction and B's chain stays empty;
//! * RAG rows are stamped with A's tenant and are invisible to a B-scoped
//!   retrieval, while their metadata carries A's token only.
//!
//! No local hash, local secret, `HaciendaFacade::new` or `EnvKeyResolver` is
//! created inside the email processing path: the resolver here is supplied by
//! the "host" (this test) just as `ApiState::facade` supplies it in HTTP.

use hacienda_core::auth::{AuthContext, Caller, Capability, CapabilitySet};
use hacienda_core::redaction::pseudonym::{EnvKeyResolver, KEY_BYTES};
use hacienda_core::tenancy::TenantId;
use hacienda_core::{HaciendaConfig, HaciendaFacade};
use hacienda_email::pii::PiiEngine;
use hacienda_email::rag::RagIngestion;
use hacienda_email::{
    models::{EmailAddress, EmailAttachment, EmailMessage, Envelope},
    EmailConfig, EmailDestination, EmailOperation, EmailSource,
};
use hacienda_rag::{CollectionSpec, InMemoryVectorStore, RagStore, RetrieveQuery};
use std::sync::Arc;
use tempfile::tempdir;

const TENANT_A: &str = "tenant_a";
const TENANT_B: &str = "tenant_b";
const CLEAR_EMAIL: &str = "jane.doe@acme.example";
const CLEAR_SUBJECT: &str = "jane doe confidential review";

/// The host-side resolver: two tenants, two distinct key materials. Uses the
/// public env-name convention so the real `EnvKeyResolver` code path is
/// exercised without mutating process-global environment variables.
fn host_resolver_exact() -> Arc<dyn hacienda_core::redaction::KeyResolver> {
    Arc::new(EnvKeyResolver::with_lookup(|name| match name {
        // The default tenant is resolved eagerly at facade construction; give
        // it its own material so startup does not fail. It is not used by A/B.
        "HACIENDA_PSEUDONYM_ACTIVE_KEY" => Some("k1".to_string()),
        "HACIENDA_PSEUDONYM_KEY_K1" => Some("11".repeat(KEY_BYTES)),
        "HACIENDA_TENANT_TENANT_A_HACIENDA_PSEUDONYM_ACTIVE_KEY" => Some("k1".to_string()),
        "HACIENDA_TENANT_TENANT_A_HACIENDA_PSEUDONYM_KEY_K1" => Some("07".repeat(KEY_BYTES)),
        "HACIENDA_TENANT_TENANT_B_HACIENDA_PSEUDONYM_ACTIVE_KEY" => Some("k1".to_string()),
        "HACIENDA_TENANT_TENANT_B_HACIENDA_PSEUDONYM_KEY_K1" => Some("a9".repeat(KEY_BYTES)),
        _ => None,
    }))
}

fn host_facade() -> Arc<HaciendaFacade> {
    let config = HaciendaConfig::default().with_pii(hacienda_core::pii::PipelineConfig::default());
    Arc::new(
        HaciendaFacade::with_key_resolver(config, host_resolver_exact(), &[])
            .expect("host facade with two tenant keys"),
    )
}

fn ctx(tenant: &str, principal: &str, caps: &[Capability]) -> AuthContext {
    AuthContext::with_tenant(
        principal,
        TenantId::new(tenant),
        CapabilitySet::new(caps.iter().copied()),
    )
}

fn process_caller(tenant: &str, principal: &str) -> AuthContext {
    ctx(
        tenant,
        principal,
        &[Capability::DocumentsProcess, Capability::PiiReveal],
    )
}

struct FixtureClient {
    envelope: Envelope,
    message: EmailMessage,
}

#[async_trait::async_trait]
impl hacienda_email::backend::EmailClient for FixtureClient {
    async fn list_mailboxes(&self) -> hacienda_email::Result<Vec<hacienda_email::models::Mailbox>> {
        Ok(Vec::new())
    }
    async fn list_envelopes(
        &self,
        _mailbox: &str,
        _limit: Option<usize>,
    ) -> hacienda_email::Result<Vec<Envelope>> {
        Ok(vec![self.envelope.clone()])
    }
    async fn search_envelopes(
        &self,
        _mailbox: &str,
        _query: &str,
    ) -> hacienda_email::Result<Vec<Envelope>> {
        Ok(Vec::new())
    }
    async fn get_message(
        &self,
        _mailbox: &str,
        _uid: &str,
    ) -> hacienda_email::Result<EmailMessage> {
        Ok(self.message.clone())
    }
    async fn add_message(
        &self,
        _mailbox: &str,
        _message: &EmailMessage,
    ) -> hacienda_email::Result<()> {
        Ok(())
    }
    async fn copy_messages(
        &self,
        _src: &str,
        _dst: &str,
        _uids: &[String],
    ) -> hacienda_email::Result<()> {
        Ok(())
    }
    async fn move_messages(
        &self,
        _src: &str,
        _dst: &str,
        _uids: &[String],
    ) -> hacienda_email::Result<()> {
        Ok(())
    }
    async fn delete_messages(
        &self,
        _mailbox: &str,
        _uids: &[String],
    ) -> hacienda_email::Result<()> {
        Ok(())
    }
    async fn store_flags(
        &self,
        _mailbox: &str,
        _uids: &[String],
        _flags: &[hacienda_email::models::Flag],
        _add: bool,
    ) -> hacienda_email::Result<()> {
        Ok(())
    }
    async fn send_message(&self, _message: &EmailMessage) -> hacienda_email::Result<()> {
        Ok(())
    }
}

fn fixture() -> FixtureClient {
    let envelope = Envelope {
        id: "uid-a-1".to_string(),
        mailbox: "INBOX".to_string(),
        subject: CLEAR_SUBJECT.to_string(),
        from: vec![EmailAddress {
            name: Some("Jane Doe".to_string()),
            address: CLEAR_EMAIL.to_string(),
        }],
        to: Vec::new(),
        date: chrono::Utc::now(),
        flags: Vec::new(),
        has_attachments: true,
    };
    let message = EmailMessage {
        id: envelope.id.clone(),
        mailbox: envelope.mailbox.clone(),
        subject: envelope.subject.clone(),
        from: envelope.from.clone(),
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        date: envelope.date,
        text_body: Some(format!("Contact {CLEAR_EMAIL} for the review")),
        html_body: None,
        attachments: vec![EmailAttachment {
            filename: "jane-doe-review.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            size: 4,
            content_id: None,
            content: vec![0; 4],
        }],
        flags: Vec::new(),
    };
    FixtureClient { envelope, message }
}

#[tokio::test]
async fn metadata_tokens_are_tenant_bound_and_not_revealable_under_b() {
    let facade = host_facade();
    let pii = PiiEngine::from_facade(facade.clone());
    let a = process_caller(TENANT_A, "alice");
    let b = process_caller(TENANT_B, "bob");

    let token_a = hacienda_email::redacted::metadata_token(
        &pii,
        Caller::Principal(&a),
        "subject",
        CLEAR_EMAIL,
    )
    .await
    .unwrap();
    let token_b = hacienda_email::redacted::metadata_token(
        &pii,
        Caller::Principal(&b),
        "subject",
        CLEAR_EMAIL,
    )
    .await
    .unwrap();

    assert_ne!(
        token_a, token_b,
        "same value must not correlate across tenants"
    );
    assert!(!token_a.contains("jane") && !token_a.contains("acme"));
    assert!(!token_b.contains("jane") && !token_b.contains("acme"));

    // A can reveal its own token through the shared facade.
    let revealed = facade
        .reveal_token_with_auth(Caller::Principal(&a), &token_a)
        .await
        .unwrap();
    assert_eq!(revealed, CLEAR_EMAIL);

    // B's key cannot read A's token. The failure is indistinguishable from an
    // unknown/malformed token — no "valid for another tenant" oracle.
    let rejected = facade
        .reveal_token_with_auth(Caller::Principal(&b), &token_a)
        .await;
    assert!(rejected.is_err(), "B must not be able to reveal A's token");
}

#[tokio::test]
async fn email_redaction_writes_to_shared_audit_under_a_only() {
    let facade = host_facade();
    let pii = PiiEngine::from_facade(facade.clone());
    let a = process_caller(TENANT_A, "alice");
    let dir = tempdir().unwrap();

    let op = EmailOperation::Redact {
        source: EmailSource::Account {
            name: "test".to_string(),
            mailbox: None,
        },
        destination: EmailDestination::Files {
            path: dir.path().to_str().unwrap().to_string(),
        },
        rules: None,
    };
    let response = hacienda_email::redact::redact(
        &fixture(),
        &pii,
        Caller::Principal(&a),
        op,
        &EmailConfig::default(),
    )
    .await
    .unwrap();
    assert!(response.success, "errors: {:?}", response.errors);

    // The redacted file carries A's token, never the clear metadata.
    let written: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "eml"))
        .collect();
    assert_eq!(written.len(), 1);
    let eml = std::fs::read_to_string(written[0].path()).unwrap();
    for clear in [
        CLEAR_EMAIL,
        CLEAR_SUBJECT,
        "Jane Doe",
        "jane-doe-review.pdf",
    ] {
        assert!(!eml.contains(clear), "clear value {clear:?} leaked: {eml}");
    }
    let expected_subject = hacienda_email::redacted::metadata_token(
        &pii,
        Caller::Principal(&a),
        "subject",
        CLEAR_SUBJECT,
    )
    .await
    .unwrap();
    assert!(
        eml.contains(&expected_subject),
        "output must carry A's tenant-keyed subject token: {eml}"
    );

    // Audit: the shared facade's A chain recorded the redaction; B's is empty.
    let a_audit = ctx(TENANT_A, "auditor", &[Capability::AuditRead]);
    let page = facade
        .audit_history_with_auth(Caller::Principal(&a_audit), None, 100)
        .await
        .unwrap()
        .expect("audit enabled");
    assert!(
        !page.entries.is_empty(),
        "A's audit chain must be populated"
    );
    assert!(
        page.entries
            .iter()
            .any(|e| e.principal.as_deref() == Some("alice")),
        "the email redaction must be attributed to A's principal in the shared audit"
    );
    assert!(
        facade
            .audit_tip_with_auth(Caller::Principal(&a))
            .await
            .unwrap()
            .is_some(),
        "A's chain tip must be available from the shared facade"
    );

    let b_audit = ctx(TENANT_B, "auditor", &[Capability::AuditRead]);
    let b_page = facade
        .audit_history_with_auth(Caller::Principal(&b_audit), None, 100)
        .await
        .unwrap()
        .expect("audit enabled");
    assert!(b_page.entries.is_empty(), "B's audit chain must stay empty");
}

#[tokio::test]
async fn rag_rows_are_scoped_to_a_with_a_keyed_metadata() {
    let facade = host_facade();
    let pii = PiiEngine::from_facade(facade);
    let a = process_caller(TENANT_A, "alice");
    let store = InMemoryVectorStore::new("email-tenant-ab");
    let collection = "email-corpus";
    store
        .ensure_collection(&CollectionSpec::new(collection, 0))
        .await
        .unwrap();

    let rag = RagIngestion {
        store: &store,
        dossier_id: None,
        account: TENANT_A,
        tenant_id: Some(TENANT_A),
        page_size: 10,
        max_pages: 1,
        concurrency: 1,
        incremental: false,
        optimize_after_run: false,
        persist_watermark: false,
        on_page: None,
    };
    let op = EmailOperation::IndexForRag {
        source: EmailSource::Account {
            name: "test".to_string(),
            mailbox: None,
        },
        index_name: Some(collection.to_string()),
        chunk_size: Some(512),
    };
    let response = hacienda_email::rag::index_for_rag(
        &fixture(),
        &pii,
        Caller::Principal(&a),
        op,
        &EmailConfig::default(),
        Some(rag),
    )
    .await
    .unwrap();
    assert!(response.success, "errors: {:?}", response.errors);

    let (summaries, _) = store.list_documents(collection, 10, 0).await.unwrap();
    assert_eq!(summaries.len(), 1);
    let (record, chunks) = store
        .get_document_chunks(collection, &summaries[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.tenant_id.as_deref(), Some(TENANT_A));
    assert!(chunks
        .iter()
        .all(|c| c.tenant_id.as_deref() == Some(TENANT_A)));

    let a_subject = hacienda_email::redacted::metadata_token(
        &pii,
        Caller::Principal(&a),
        "subject",
        CLEAR_SUBJECT,
    )
    .await
    .unwrap();
    let b_subject = {
        let b = process_caller(TENANT_B, "bob");
        hacienda_email::redacted::metadata_token(
            &pii,
            Caller::Principal(&b),
            "subject",
            CLEAR_SUBJECT,
        )
        .await
        .unwrap()
    };
    let snapshot = serde_json::to_string(&(&summaries, &record, &chunks)).unwrap();
    assert!(snapshot.contains(&a_subject), "A's token must be persisted");
    assert!(
        !snapshot.contains(&b_subject),
        "B's token must not appear in A's rows"
    );
    for clear in [
        CLEAR_EMAIL,
        CLEAR_SUBJECT,
        "Jane Doe",
        "jane-doe-review.pdf",
    ] {
        assert!(
            !snapshot.contains(clear),
            "clear {clear:?} leaked into RAG rows"
        );
    }

    // Retrieval honours the tenant boundary: A sees the chunk, B does not.
    // The in-memory backend has no full-text index and email chunks are BYOV
    // (dim 0), so use a vector query with the matching empty query vector.
    let mut qa = RetrieveQuery::vector(10);
    qa.query_vector = Some(Vec::new());
    qa.tenant_id = Some(TENANT_A.to_string());
    let out_a = store.retrieve(collection, &qa).await.unwrap();
    assert!(!out_a.chunks.is_empty(), "A must retrieve its own email chunks");

    let mut qb = RetrieveQuery::vector(10);
    qb.query_vector = Some(Vec::new());
    qb.tenant_id = Some(TENANT_B.to_string());
    let out_b = store.retrieve(collection, &qb).await.unwrap();
    assert!(out_b.chunks.is_empty(), "B must not retrieve A's email chunks");
}
