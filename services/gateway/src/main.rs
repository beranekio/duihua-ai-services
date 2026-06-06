use std::env;

use anyhow::Result;

use duihua_gateway::{init_rustls_provider, run};

#[tokio::main]
async fn main() -> Result<()> {
    init_rustls_provider();

    let env_filter =
        env::var("RUST_LOG").unwrap_or_else(|_| "info,duihua_gateway=debug".to_string());
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    run().await
}
