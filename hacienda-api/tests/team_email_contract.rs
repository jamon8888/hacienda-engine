//! Contract / integration tests for the TeamOnPremise email surface.
//!
//! These drive the real axum router (auth middleware, capability guards,
//! extractors and handlers) rather than calling handlers directly, so they pin
//! the renderer-visible boundary: opaque refs only, content-free errors, and a
//! viewer capability that is usable only by the tenant + principal that minted
//! it. No raw email, attachment or pseudonym plaintext may appear in a job, an
//! error, a log-shaped string or a renderer DTO.

use axum::{body::Body, http::Request, response::Response, Router};
use hacienda_api::{
    router,
    team_email::{opaque_ref, EmailSourceRegistry, ViewerCapabilityStore},
    ApiLimits, ApiState,
};
use hacienda_core::{
    auth::{
        authn::{InMemoryTokenStore, Token},
        AuthState, Capability, CapabilitySet,
    },
    jobs::InMemoryJobStore,
    pii::PipelineConfig,
    redaction::{EnvKeyResolver, RedactionConfig, RedactionMode},
    tenancy::TenantId,
    HaciendaConfig, HaciendaFacade,
};
use hacienda_rag::{
    ChunkRecord, CollectionSpec, DistanceMetric, DocumentRecord, InMemoryVectorStore, IndexMethod,
    RagStore,
};
use std::sync::Arc;
use tower::ServiceExt;

const TENANT_A: &str = "tok-acme";
const TENANT_B: &str = "tok-globex";
const LIMITED: &str = "tok-limited";

fn test_state() -> ApiState {
    test_state_with_store().0
}

fn test_state_with_store() -> (ApiState, Arc<dyn RagStore>) {
    // Pseudonymize mode with a deterministic host key so a bracketed token can
    // be revealed server-side by its owning tenant.
    let resolver = EnvKeyResolver::with_lookup(|name: &str| {
        if name.ends_with("HACIENDA_PSEUDONYM_ACTIVE_KEY") {
            Some("k1".to_string())
        } else if name.ends_with("HACIENDA_PSEUDONYM_KEY_K1") {
            Some("07".repeat(64))
        } else {
            None
        }
    });
    let pii = PipelineConfig {
        redaction: RedactionConfig {
            mode: RedactionMode::Pseudonymize,
            ..Default::default()
        },
        ..Default::default()
    };
    let facade = Arc::new(
        HaciendaFacade::with_key_resolver(
            HaciendaConfig::default().with_pii(pii),
            Arc::new(resolver),
            &[],
        )
        .unwrap(),
    );

    // Three real, distinct identities. The dev token resolver cannot express a
    // tenant, so an in-memory token store supplies them explicitly.
    let mut tokens = InMemoryTokenStore::new();
    let caps = CapabilitySet::new([
        Capability::DocumentsProcess,
        Capability::PiiReveal,
        Capability::AuditRead,
    ]);
    tokens.insert(
        TENANT_A,
        Token::new("id-alice", "alice", caps.clone()).with_tenant(TenantId::new("acme")),
    );
    tokens.insert(
        TENANT_B,
        Token::new("id-bob", "bob", caps).with_tenant(TenantId::new("globex")),
    );
    tokens.insert(
        LIMITED,
        Token::new(
            "id-limited",
            "carol",
            CapabilitySet::new([Capability::DocumentsProcess]),
        )
        .with_tenant(TenantId::new("acme")),
    );

    let store: Arc<dyn RagStore> = Arc::new(InMemoryVectorStore::new("team-contract"));
    let state = ApiState::new(
        facade,
        InMemoryJobStore::new().into_arc(),
        AuthState::new(Arc::new(tokens)),
        ApiLimits::default(),
    )
    .with_rag_store(store.clone())
    // Only opaque refs are provisioned. No absolute path can ever be selected.
    .with_email_source_registry(EmailSourceRegistry::from_parts(vec![(
        "default".to_string(),
        vec![("inbox".to_string(), "INBOX".to_string())],
    )]))
    .with_viewer_capability_store(ViewerCapabilityStore::default());
    (state, store)
}

async fn call(app: &Router, method: &str, uri: &str, token: &str, body: Option<&str>) -> Response {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    app.clone()
        .oneshot(
            builder
                .body(Body::from(body.unwrap_or("").to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn response_text(response: Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn response_json(response: Response) -> serde_json::Value {
    serde_json::from_str(&response_text(response).await).unwrap()
}

#[tokio::test]
async fn capability_is_tenant_and_principal_bound_and_cross_tenant_use_is_audited() {
    let app = router(test_state());

    // A bracketed pseudonym is the only token shape a capability accepts.
    let issued = response_json(
        call(
            &app,
            "POST",
            "/v1/team/viewer/capability",
            TENANT_A,
            Some(
                r#"{"document_ref":"doc1","email_ref":"msg1","token_ref":"[EMAIL:k1:AAAA]","purpose":"review"}"#,
            ),
        )
        .await,
    )
    .await;
    // The route must have succeeded; a 400 envelope has no capability_ref.
    let capability_ref = issued["capability_ref"]
        .as_str()
        .expect("capability must be issued for a valid pseudonym")
        .to_string();
    assert!(capability_ref.starts_with("vcap_"));
    for secret in ["acme", "alice", "doc1", "msg1", "EMAIL"] {
        assert!(
            !capability_ref.contains(secret),
            "capability leaked `{secret}`"
        );
    }

    // Same principal, different tenant: denied. Same tenant, different principal
    // is covered by the store-level unit test; the wire behaviour must be the
    // same generic 403 with no hint about the owning tenant.
    let denial = call(
        &app,
        "POST",
        "/v1/team/viewer/reveal",
        TENANT_B,
        Some(&format!(r#"{{"capability_ref":"{capability_ref}"}}"#)),
    )
    .await;
    assert_eq!(denial.status().as_u16(), 403);
    let denial_text = response_text(denial).await;
    for secret in ["alice", "acme", "doc1", "msg1", "[EMAIL:"] {
        assert!(
            !denial_text.contains(secret),
            "denial response leaked `{secret}`: {denial_text}"
        );
    }

    // The audit trail is visible only to the caller's own tenant, and is itself
    // content-free (refs, action and outcome only).
    let globex_audit =
        response_json(call(&app, "GET", "/v1/team/viewer/audit", TENANT_B, None).await).await;
    let events = globex_audit["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["outcome"], "denied");
    assert_eq!(events[0]["action"], "reveal");
    assert_eq!(events[0]["tenant"], "globex");
    assert_eq!(events[0]["capability_ref"], capability_ref);
    let audit_text = globex_audit.to_string();
    for secret in ["jean.dupont@example.com", "123-45-6789", "/srv/mail"] {
        assert!(!audit_text.contains(secret), "audit leaked `{secret}`");
    }

    let acme_audit =
        response_json(call(&app, "GET", "/v1/team/viewer/audit", TENANT_A, None).await).await;
    assert_eq!(acme_audit["events"][0]["outcome"], "issued");
    assert_eq!(acme_audit["events"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn viewer_audit_requires_audit_read_not_just_documents_process() {
    let app = router(test_state());
    let response = call(&app, "GET", "/v1/team/viewer/audit", LIMITED, None).await;
    assert_eq!(response.status().as_u16(), 403);
}

#[tokio::test]
async fn raw_pii_is_rejected_as_a_token_ref_and_never_echoed() {
    let app = router(test_state());
    let secret = "jean.dupont@example.com";
    let response = call(
        &app,
        "POST",
        "/v1/team/viewer/capability",
        TENANT_A,
        Some(&format!(
            r#"{{"document_ref":"doc1","email_ref":"msg1","token_ref":"{secret}"}}"#
        )),
    )
    .await;
    assert_eq!(response.status().as_u16(), 400);
    let text = response_text(response).await;
    assert!(!text.contains(secret), "error echoed raw PII: {text}");
}

#[tokio::test]
async fn ingest_dto_rejects_paths_credentials_and_raw_message_text_over_http() {
    let app = router(test_state());
    let base = r#"{"source_ref":"default","mailbox_ref":"inbox","operation":{"operation":"index_for_rag"}}"#;
    let carriers = [
        ("backend", r#""backend":{"type":"maildir","path":"/etc"}"#),
        ("path", r#""path":"/etc""#),
        ("token", r#""token":"oauth-secret""#),
        ("password", r#""password":"hunter2""#),
        ("server", r#""server":"evil.example""#),
        ("token_command", r#""token_command":["sh"]"#),
        ("text", r#""text":"jean.dupont@example.com""#),
        ("message", r#""message":"raw message body""#),
    ];
    for (field, carrier) in carriers {
        let body = format!("{}, {carrier}}}", &base[..base.len() - 1]);
        let response = call(&app, "POST", "/v1/team/email/ingest", TENANT_A, Some(&body)).await;
        assert_eq!(
            response.status().as_u16(),
            400,
            "insecure field `{field}` was accepted"
        );
        let text = response_text(response).await;
        for secret in ["/etc", "oauth-secret", "hunter2", "jean.dupont@example.com"] {
            assert!(
                !text.contains(secret),
                "error for `{field}` leaked `{secret}`"
            );
        }
    }
}

/// The Team ingest surface shares `EmailOperationResultDto` with the sync and
/// async core routes. Sentinel diagnostics (envelope id, IO text, filename, PII)
/// must be projected to a count only, so a renderer polling an ingest response
/// or a job can never see them.
#[tokio::test]
async fn team_ingest_errors_are_a_content_free_count() {
    use hacienda_email::models::EmailResponse;

    let sentinels = [
        "SENTINEL_ENV_550e8400-e29b-41d4-a716-446655440000",
        "io error: No such file or directory (os error 2)",
        "/srv/mail/tenant/jean-dupont.mbox",
        "jean.dupont@example.com",
    ];
    let domain = EmailResponse {
        success: false,
        processed_count: 0,
        output_path: None,
        entities: None,
        rag_stats: None,
        redaction_stats: None,
        errors: sentinels.iter().map(|value| (*value).to_string()).collect(),
    };
    let wire = serde_json::to_value(hacienda_api::dto::EmailOperationResultDto::from_response(
        &domain, 4,
    ))
    .unwrap();
    let text = wire.to_string();
    for sentinel in sentinels {
        assert!(
            !text.contains(sentinel),
            "Team ingest response leaked `{sentinel}`: {text}"
        );
    }
    assert!(
        wire.get("errors").is_none(),
        "raw error array must not exist"
    );
    assert_eq!(wire["error_count"].as_u64(), Some(sentinels.len() as u64));
}

/// A failing Team ingest over HTTP must not echo the rejected path or any
/// backend/IO diagnostic back to the renderer.
#[tokio::test]
async fn team_ingest_failure_response_is_content_free() {
    let app = router(test_state());
    let secret_path = "jean.dupont@example.com.mbox";
    let body = format!(
        r#"{{"source_ref":"default","mailbox_ref":"inbox","operation":{{"operation":"extract_ner","source":"{secret_path}","output":"out"}}}}"#
    );
    // The Team DTO has no `source`/`output` fields, so this is a 400 with a
    // generic envelope — the path string must not be reflected anywhere.
    let response = call(&app, "POST", "/v1/team/email/ingest", TENANT_A, Some(&body)).await;
    assert_eq!(response.status().as_u16(), 400);
    let text = response_text(response).await;
    assert!(!text.contains(secret_path), "error echoed the path: {text}");
    assert!(
        !text.contains("os error"),
        "error echoed an IO diagnostic: {text}"
    );
}

/// The viewer `open` consumes a short-lived capability and returns a typed,
/// content-safe projection — not an `opened:true` boolean. Replay, expiry and
/// cross-tenant use are rejected and audited; no clear email crosses the wire.
#[tokio::test]
async fn viewer_open_returns_a_content_safe_projection_and_rejects_replay() {
    use serde_json::json;

    let (state, store) = test_state_with_store();
    let app = router(state);
    let collection = "acme:email-corpus";
    store
        .ensure_collection(&CollectionSpec {
            name: collection.to_string(),
            embedding_dim: 0,
            distance_metric: DistanceMetric::Cosine,
            index_method: IndexMethod::Flat,
            created_at: None,
            embedding_source: None,
            embedding_version: 1,
        })
        .await
        .unwrap();

    let external_id = "email:acme:INBOX:msg-1";
    let raw = "jean.dupont@example.com";
    let document = DocumentRecord {
        external_id: Some(external_id.to_string()),
        title: Some("[EMAIL:k1:AAAA]".to_string()),
        mime: Some("message/rfc822".to_string()),
        source_uri: None,
        full_text: format!("Bonjour [PERSON:k1:BBBB] from {raw}"),
        keywords: Vec::new(),
        entities: serde_json::Value::Null,
        labels: serde_json::Value::Null,
        // Deliberately poisoned with clear PII to prove the projection filters it.
        metadata: json!({
            "subject": format!("Re: {raw} [EMAIL:k1:AAAA]"),
            "from": raw,
            "privacy": "redacted",
            "attachments": ["invoice-jean-dupont.pdf"],
        }),
        dossier_id: None,
        version_ref: None,
        tenant_id: Some("acme".to_string()),
        acl: None,
    };
    let chunks = vec![ChunkRecord {
        external_id: None,
        ordinal: 0,
        content: "Bonjour [PERSON:k1:BBBB]".to_string(),
        embedding: Vec::new(),
        sparse_embedding: None,
        multi_vector: None,
        chunk_metadata: serde_json::Value::Null,
        dossier_id: None,
        version_ref: None,
        tenant_id: Some("acme".to_string()),
        acl: None,
    }];
    let document_id = store
        .upsert_document(collection, &document, &chunks)
        .await
        .unwrap();
    let document_ref = opaque_ref(&document_id.0);
    let email_ref = opaque_ref(external_id);

    let issued = response_json(
        call(
            &app,
            "POST",
            "/v1/team/viewer/capability",
            TENANT_A,
            Some(&format!(
                r#"{{"document_ref":"{document_ref}","email_ref":"{email_ref}","token_ref":"[EMAIL:k1:AAAA]"}}"#
            )),
        )
        .await,
    )
    .await;
    let capability_ref = issued["capability_ref"].as_str().unwrap().to_string();

    let opened = call(
        &app,
        "POST",
        "/v1/team/viewer/open",
        TENANT_A,
        Some(&format!(r#"{{"capability_ref":"{capability_ref}"}}"#)),
    )
    .await;
    assert_eq!(opened.status().as_u16(), 200);
    let projection = response_json(opened).await;
    assert_eq!(projection["projection_kind"], "redacted_token_only");
    assert_eq!(projection["document_ref"], document_ref);
    assert_eq!(projection["email_ref"], email_ref);
    assert!(
        projection.get("opened").is_none(),
        "opened boolean must be gone"
    );
    let projection_text = projection.to_string();
    for secret in [raw, "invoice-jean-dupont.pdf", "Re:", "acme:email-corpus"] {
        assert!(
            !projection_text.contains(secret),
            "projection leaked `{secret}`: {projection_text}"
        );
    }
    assert!(projection_text.contains("[EMAIL:k1:AAAA]"));
    assert!(projection_text.contains("[REDACTED]"));
    let headers = projection["headers"].as_array().unwrap();
    assert!(headers.iter().any(|header| header["name"] == "subject"));
    assert!(headers.iter().any(|header| header["name"] == "from"));
    assert!(!projection["attachment_refs"].as_array().unwrap().is_empty());

    // Replaying the consumed capability is rejected and audited.
    let replay = call(
        &app,
        "POST",
        "/v1/team/viewer/open",
        TENANT_A,
        Some(&format!(r#"{{"capability_ref":"{capability_ref}"}}"#)),
    )
    .await;
    assert_eq!(replay.status().as_u16(), 404);
    let acme_audit =
        response_json(call(&app, "GET", "/v1/team/viewer/audit", TENANT_A, None).await).await;
    let outcomes: Vec<&str> = acme_audit["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["outcome"].as_str().unwrap())
        .collect();
    assert!(outcomes.contains(&"issued"));
    assert!(outcomes.contains(&"consumed_projection"));
    assert!(outcomes.contains(&"replayed"));

    // A capability minted by tenant B for tenant A's refs cannot read A's
    // document: B's own email corpus is empty and the lookup fails closed.
    let issued_b = response_json(
        call(
            &app,
            "POST",
            "/v1/team/viewer/capability",
            TENANT_B,
            Some(&format!(
                r#"{{"document_ref":"{document_ref}","email_ref":"{email_ref}","token_ref":"[EMAIL:k1:AAAA]"}}"#
            )),
        )
        .await,
    )
    .await;
    let capability_b = issued_b["capability_ref"].as_str().unwrap().to_string();
    let cross = call(
        &app,
        "POST",
        "/v1/team/viewer/open",
        TENANT_B,
        Some(&format!(r#"{{"capability_ref":"{capability_b}"}}"#)),
    )
    .await;
    assert_eq!(cross.status().as_u16(), 404);
    let cross_text = response_text(cross).await;
    assert!(
        !cross_text.contains(raw),
        "cross-tenant response leaked PII"
    );
}

/// Team ingestion rejects pseudonymization: there is no safe multi-tenant
/// capability/reveal contract for deterministic tenant-keyed tokens. The
/// low-level email route and LocalFull remain unchanged.
#[tokio::test]
async fn team_ingest_rejects_pseudonymize_on_sync_and_async_routes() {
    let app = router(test_state());
    let body = r#"{"source_ref":"default","mailbox_ref":"inbox","operation":{"operation":"index_for_rag"},"privacy_mode":"pseudonymize"}"#;
    for path in ["/v1/team/email/ingest", "/v1/team/email/ingest/async"] {
        let response = call(&app, "POST", path, TENANT_A, Some(body)).await;
        assert_eq!(
            response.status().as_u16(),
            400,
            "{path} accepted pseudonymize"
        );
        let text = response_text(response).await;
        assert!(
            !text.contains("pseudonym"),
            "error echoed a diagnostic: {text}"
        );
    }
}

/// The deterministic mint is an offline oracle: a documents:process caller
/// (LIMITED) must get 403, while a pii:reveal caller can still mint the stable,
/// tenant-keyed token used by authorized GDPR access and LocalFull.
#[tokio::test]
async fn deterministic_token_oracle_requires_pii_reveal() {
    let app = router(test_state());
    let body = r#"{"category":"email","value":"bob@example.com"}"#;

    let denied = call(&app, "POST", "/v1/pii/token", LIMITED, Some(body)).await;
    assert_eq!(denied.status().as_u16(), 403);
    let denied_text = response_text(denied).await;
    assert!(
        !denied_text.contains("bob@example.com"),
        "denial echoed the candidate: {denied_text}"
    );

    let first =
        response_json(call(&app, "POST", "/v1/pii/token", TENANT_A, Some(body)).await).await;
    let second =
        response_json(call(&app, "POST", "/v1/pii/token", TENANT_A, Some(body)).await).await;
    assert_eq!(
        first["token"], second["token"],
        "mint must stay deterministic"
    );
    assert!(first["token"].as_str().unwrap().starts_with("[EMAIL:"));
}

#[tokio::test]
async fn search_never_accepts_a_path_as_a_source_ref() {
    let app = router(test_state());
    let response = call(
        &app,
        "POST",
        "/v1/team/email/search",
        TENANT_A,
        Some(r#"{"source_ref":"/srv/mail","mailbox_ref":"inbox","query_text":"review"}"#),
    )
    .await;
    assert_eq!(response.status().as_u16(), 400);
    let text = response_text(response).await;
    assert!(!text.contains("/srv/mail"), "error echoed a path: {text}");
}
