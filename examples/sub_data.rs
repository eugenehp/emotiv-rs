//! Subscribe to data streams — mirrors `cortex-example/python/sub_data.py`.
//!
//! Subscribes to EEG, motion, device info, performance metrics, and band power
//! streams, printing each data packet to stdout.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example sub_data
//! ```
//!
//! # API credentials
//!
//! Requires `EMOTIV_CLIENT_ID` and `EMOTIV_CLIENT_SECRET` environment variables.
//! Create them at <https://www.emotiv.com/my-account/cortex-apps/>.
//!
//! ```bash
//! export EMOTIV_CLIENT_ID="your_client_id"
//! export EMOTIV_CLIENT_SECRET="your_client_secret"
//! ```

use anyhow::Result;

use emotiv::client::{CortexClient, CortexClientConfig};
use emotiv::types::*;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let streams = &["eeg", "mot", "met", "pow"];

    {
        let client_id = std::env::var("EMOTIV_CLIENT_ID")
            .expect("Set EMOTIV_CLIENT_ID environment variable");
        let client_secret = std::env::var("EMOTIV_CLIENT_SECRET")
            .expect("Set EMOTIV_CLIENT_SECRET environment variable");

        let config = CortexClientConfig {
            client_id,
            client_secret,
            ..Default::default()
        };

        let client = CortexClient::new(config);
        let (mut rx, handle) = client.connect().await?;

        while let Some(event) = rx.recv().await {
            match event {
                CortexEvent::SessionCreated(id) => {
                    println!("Session created: {id}");
                    handle.subscribe(streams).await?;
                }
                CortexEvent::DataLabels(labels) => {
                    println!("{} labels are: {:?}", labels.stream_name, labels.labels);
                }
                CortexEvent::Eeg(data) => {
                    println!("eeg data: {:?}", data);
                }
                CortexEvent::Motion(data) => {
                    println!("motion data: {:?}", data);
                }
                CortexEvent::Dev(data) => {
                    println!("dev data: {:?}", data);
                }
                CortexEvent::Metrics(data) => {
                    println!("pm data: {:?}", data);
                }
                CortexEvent::BandPower(data) => {
                    println!("pow data: {:?}", data);
                }
                CortexEvent::Disconnected => {
                    println!("Disconnected");
                    break;
                }
                CortexEvent::Error(msg) => {
                    eprintln!("Error: {msg}");
                }
                _ => {}
            }
        }
    }

    Ok(())
}
