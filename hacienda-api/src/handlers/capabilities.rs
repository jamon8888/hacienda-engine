//! Safe build and runtime capability discovery.

use std::{sync::Arc, time::Duration};

use axum::{extract::State, Json};

use crate::{
    dto::CapabilitiesResponse,
    state::{ApiState, ProbeState},
};

/// `GET /capabilities` — report safe feature availability without exposing document
/// content, configuration secrets, model paths, or filesystem details.
#[utoipa::path(
    get,
    path = "/capabilities",
    tag = "info",
    operation_id = "getCapabilities",
    responses((status = 200, description = "Build and runtime capability state", body = CapabilitiesResponse))
)]
pub async fn capabilities(State(state): State<ApiState>) -> Json<CapabilitiesResponse> {
    let onnx_runtime = probe_onnx_runtime_status(&state).await;
    let report = hacienda_core::capabilities::report(
        env!("CARGO_PKG_VERSION"),
        cfg!(feature = "rag-embeddings"),
        onnx_runtime,
        state.rag_store.is_some(),
    );
    Json(report.into())
}

async fn probe_onnx_runtime_status(
    state: &ApiState,
) -> hacienda_core::capabilities::CapabilityStatus {
    let probe_state = Arc::clone(&state.probe_state);

    {
        let guard = probe_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &*guard {
            ProbeState::Ready(status) => return status.clone(),
            ProbeState::Running => {
                return unavailable_status("runtime preflight is in progress or timed out");
            }
            ProbeState::Idle => {}
        }
    }

    {
        let mut guard = probe_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !matches!(*guard, ProbeState::Idle) {
            return match &*guard {
                ProbeState::Ready(status) => status.clone(),
                ProbeState::Running => {
                    unavailable_status("runtime preflight is in progress or timed out")
                }
                ProbeState::Idle => unavailable_status("runtime preflight state changed"),
            };
        }
        *guard = ProbeState::Running;
    }

    let task_state = Arc::clone(&probe_state);
    let task = tokio::task::spawn_blocking(move || {
        let status = hacienda_core::capabilities::onnx_runtime_status();
        let mut guard = task_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = ProbeState::Ready(status.clone());
        status
    });
    let result = tokio::time::timeout(Duration::from_secs(5), task).await;

    match result {
        Ok(Ok(status)) => status,
        Ok(Err(_)) => {
            let mut guard = probe_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *guard = ProbeState::Idle;
            unavailable_status("runtime preflight could not start")
        }
        Err(_) => {
            let mut guard = probe_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if matches!(*guard, ProbeState::Running) {
                *guard = ProbeState::Idle;
            }
            unavailable_status("runtime preflight timed out")
        }
    }
}

fn unavailable_status(reason: &'static str) -> hacienda_core::capabilities::CapabilityStatus {
    hacienda_core::capabilities::CapabilityStatus {
        name: "onnx_runtime",
        compiled: cfg!(feature = "rag-embeddings"),
        runtime_ready: false,
        reason: Some(reason),
    }
}
