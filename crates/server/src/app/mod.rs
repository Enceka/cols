pub mod config;
pub(crate) mod hosting;

mod cli;
mod readiness;

/// Run the command selected by the command line.
pub async fn run() -> anyhow::Result<()> {
    cli::run().await
}
pub mod services;
