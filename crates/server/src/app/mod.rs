pub mod config;

mod cli;

/// Run the command selected by the command line.
pub async fn run() -> anyhow::Result<()> {
    cli::run().await
}
pub mod services;
