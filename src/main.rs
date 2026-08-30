//! Stage 0 probe: prove the rmcp 3.1.4 wiring compiles before building on it.

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, ContentBlock, Implementation, ProtocolVersion, ServerCapabilities,
        ServerInfo,
    },
    schemars, tool, tool_handler, tool_router, transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PinoutArgs {
    /// Optional DIP-40 pin number to look up (1-40).
    pub pin: Option<u8>,
}

#[derive(Clone)]
pub struct Server {
    tool_router: ToolRouter<Server>,
}

#[tool_router]
impl Server {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "DIP-40 pin reference for the 8051.")]
    async fn pinout(
        &self,
        Parameters(PinoutArgs { pin }): Parameters<PinoutArgs>,
    ) -> Result<CallToolResult, McpError> {
        let body = match pin {
            Some(p) => format!("pin {p}"),
            None => "all pins".to_string(),
        };
        let mut result = CallToolResult::success(vec![ContentBlock::text(body)]);
        result.structured_content = Some(serde_json::json!({ "ok": true }));
        Ok(result)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("mcs51-mcp", env!("CARGO_PKG_VERSION")))
            .with_protocol_version(ProtocolVersion::V_2025_11_25)
            .with_instructions("8051 development loop.".to_string())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let service = Server::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
