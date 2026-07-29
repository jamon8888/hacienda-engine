//! Hand-built OpenAPI document derived from the route table.
//!
//! Generated from `routes::ROUTE_TABLE` so that the path set in the document is
//! structurally tied to the set of registered routes. The test
//! `openapi_path_set_equals_route_table` enforces that they cannot drift.

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::{routes::ROUTE_TABLE, state::ApiState};

pub async fn openapi(_: State<ApiState>) -> Json<Value> {
    Json(build_openapi())
}

/// Build the OpenAPI document from `ROUTE_TABLE`.
///
/// Called at request time rather than once at startup so that the path set is always
/// derived from the same table the router used — no stale copy possible.
pub fn build_openapi() -> Value {
    let paths: serde_json::Map<String, Value> = ROUTE_TABLE
        .iter()
        .map(|spec| {
            let path = spec.path.to_string();
            // Strip axum path parameters like `{id}` to OpenAPI `{id}` — they happen
            // to use the same syntax, so nothing needs replacing.
            let path_item = json!({
                "description": format!("Access: {:?}", spec.access)
            });
            (path, path_item)
        })
        .collect();

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "hacienda API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "PII redaction and compliance API. Document content never leaves the host."
        },
        "paths": paths,
        "components": {
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer"
                }
            }
        },
        "security": [{"bearerAuth": []}]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::ROUTE_TABLE;

    /// The path set in the OpenAPI document must equal the path set in the route table.
    /// These two are derived from the same source, so this test is a regression guard
    /// against someone adding a path to the OpenAPI document by hand.
    #[test]
    fn openapi_path_set_equals_route_table() {
        let doc = build_openapi();
        let doc_paths: std::collections::HashSet<String> = doc["paths"]
            .as_object()
            .expect("paths must be an object")
            .keys()
            .cloned()
            .collect();

        let table_paths: std::collections::HashSet<String> =
            ROUTE_TABLE.iter().map(|s| s.path.to_string()).collect();

        assert_eq!(
            doc_paths, table_paths,
            "OpenAPI path set diverged from ROUTE_TABLE — update is not needed; \
             build_openapi() already derives the paths from the table"
        );
    }
}
