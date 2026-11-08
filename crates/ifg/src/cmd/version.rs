//! `ifg version` and the server-version probe (design doc D10).

use crate::cli::Cli;
use crate::cmd::bootstrap;
use crate::error::AppError;

pub async fn run(cli: &Cli) -> Result<(), AppError> {
    println!("ifg {}", env!("CARGO_PKG_VERSION"));

    // Version must work without any configuration: resolve the host
    // best-effort and report a missing one instead of failing.
    let ctx = match bootstrap(cli) {
        Ok(ctx) => ctx,
        Err(error) => {
            println!("server: not reachable ({error})");
            return Ok(());
        }
    };
    match ctx.client.health().await {
        Ok(health) => {
            let version = health["version"].as_str().unwrap_or("unknown");
            let mut line = format!("server: {version}");
            if let Some(phase) = health["phase"].as_u64() {
                line.push_str(&format!(" (phase {phase})"));
            }
            println!("{line}");
            println!("host:   {}", ctx.host.url);
            if let Ok(capabilities) = ctx.client.capabilities().await {
                if let Some(commit) = capabilities["build_commit"].as_str() {
                    let dirty = match capabilities["build_dirty"].as_bool() {
                        Some(true) => " (modified source)",
                        Some(false) => "",
                        None => " (source state unknown)",
                    };
                    println!("build:  {commit}{dirty}");
                } else {
                    println!("build:  unknown");
                }
                if let Some(features) = capabilities["api_capabilities"].as_array() {
                    let features: Vec<_> = features.iter().filter_map(|v| v.as_str()).collect();
                    println!("api capabilities: {}", features.join(", "));
                } else {
                    println!("api capabilities: unknown");
                }
            } else {
                println!("build:  unknown (capability discovery unavailable)");
                println!("api capabilities: unknown");
            }
            if let Some(message) = crate::version::outdated(version) {
                eprintln!("ifg: warning: {message}");
            }
        }
        Err(error) => println!("server: not reachable ({error})"),
    }
    Ok(())
}

/// One cell for `auth status`: the server version, or `?` when unreachable.
pub(crate) async fn server_label(client: &ifg_client::Client) -> String {
    match client.health().await {
        Ok(health) => health["version"].as_str().unwrap_or("?").to_string(),
        Err(_) => "?".to_string(),
    }
}
