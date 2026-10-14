#[tokio::main]
async fn main() -> anyhow::Result<()> {
    codol_server::app::run().await
}
