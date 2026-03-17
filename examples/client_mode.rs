//! Client mode example: switch between basic and resilient clients.
//!
//! # Usage
//!
//! ```bash
//! # Basic mode (CortexClient)
//! cargo run --example client_mode
//!
//! # Resilient mode (ResilientClient with auto-reconnect + health checks)
//! cargo run --example client_mode -- --resilient
//! ```

use anyhow::Result;

use emotiv::client::{CortexClient, CortexClientConfig};
use emotiv::config::CortexConfig;
use emotiv::reconnect::ResilientClient;
use emotiv::types::CortexEvent;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let resilient = std::env::args().any(|arg| arg == "--resilient");
    let streams = ["eeg", "met"];

    if resilient {
        println!("Running in resilient mode");
        run_resilient_mode(&streams).await?;
    } else {
        println!("Running in basic mode");
        run_basic_mode(&streams).await?;
    }

    Ok(())
}

async fn run_basic_mode(streams: &[&str]) -> Result<()> {
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
                println!("[basic] Session created: {id}");
                handle.subscribe(streams).await?;
            }
            CortexEvent::Eeg(data) => {
                println!("[basic] EEG samples: {}", data.samples.len());
            }
            CortexEvent::Metrics(data) => {
                println!("[basic] Metrics values: {}", data.values.len());
            }
            CortexEvent::Disconnected => {
                println!("[basic] Disconnected");
                break;
            }
            CortexEvent::Error(message) => {
                eprintln!("[basic] Error: {message}");
            }
            _ => {}
        }
    }

    Ok(())
}

async fn run_resilient_mode(streams: &[&str]) -> Result<()> {
    let config = CortexConfig::discover(None)?;
    let (client, mut event_rx) = ResilientClient::connect(config).await?;

    let mut conn_rx = client.connection_event_receiver();
    tokio::spawn(async move {
        while let Ok(conn_event) = conn_rx.recv().await {
            println!("[resilient] Connection event: {:?}", conn_event);
        }
    });

    while let Ok(event) = event_rx.recv().await {
        match event {
            CortexEvent::SessionCreated(id) => {
                println!("[resilient] Session created: {id}");
                client.subscribe(streams).await?;
            }
            CortexEvent::Eeg(data) => {
                println!("[resilient] EEG samples: {}", data.samples.len());
            }
            CortexEvent::Metrics(data) => {
                println!("[resilient] Metrics values: {}", data.values.len());
            }
            CortexEvent::Disconnected => {
                println!("[resilient] Disconnected event observed");
            }
            CortexEvent::Error(message) => {
                eprintln!("[resilient] Error: {message}");
            }
            _ => {}
        }
    }

    Ok(())
}
