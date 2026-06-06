use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    duihua_common::init_rustls_provider();
    duihua_common::run_background_worker().await
}
