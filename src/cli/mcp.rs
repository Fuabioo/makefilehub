//! MCP server launcher
//!
//! Starts the MCP server over stdio for Claude Code integration.

use anyhow::{Context, Result};
use rmcp::ServiceExt;
use tokio::io::{stdin, stdout};

use crate::config::load_config;
use crate::mcp::MakefilehubServer;

/// Run the MCP server over stdio.
///
/// This function starts the MCP server using stdin/stdout for communication,
/// which is the standard transport for Claude Code MCP servers.
///
/// # Arguments
/// * `config_path` - Optional path to a config file override
///
/// # Returns
/// * `Ok(())` - Server ran successfully and was shut down
/// * `Err(e)` - Server failed to start or encountered an error
pub async fn run_mcp_server(config_path: Option<&str>) -> Result<()> {
    // Load configuration
    let config = load_config(config_path).context("Failed to load configuration")?;

    // Create server with loaded config
    let server = MakefilehubServer::with_config(config);

    // Create stdio transport - tuple of (reader, writer)
    let transport = (stdin(), stdout());

    // Start serving with the transport
    let service = server.serve(transport).await?;

    // Wait for completion
    service.waiting().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    // Note: actual MCP server testing requires integration tests
    // with a mock stdio transport

    #[test]
    fn test_module_compiles() {
        // Just verify the module compiles correctly
    }
}
