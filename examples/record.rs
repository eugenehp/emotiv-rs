//! Record and export data — mirrors `cortex-example/python/record.py`.
//!
//! Creates a recording, waits for a duration, stops it, and exports to CSV.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example record
//! ```
//!
//! # API credentials
//!
//! Requires `EMOTIV_CLIENT_ID` and `EMOTIV_CLIENT_SECRET` environment variables.
//! Create them at <https://www.emotiv.com/my-account/cortex-apps/>.

use anyhow::Result;

use emotiv::client::{CortexClient, CortexClientConfig};
use emotiv::types::*;

const RECORD_DURATION_SECS: u64 = 10;
const RECORD_TITLE: &str = "Rust Record Example";
const RECORD_DESCRIPTION: &str = "Recorded via emotiv-rs record example";
const EXPORT_FOLDER: &str = "/tmp/emotiv_export";
const EXPORT_FORMAT: &str = "CSV";
const EXPORT_VERSION: &str = "V2";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let client_id = std::env::var("EMOTIV_CLIENT_ID").expect("Set EMOTIV_CLIENT_ID");
    let client_secret = std::env::var("EMOTIV_CLIENT_SECRET").expect("Set EMOTIV_CLIENT_SECRET");

    let config = CortexClientConfig { client_id, client_secret, ..Default::default() };
    let client = CortexClient::new(config);
    let (mut rx, handle) = client.connect().await?;

    while let Some(event) = rx.recv().await {
        match event {
            CortexEvent::SessionCreated(_) => {
                println!("Session created. Creating record...");
                handle.create_record(RECORD_TITLE, RECORD_DESCRIPTION).await?;
            }
            CortexEvent::RecordCreated(rec) => {
                println!("Record created: {} ({})", rec.title, rec.uuid);
                println!("Recording for {RECORD_DURATION_SECS} seconds...");
                for i in 0..RECORD_DURATION_SECS {
                    println!("  recording at {i} s");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                println!("Stopping record...");
                handle.stop_record().await?;
            }
            CortexEvent::RecordStopped(rec) => {
                println!("Record stopped: {} ({} -> {})", rec.uuid, rec.start_datetime, rec.end_datetime);
            }
            CortexEvent::RecordPostProcessingDone(rid) => {
                println!("Record {rid} post-processing done. Exporting...");
                let rid_str: &str = &rid;
                handle.export_record(
                    EXPORT_FOLDER, EXPORT_FORMAT,
                    &["EEG", "MOTION", "PM", "BP"],
                    &[rid_str], EXPORT_VERSION,
                ).await?;
            }
            CortexEvent::RecordExported(ids) => {
                println!("Export complete for records: {ids:?}");
                break;
            }
            CortexEvent::Error(msg) => eprintln!("Error: {msg}"),
            CortexEvent::Disconnected => { println!("Disconnected"); break; }
            _ => {}
        }
    }

    Ok(())
}
