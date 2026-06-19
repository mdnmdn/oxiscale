use anyhow::Result;
use control_api::ControlServerBuilder;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let listen = std::env::var("OXISCALE_LISTEN").unwrap_or_else(|_| "0.0.0.0:8080".to_owned());

    let server = ControlServerBuilder::new()
        .memory_store()
        .listen(listen)
        .build()?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        listen = server.listen_addr(),
        server_key = %server.server_public_key(),
        "starting tailcontrold"
    );

    server.serve().await?;
    Ok(())
}
