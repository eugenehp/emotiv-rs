//! Standalone simulation example — demonstrates the signal simulator without any hardware.
//!
//! No API credentials or EMOTIV Launcher required — purely synthetic data.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example simulate --features simulate
//! ```

use anyhow::Result;
use tokio::sync::mpsc;

use emotiv::simulator::{SimulatorConfig, spawn_simulator};
use emotiv::types::*;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    println!("=== Emotiv Signal Simulator ===");
    println!("Generating synthetic EEG, motion, metrics, band power, and mental command data.\n");

    let config = SimulatorConfig {
        num_eeg_channels: 14,
        eeg_rate_hz: 128.0,
        enable_motion: true,
        enable_metrics: true,
        enable_band_power: true,
        enable_dev: true,
        enable_mental_command: true,
        battery_percent: 90.0,
    };

    let (tx, mut rx) = mpsc::channel(512);
    spawn_simulator(config, tx);

    let mut eeg_count = 0u64;
    let mut mot_count = 0u64;
    let mut met_count = 0u64;
    let mut pow_count = 0u64;
    let mut dev_count = 0u64;
    let mut com_count = 0u64;

    println!("Streaming for 5 seconds...\n");

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                break;
            }
            event = rx.recv() => {
                match event {
                    Some(CortexEvent::Connected) => println!("[SIM] Connected"),
                    Some(CortexEvent::Authorized) => println!("[SIM] Authorized"),
                    Some(CortexEvent::SessionCreated(id)) => println!("[SIM] Session: {id}"),
                    Some(CortexEvent::Eeg(data)) => {
                        eeg_count += 1;
                        if eeg_count % 128 == 1 {
                            let preview: Vec<String> = data.samples.iter().take(4)
                                .map(|v| format!("{v:+7.2}")).collect();
                            println!("[EEG] #{eeg_count:<6} [{} ...]", preview.join(", "));
                        }
                    }
                    Some(CortexEvent::Motion(data)) => {
                        mot_count += 1;
                        if mot_count <= 3 {
                            let acc = &data.samples[6..9];
                            println!("[MOT] #{mot_count} acc=[{:.4}, {:.4}, {:.4}]", acc[0], acc[1], acc[2]);
                        }
                    }
                    Some(CortexEvent::Metrics(data)) => {
                        met_count += 1;
                        let eng = data.values.get(1).unwrap_or(&0.0);
                        let foc = data.values.get(12).unwrap_or(&0.0);
                        println!("[MET] #{met_count} engagement={eng:.3} focus={foc:.3}");
                    }
                    Some(CortexEvent::BandPower(data)) => {
                        pow_count += 1;
                        let preview: Vec<String> = data.powers.iter().take(5)
                            .map(|v| format!("{v:.2}")).collect();
                        println!("[POW] #{pow_count} [{} ...]", preview.join(", "));
                    }
                    Some(CortexEvent::Dev(data)) => {
                        dev_count += 1;
                        println!("[DEV] #{dev_count} battery={:.0}% signal={:.1}", data.battery_percent, data.signal);
                    }
                    Some(CortexEvent::MentalCommand(data)) => {
                        com_count += 1;
                        println!("[COM] #{com_count} action={:<10} power={:.3}", data.action, data.power);
                    }
                    Some(CortexEvent::DataLabels(labels)) => {
                        println!("[LABELS] {}: {:?}", labels.stream_name, &labels.labels[..5.min(labels.labels.len())]);
                    }
                    None => break,
                    _ => {}
                }
            }
        }
    }

    println!("\n=== Summary ===");
    println!("EEG packets:  {eeg_count}");
    println!("Motion packets: {mot_count}");
    println!("Metrics packets: {met_count}");
    println!("BandPower packets: {pow_count}");
    println!("Device packets: {dev_count}");
    println!("MentalCmd packets: {com_count}");

    Ok(())
}
