mod api;
mod auth;
mod config;
mod knowledge;
mod render;
mod roots;
mod server;
mod test_support;
mod tools;
mod workspace;

use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .init();
    let cfg = config::Config::from_env();
    match std::env::args().nth(1).as_deref() {
        None | Some("serve") => {
            let workspace = workspace::Workspace::from_config(&cfg)?;
            tracing::info!(
                "recipe root: {} ({:?})",
                workspace.root(),
                workspace.source()
            );
            let service = server::CookMcp::new(cfg, workspace).serve(stdio()).await?;
            service.waiting().await?;
        }
        Some("login") => {
            let mgr = auth::AuthManager::new(&cfg);
            let flow = auth::device::DeviceFlow::new(cfg.cookmd_url.clone());
            let start = flow.start().await?;
            eprintln!(
                "Open {} and enter code: {}",
                start.verification_uri, start.user_code
            );
            if let Some(complete) = &start.verification_uri_complete {
                eprintln!("(or open {complete} directly)");
            }
            let token = flow
                .poll(
                    &start.device_code,
                    std::time::Duration::from_secs(start.interval.max(1)),
                    std::time::Duration::from_secs(start.expires_in),
                )
                .await?;
            let stored = mgr.store_token(&token)?;
            eprintln!(
                "Logged in as {}. Token stored at {}",
                stored.email.as_deref().unwrap_or("<unknown>"),
                mgr.path.display()
            );
        }
        Some("logout") => {
            let mgr = auth::AuthManager::new(&cfg);
            mgr.logout()?;
            eprintln!("Logged out.");
        }
        Some(other) => {
            eprintln!("usage: cook-mcp [serve|login|logout]  (unknown: {other})");
            std::process::exit(2);
        }
    }
    Ok(())
}
