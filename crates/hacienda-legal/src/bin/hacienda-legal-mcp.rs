//! Standalone MCP server pinned to one official legal source.
//!
//! Configure `HACIENDA_LEGAL_SOURCE` to eurlex, legifrance, gesetze, rechtspraak,
//! boe, or normattiva. Provider credentials are read only from the server environment.

use hacienda_legal::{
    LegalError, LegalFetchRequest, LegalResearchClient, LegalSearchRequest, LegalSource,
};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::{async_rw::AsyncRwTransport, stdio},
    RoleServer, ServerHandler, ServiceExt,
};
use serde::Deserialize;
use std::{str::FromStr, sync::Arc};

const MAX_MCP_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchParams {
    /// Legal question or search terms. Only this query is sent to Parallel.
    query: String,
    /// Maximum number of results to return (1 to 20).
    max_results: Option<u8>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FetchParams {
    /// HTTPS URL from the pinned source's official domain.
    url: String,
}

#[derive(Clone)]
struct LegalSourceMcp {
    source: LegalSource,
    client: Arc<LegalResearchClient>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl LegalSourceMcp {
    fn new(source: LegalSource, client: LegalResearchClient) -> Self {
        Self {
            source,
            client: Arc::new(client),
            tool_router: Self::tool_router(),
        }
    }

    async fn serve_stdio(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (reader, writer) = stdio();
        let transport = AsyncRwTransport::<RoleServer, _, _>::new(reader, writer)
            .with_max_line_length(MAX_MCP_REQUEST_BYTES);
        self.serve(transport).await?.waiting().await?;
        Ok(())
    }

    #[tool(
        description = "Search this server's one configured official legal source. The query is the only caller-provided data sent to Parallel; fetched document text is never sent there.",
        annotations(
            title = "Search Official Legal Source",
            read_only_hint = true,
            idempotent_hint = true
        )
    )]
    async fn legal_search(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = self
            .client
            .search(LegalSearchRequest {
                query: params.query,
                source: self.source,
                max_results: params.max_results.unwrap_or(10),
            })
            .await
            .map_err(map_error)?;
        json_result(&response)
    }

    #[tool(
        description = "Fetch a bounded response from an HTTPS URL on this server's configured official legal source. Redirects to other hosts are refused.",
        annotations(
            title = "Fetch Official Legal Document",
            read_only_hint = true,
            idempotent_hint = true
        )
    )]
    async fn legal_fetch(
        &self,
        Parameters(params): Parameters<FetchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = self
            .client
            .fetch(LegalFetchRequest {
                source: self.source,
                url: params.url,
            })
            .await
            .map_err(map_error)?;
        json_result(&response)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LegalSourceMcp {
    fn get_info(&self) -> ServerConfig {
        let source = self.source;
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(
                    format!("hacienda-legal-{}", source.tool_prefix()),
                    env!("CARGO_PKG_VERSION"),
                )
                .with_title(format!("{} Legal Research MCP", source.host())),
            )
            .with_instructions(format!(
                "This server is pinned to {}. Search results are restricted to its official domain; fetch accepts only HTTPS URLs on that domain and caps response bodies. Configure the Parallel API key server-side.",
                source.host()
            ))
    }
}

fn json_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, rmcp::ErrorData> {
    let json = serde_json::to_value(value).map_err(|_| {
        rmcp::ErrorData::internal_error("failed to serialize legal source response", None)
    })?;
    let text = serde_json::to_string(&json).unwrap_or_default();
    let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
    result.structured_content = Some(json);
    Ok(result)
}

fn map_error(error: LegalError) -> rmcp::ErrorData {
    match error {
        LegalError::InvalidQuery
        | LegalError::InvalidFrequency
        | LegalError::InvalidProcessor
        | LegalError::InvalidCursor
        | LegalError::InvalidIdempotencyKey
        | LegalError::IdempotencyKeyConflict
        | LegalError::UnsafeUrl
        | LegalError::UnsupportedDocumentFormat
        | LegalError::UnsupportedSource => rmcp::ErrorData::invalid_params(error.to_string(), None),
        _ => rmcp::ErrorData::internal_error(error.to_string(), None),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let configured = std::env::var("HACIENDA_LEGAL_SOURCE")?;
    let source = LegalSource::from_str(&configured)?;
    let client = LegalResearchClient::from_env()?;
    LegalSourceMcp::new(source, client).serve_stdio().await
}
