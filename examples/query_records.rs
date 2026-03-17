//! Query and download records — mirrors `cortex-example/python/query_records.py`.
//!
//! Queries all records for the current user, lists their sync status, and
//! requests download for any that are not yet available locally.
//! Does **not** require a headset connection (`auto_create_session = false`).
//!
//! # Usage
//!
//! ```bash
//! cargo run --example query_records
//! ```
//!
//! # API credentials
//!
//! Requires `EMOTIV_CLIENT_ID` and `EMOTIV_CLIENT_SECRET` environment variables.
//! Create them at <https://www.emotiv.com/my-account/cortex-apps/>.

use anyhow::Result;
use serde_json::json;

use emotiv::client::{CortexClient, CortexClientConfig};
use emotiv::types::*;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let client_id = std::env::var("EMOTIV_CLIENT_ID").expect("Set EMOTIV_CLIENT_ID");
    let client_secret = std::env::var("EMOTIV_CLIENT_SECRET").expect("Set EMOTIV_CLIENT_SECRET");

    let config = CortexClientConfig {
        client_id, client_secret,
        auto_create_session: false,
        ..Default::default()
    };

    let client = CortexClient::new(config);
    let (mut rx, handle) = client.connect().await?;

    while let Some(event) = rx.recv().await {
        match event {
            CortexEvent::Authorized => {
                println!("Authorized. Querying records...");
                handle.query_records(json!({
                    "orderBy": [{ "startDatetime": "DESC" }],
                    "query": {},
                    "includeSyncStatusInfo": true
                })).await?;
            }
            CortexEvent::QueryRecordsDone { records, count } => {
                println!("Total records: {count}");
                let mut not_downloaded = Vec::new();
                for rec in &records {
                    let sync_status = rec.extra.get("syncStatus")
                        .and_then(|v| v.get("status")).and_then(|v| v.as_str()).unwrap_or("unknown");
                    println!("  Record: id={}, title='{}', sync={sync_status}", rec.uuid, rec.title);
                    if sync_status == "notDownloaded" { not_downloaded.push(rec.uuid.as_str()); }
                }
                if !not_downloaded.is_empty() {
                    println!("\nRequesting download for {} records...", not_downloaded.len());
                    handle.request_download_records(&not_downloaded).await?;
                } else {
                    println!("\nAll records are available locally.");
                    break;
                }
            }
            CortexEvent::DownloadRecordsDone(data) => { println!("Download result: {data}"); break; }
            CortexEvent::Error(msg) => { eprintln!("Error: {msg}"); break; }
            CortexEvent::Disconnected => { println!("Disconnected"); break; }
            _ => {}
        }
    }
    Ok(())
}
