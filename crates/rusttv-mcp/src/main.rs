//! `rusttv-mcp`: the read-only RustTV MCP server on stdin/stdout. See MCP.md.

use std::error::Error;
use std::process::ExitCode;

use rmcp::ServiceExt;
use rmcp::transport::stdio;
use rusttv_core::SystemAdb;
use rusttv_mcp::RustTvServer;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // RUSTTV_ADB points to a specific adb executable; otherwise `adb` from PATH.
    let adb = std::env::var_os("RUSTTV_ADB").map_or_else(SystemAdb::new, SystemAdb::with_program);
    match serve(adb).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            print_error(&error.to_string());
            ExitCode::FAILURE
        }
    }
}

async fn serve(adb: SystemAdb) -> Result<(), Box<dyn Error>> {
    let service = RustTvServer::new(adb).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[expect(
    clippy::print_stderr,
    reason = "stdout is the MCP transport; startup errors can only go to stderr"
)]
fn print_error(message: &str) {
    eprintln!("rusttv-mcp: {message}");
}
