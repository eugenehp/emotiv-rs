//! Headless CLI: connect to an Emotiv headset and print all events to stdout.
//!
//! # Usage
//!
//! ```bash
//! # Real headset (requires EMOTIV Launcher + API credentials)
//! cargo run --bin emotiv-cli
//!
//! # Simulated signal (no hardware or credentials needed)
//! cargo run --bin emotiv-cli --features simulate -- --simulate
//! ```
//!
//! # API credentials
//!
//! Create a Cortex App at <https://www.emotiv.com/my-account/cortex-apps/>
//! to get a Client ID and Client Secret, then export them:
//!
//! ```bash
//! export EMOTIV_CLIENT_ID="your_client_id"
//! export EMOTIV_CLIENT_SECRET="your_client_secret"
//! ```
//!
//! On the first run the EMOTIV Launcher will ask you to approve the app.
//!
//! # Environment variables
//!
//! | Variable | Description |
//! |---|---|
//! | `EMOTIV_CLIENT_ID` | Cortex App client ID (required for real headset) |
//! | `EMOTIV_CLIENT_SECRET` | Cortex App client secret (required for real headset) |
//! | `EMOTIV_DEBUG` | Set to any value to log all WebSocket messages |
//! | `RUST_LOG` | Log verbosity, e.g. `RUST_LOG=debug` |

use std::io::{self, BufRead};

use anyhow::Result;
use log::{error, info};

use emotiv::client::{CortexClient, CortexClientConfig};
use emotiv::types::*;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let simulate = std::env::args().any(|a| a == "--simulate");

    #[cfg(not(feature = "simulate"))]
    if simulate {
        eprintln!("Error: --simulate requires the `simulate` feature.");
        eprintln!("  cargo run --bin emotiv-cli --features simulate -- --simulate");
        std::process::exit(1);
    }

    let (mut rx, handle) = if simulate {
        #[cfg(feature = "simulate")]
        {
            use emotiv::simulator::{SimulatorConfig, spawn_simulator};
            use tokio::sync::mpsc;

            info!("Running in simulation mode (no headset required)");
            let (tx, rx) = mpsc::channel(512);
            spawn_simulator(SimulatorConfig::default(), tx);
            let dummy_handle: Option<emotiv::client::CortexHandle> = None;
            (rx, dummy_handle)
        }
        #[cfg(not(feature = "simulate"))]
        unreachable!()
    } else {
        let client_id = std::env::var("EMOTIV_CLIENT_ID")
            .unwrap_or_else(|_| "your_client_id".into());
        let client_secret = std::env::var("EMOTIV_CLIENT_SECRET")
            .unwrap_or_else(|_| "your_client_secret".into());

        let config = CortexClientConfig {
            client_id,
            client_secret,
            debug_mode: std::env::var("EMOTIV_DEBUG").is_ok(),
            ..Default::default()
        };

        let client = CortexClient::new(config);
        info!("Connecting to Emotiv Cortex service...");
        let (rx, h) = client.connect().await?;
        (rx, Some(h))
    };

    info!("Streaming started. Press Ctrl-C or type 'q' + Enter to quit.");
    info!("Commands: q=quit, s=subscribe, u=unsubscribe\n");

    // Stdin command loop (dedicated thread)
    let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) => {
                    if line_tx.send(l.trim().to_owned()).is_err() { break; }
                }
                Err(_) => break,
            }
        }
    });

    // Process stdin commands
    if let Some(ref h) = handle {
        let h = h.clone();
        tokio::spawn(async move {
            while let Some(line) = line_rx.recv().await {
                if line.is_empty() { continue; }
                match line.as_str() {
                    "q" => {
                        info!("Quit requested.");
                        std::process::exit(0);
                    }
                    "s" => {
                        info!("Subscribing to all streams...");
                        if let Err(e) = h.subscribe(&["eeg", "mot", "dev", "met", "pow"]).await {
                            error!("Subscribe error: {e}");
                        }
                    }
                    "u" => {
                        info!("Unsubscribing from all streams...");
                        if let Err(e) = h.unsubscribe(&["eeg", "mot", "dev", "met", "pow"]).await {
                            error!("Unsubscribe error: {e}");
                        }
                    }
                    _ => { info!("Unknown command: {line}"); }
                }
            }
        });
    }

    // Main event loop
    let mut eeg_count = 0u64;
    while let Some(event) = rx.recv().await {
        match event {
            CortexEvent::Connected => info!("✅ Connected to Cortex service"),
            CortexEvent::Authorized => info!("🔑 Authorized successfully"),
            CortexEvent::SessionCreated(id) => {
                info!("📋 Session created: {id}");
                if let Some(ref h) = handle {
                    if let Err(e) = h.subscribe(&["eeg", "mot", "dev", "met", "pow"]).await {
                        error!("Subscribe error: {e}");
                    }
                }
            }
            CortexEvent::Disconnected => { info!("❌ Disconnected"); break; }
            CortexEvent::Error(msg) => error!("⚠️  Error: {msg}"),
            CortexEvent::Eeg(data) => {
                eeg_count += 1;
                if eeg_count % 128 == 1 {
                    let preview: Vec<String> = data.samples.iter().take(5)
                        .map(|v| format!("{v:+8.3}")).collect();
                    println!("[EEG] #{eeg_count:<8} t={:.3}  [{} ...]", data.time, preview.join(", "));
                }
            }
            CortexEvent::Motion(data) => {
                let acc = &data.samples[6..9.min(data.samples.len())];
                println!("[MOT] t={:.3}  acc=[{:.4}, {:.4}, {:.4}]",
                    data.time, acc.first().unwrap_or(&0.0), acc.get(1).unwrap_or(&0.0), acc.get(2).unwrap_or(&0.0));
            }
            CortexEvent::Dev(data) => {
                println!("[DEV] battery={:.0}%  signal={:.1}  cq={:?}",
                    data.battery_percent, data.signal, &data.contact_quality[..5.min(data.contact_quality.len())]);
            }
            CortexEvent::Metrics(data) => {
                let eng = data.values.get(1).unwrap_or(&0.0);
                let exc = data.values.get(3).unwrap_or(&0.0);
                let foc = data.values.get(12).unwrap_or(&0.0);
                println!("[MET] engagement={eng:.3}  excitement={exc:.3}  focus={foc:.3}");
            }
            CortexEvent::BandPower(data) => {
                let preview: Vec<String> = data.powers.iter().take(5).map(|v| format!("{v:.3}")).collect();
                println!("[POW] [{} ...]", preview.join(", "));
            }
            CortexEvent::MentalCommand(data) => {
                println!("[COM] action={:<10} power={:.3}", data.action, data.power);
            }
            CortexEvent::FacialExpression(data) => {
                println!("[FAC] eye={:<10} upper={}({:.2}) lower={}({:.2})",
                    data.eye_action, data.upper_action, data.upper_power, data.lower_action, data.lower_power);
            }
            CortexEvent::DataLabels(labels) => {
                println!("[LABELS] {}: {:?}", labels.stream_name, labels.labels);
            }
            CortexEvent::Warning { code, message } => {
                println!("[WARN] code={code} msg={message}");
            }
            _ => {}
        }
    }

    info!("Event loop finished – exiting.");
    Ok(())
}
