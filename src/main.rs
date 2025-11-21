use mimalloc::MiMalloc;
use std::net::SocketAddr;
use tokio::{net::TcpListener, signal};
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let cfg = &gcli_nexus::config::CONFIG;

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(cfg.loglevel.clone()));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_level(true)
                .with_target(false),
        )
        .init();

    info!(
        database_url = %cfg.database_url,
        proxy = %cfg.proxy.as_ref().map(|u| u.as_str()).unwrap_or("<none>"),
        loglevel = %cfg.loglevel,
        nexus_key = %cfg.nexus_key,
        listen_addr = %cfg.listen_addr,
        listen_port = cfg.listen_port,
        client_id = %gcli_nexus::config::GCLI_CLIENT_ID,
        client_secret = %gcli_nexus::config::GCLI_CLIENT_SECRET
    );

    let _ = gcli_nexus::config::CONFIG.nexus_key.len();

    let handle = gcli_nexus::service::credentials_actor::spawn().await;
    let handle_clone = handle.clone();
    if let Some(cred_path) = cfg.cred_path.as_ref() {
        tokio::spawn(async move {
            info!(path = %cred_path.display(), "Background task: starting credential loading...");

            match gcli_nexus::service::credential_loader::load_from_dir(cred_path) {
                Ok(files) if !files.is_empty() => {
                    let count = files.len();
                    info!(
                        count,
                        "Background task: files loaded from filesystem, submitting to actor..."
                    );

                    handle_clone.submit_credentials(files).await;

                    info!(
                        count,
                        "Background task: all credentials successfully submitted."
                    );
                }
                Ok(_) => {
                    info!(
                        path = %cred_path.display(),
                        "Background task: no credential files discovered in directory."
                    );
                }
                Err(e) => {
                    warn!(
                        path = %cred_path.display(),
                        error = %e,
                        "Background task: failed to load credentials from directory."
                    );
                }
            }
        });
    }
    // Build axum router and serve
    let state = gcli_nexus::router::NexusState::new(handle.clone());
    let app = gcli_nexus::router::nexus_router(state);

    let addr = SocketAddr::from((cfg.listen_addr, cfg.listen_port));
    let listener = TcpListener::bind(addr).await?;
    info!("HTTP server listening on {}", addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    info!("Server has shut down gracefully.");
    Ok(())
}
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { /* ... */ },
        _ = terminate => { /* ... */ },
    }
}
