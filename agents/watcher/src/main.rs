//! The watcher observes the home and replicas for double update fraud.
//!
//! At a regular interval, the watcher polls Home and Replicas for signed
//! updates and checks them against its local DB of updates for fraud. It
//! checks for double updates on both the Home and Replicas and fraudulent
//! updates on just the Replicas by verifying Replica updates on the Home.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(unused_extern_crates)]

mod settings;
mod watcher;

use crate::{settings::WatcherSettings as Settings, watcher::Watcher};
use color_eyre::Result;
use nomad_base::NomadAgent;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let settings = Settings::new()?;

    let agent = Watcher::from_settings(settings).await?;

    agent.start_tracing(agent.metrics().span_duration())?;
    let _ = agent.metrics().run_http_server();

    agent.run_all().await??;
    Ok(())
}
