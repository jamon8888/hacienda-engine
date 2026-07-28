use axum::{routing::get, routing::post, Router};
use hacienda_core::{HaciendaFacade, HaciendaFacadeConfig};
use std::sync::Arc;

#[derive(Clone)]
pub struct HaciendaApiState {
    pub facade: Arc<HaciendaFacade>,
}

pub fn create_app(state: HaciendaApiState) -> Router {
    Router::new()
        // Mount ALL xberg routes under /v1
        .nest("/v1", xberg::api::create_app(state.xberg_state()))
        // Add hacienda PII routes
        .route("/v1/pii/scan", post(pii_scan))
        .route("/v1/pii/redact", post(pii_redact))
        .route("/v1/pii/explain", post(pii_explain))
        .route("/v1/pii/batch", post(pii_batch))
        // Compliance
        .route("/v1/compliance/report", get(compliance_report))
        .route("/v1/compliance/model-card", get(model_card))
        .route("/v1/compliance/dora", post(dora_report))
        .route("/v1/compliance/checklist", get(checklist))
        // Review queue
        .route("/v1/review", post(create_review))
        .route("/v1/review", get(list_reviews))
        .route("/v1/review/:id", get(get_review))
        .route("/v1/review/:id/decision", post(submit_decision))
        .route("/v1/review/:id/assign", post(assign_reviewer))
        // Audit
        .route("/v1/audit/logs", get(query_audit))
        .route("/v1/audit/export", get(export_audit))
        .route("/v1/audit/stats", get(audit_stats))
        .with_state(state)
}

// PII handlers
async fn pii_scan(
    axum::extract::State(state): axum::extract::State<HaciendaApiState>,
    axum::Json(req): axum::Json<ScanRequest>,
) -> axum::Json<ScanResponse> {
    let result = state.facade.process(req.input.into()).await.unwrap();
    axum::Json(ScanResponse {
        entities: result.pii.map(|p| p.entities).unwrap_or_default(),
        redacted: result.pii.map(|p| p.redacted_text),
    })
}

async fn pii_redact(
    axum::extract::State(state): axum::extract::State<HaciendaApiState>,
    axum::Json(req): axum::Json<RedactRequest>,
) -> axum::Json<RedactResponse> {
    let result = state.facade.process(req.input.into()).await.unwrap();
    axum::Json(RedactResponse {
        redacted_text: result.pii.map(|p| p.redacted_text).unwrap_or(req.input),
        entities: result.pii.map(|p| p.entities).unwrap_or_default(),
    })
}

async fn pii_explain(
    axum::extract::State(state): axum::extract::State<HaciendaApiState>,
    axum::Json(req): axum::Json<ExplainRequest>,
) -> axum::Json<ExplainResponse> {
    let result = state.facade.process(req.input.into()).await.unwrap();
    if let Some(pii) = result.pii {
        if let Some(entity) = pii.entities.get(req.index) {
            return axum::Json(ExplainResponse {
                entity: entity.clone(),
                detection_method: "regex+model".into(),
                confidence: entity.confidence,
                threshold_used: 0.5,
                matched_pattern: "N/A".into(),
                context_window: "".into(),
                similar_examples: vec![],
                model_reasoning: None,
            });
        }
    }
    axum::Json(ExplainResponse::default())
}

async fn pii_batch(
    axum::extract::State(state): axum::extract::State<HaciendaApiState>,
    axum::Json(req): axum::Json<BatchRequest>,
) -> axum::Json<BatchResponse> {
    let results = state
        .facade
        .process_batch(req.texts.into_iter().map(Into::into).collect())
        .await
        .unwrap();
    axum::Json(BatchResponse {
        results: results
            .into_iter()
            .map(|r| ScanResponse {
                entities: r.pii.map(|p| p.entities).unwrap_or_default(),
                redacted: r.pii.map(|p| p.redacted_text),
            })
            .collect(),
    })
}

// Compliance handlers
async fn compliance_report() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

async fn model_card() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

async fn dora_report() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

async fn checklist() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

// Review handlers
async fn create_review() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

async fn list_reviews() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

async fn get_review() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

async fn submit_decision() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

async fn assign_reviewer() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

// Audit handlers
async fn query_audit() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

async fn export_audit() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

async fn audit_stats() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

// Request/Response types
#[derive(serde::Deserialize)]
struct ScanRequest {
    input: String,
}

#[derive(serde::Serialize)]
struct ScanResponse {
    entities: Vec<serde_json::Value>,
    redacted: Option<String>,
}

#[derive(serde::Deserialize)]
struct RedactRequest {
    input: String,
}

#[derive(serde::Serialize)]
struct RedactResponse {
    redacted_text: String,
    entities: Vec<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct ExplainRequest {
    input: String,
    index: usize,
}

#[derive(serde::Serialize)]
struct ExplainResponse {
    entity: serde_json::Value,
    detection_method: String,
    confidence: f32,
    threshold_used: f32,
    matched_pattern: String,
    context_window: String,
    similar_examples: Vec<String>,
    model_reasoning: Option<String>,
}

impl Default for ExplainResponse {
    fn default() -> Self {
        Self {
            entity: serde_json::Value::Null,
            detection_method: String::new(),
            confidence: 0.0,
            threshold_used: 0.0,
            matched_pattern: String::new(),
            context_window: String::new(),
            similar_examples: Vec::new(),
            model_reasoning: None,
        }
    }
}

#[derive(serde::Deserialize)]
struct BatchRequest {
    texts: Vec<String>,
}

#[derive(serde::Serialize)]
struct BatchResponse {
    results: Vec<ScanResponse>,
}
