//! Inject markers into a recording — mirrors `cortex-example/python/marker.py`.
//!
//! Creates a recording, injects time-stamped markers at intervals, then stops.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example marker
//! ```
//!
//! # API credentials
//!
//! Requires `EMOTIV_CLIENT_ID` and `EMOTIV_CLIENT_SECRET` environment variables.
//! Create them at <https://www.emotiv.com/my-account/cortex-apps/>.

use anyhow::Result;
use std::time::{SystemTime, UNIX_EPOCH};

use emotiv::client::{CortexClient, CortexClientConfig};
use emotiv::types::*;

const NUM_MARKERS: usize = 5;
const MARKER_INTERVAL_SECS: u64 = 3;
const MARKER_VALUE: &str = "test_marker_value";
const MARKER_LABEL_PREFIX: &str = "rust_marker";
const RECORD_TITLE: &str = "Rust Marker Example";

fn now_ms() -> f64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64()*1000.0 }

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let client_id = std::env::var("EMOTIV_CLIENT_ID").expect("Set EMOTIV_CLIENT_ID");
    let client_secret = std::env::var("EMOTIV_CLIENT_SECRET").expect("Set EMOTIV_CLIENT_SECRET");

    let config = CortexClientConfig { client_id, client_secret, ..Default::default() };
    let client = CortexClient::new(config);
    let (mut rx, handle) = client.connect().await?;
    let mut marker_idx = 0usize;

    while let Some(event) = rx.recv().await {
        match event {
            CortexEvent::SessionCreated(_) => {
                println!("Session created. Syncing headset clock...");
                handle.sync_headset_clock().await?;
            }
            CortexEvent::HeadsetClockSynced(_) => {
                println!("Clock synced. Creating record...");
                handle.create_record(RECORD_TITLE, "Marker injection example").await?;
            }
            CortexEvent::RecordCreated(rec) => {
                println!("Record created: {}", rec.uuid);
                println!("Injecting {NUM_MARKERS} markers (every {MARKER_INTERVAL_SECS}s)...");
                let h = handle.clone();
                tokio::spawn(async move {
                    for m in 0..NUM_MARKERS {
                        let label = format!("{MARKER_LABEL_PREFIX}_{m}");
                        if let Err(e) = h.inject_marker(now_ms(), MARKER_VALUE, &label).await {
                            eprintln!("Failed to inject marker: {e}");
                        }
                        if m+1 < NUM_MARKERS { tokio::time::sleep(std::time::Duration::from_secs(MARKER_INTERVAL_SECS)).await; }
                    }
                });
            }
            CortexEvent::MarkerInjected(marker) => {
                println!("Marker injected: id={} type={} label={}", marker.uuid, marker.marker_type, marker.label);
                marker_idx += 1;
                if marker_idx >= NUM_MARKERS {
                    println!("All markers injected. Stopping record...");
                    handle.stop_record().await?;
                }
            }
            CortexEvent::RecordStopped(rec) => { println!("Record stopped: {}", rec.uuid); break; }
            CortexEvent::Error(msg) => eprintln!("Error: {msg}"),
            CortexEvent::Disconnected => { println!("Disconnected"); break; }
            _ => {}
        }
    }

    Ok(())
}
