//! Safety invariant tests: assertions about what this crate must NOT do.

/// `ExtractInput::from_uri` must never appear in any source file in this crate.
///
/// `from_uri` accepts filesystem paths, `file://` URIs, and HTTP(S) URLs. Accepting
/// one from the wire hands any authenticated caller arbitrary local file reads (path
/// traversal) and SSRF against the host network — on a product whose core claim is that
/// document content never leaves the host.
///
/// This test fails the build if the forbidden constructor appears in the crate source,
/// so a future contributor cannot accidentally introduce the vulnerability.
#[test]
fn no_from_uri_in_crate_source() {
    let crate_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    let violations: Vec<String> = walk_rust_files(&crate_src)
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            // Filter out comment lines and doc strings so that warnings about
            // from_uri in comments don't self-trigger the check.
            let has_call = content
                .lines()
                .filter(|line| {
                    let trimmed = line.trim_start();
                    !trimmed.starts_with("//") && !trimmed.starts_with('*')
                })
                .any(|line| line.contains("from_uri(") || line.contains("::from_uri"));
            if has_call {
                Some(path.display().to_string())
            } else {
                None
            }
        })
        .collect();

    assert!(
        violations.is_empty(),
        "ExtractInput::from_uri found in crate source — this is a SSRF / path-traversal \
         vulnerability. Use ExtractInput::from_bytes instead.\n\nFiles:\n{}",
        violations.join("\n")
    );
}

/// A JSON body carrying a `uri`, `path`, or `file` key — the names an attacker probes
/// for when looking for a passthrough to the host filesystem — must be **rejected**,
/// not silently ignored.
///
/// A 200 answer to `{"uri":"file:///etc/passwd","documents":[]}` reads to the prober as
/// "accepted, keep going". Rejecting the field states that the surface does not exist.
/// The same `deny_unknown_fields` also turns a legitimate client's misspelled field into
/// an error rather than a setting that silently did nothing.
///
/// The earlier version of this test asserted only `status != 500`, which every one of
/// those outcomes satisfies.
#[tokio::test]
async fn body_with_uri_field_is_rejected_not_silently_ignored() {
    let probes = [
        r#"{"uri":"file:///etc/passwd","documents":[]}"#,
        r#"{"path":"/etc/shadow","documents":[]}"#,
        r#"{"file":"../../etc/passwd","documents":[]}"#,
        r#"{"documents":[{"mime_type":"text/plain","content_base64":"","uri":"file:///etc/passwd"}]}"#,
    ];

    for probe in probes {
        let status = post_documents(probe).await;
        assert_eq!(
            status, 400,
            "a body naming a host resource must be refused, got {status} for {probe}"
        );
    }
}

/// The rejection above must be caused by the unknown field, not by the body being
/// malformed in some other way — otherwise the assertion would hold for a server that
/// rejects every request.
#[tokio::test]
async fn a_well_formed_body_without_probe_fields_is_accepted() {
    let status = post_documents(r#"{"documents":[]}"#).await;
    assert_eq!(
        status, 200,
        "a valid request must be accepted, or the rejection test above proves nothing"
    );
}

/// POST a raw body to `/v1/documents` with auth disabled; return the status code.
async fn post_documents(body: &str) -> u16 {
    use axum::{body::Body, http::Request};
    use hacienda_api::{router, ApiLimits, ApiState};
    use hacienda_core::{
        auth::{authn::DevTokenResolver, AuthState},
        jobs::InMemoryJobStore,
        HaciendaConfig, HaciendaFacade,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    let facade = Arc::new(HaciendaFacade::new(HaciendaConfig::default()).unwrap());
    let jobs = InMemoryJobStore::new().into_arc();
    let auth = AuthState::new(Arc::new(DevTokenResolver)).with_enabled(false);
    let state = ApiState::new(facade, jobs, auth, ApiLimits::default());

    router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/documents")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
        .as_u16()
}

fn walk_rust_files(dir: &std::path::Path) -> impl Iterator<Item = std::path::PathBuf> {
    walkdir(dir)
}

fn walkdir(dir: &std::path::Path) -> Box<dyn Iterator<Item = std::path::PathBuf>> {
    let mut entries = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                entries.extend(walkdir(&path));
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                entries.push(path);
            }
        }
    }
    Box::new(entries.into_iter())
}
