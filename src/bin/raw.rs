//! Raw BLE/USB device streaming CLI.
//!
//! Direct connection to Emotiv headsets via BLE or USB, bypassing the Cortex API.
//!
//! # Usage
//!
//! ```bash
//! # Discover devices
//! cargo run --bin emotiv-raw --features raw -- --list
//!
//! # Connect to a specific device
//! cargo run --bin emotiv-raw --features raw -- --connect "MOCK-SN-000001"
//!
//! # Connect to first discovered device
//! cargo run --bin emotiv-raw --features raw
//! ```

use anyhow::Result;
use log::{error, info};
use std::io::{self, BufRead};

#[cfg(feature = "raw")]
use emotiv::raw;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    #[cfg(not(feature = "raw"))]
    {
        eprintln!("Error: raw mode requires the `raw` feature.");
        eprintln!("  cargo run --bin emotiv-raw --features raw");
        std::process::exit(1);
    }

    let args: Vec<String> = std::env::args().collect();

    // Handle --list flag
    if args.contains(&"--list".to_string()) {
        #[cfg(feature = "raw")]
        list_devices().await?;
        return Ok(());
    }

    // Get device address from --connect flag or use first available
    let device_address = args
        .windows(2)
        .find_map(|w| {
            if w[0] == "--connect" {
                Some(w[1].clone())
            } else {
                None
            }
        });

    #[cfg(feature = "raw")]
    {
        match device_address {
            Some(addr) => connect_to_device(&addr).await?,
            None => connect_to_first_device().await?,
        }
    }

    Ok(())
}

#[cfg(feature = "raw")]
async fn list_devices() -> Result<()> {
    info!("Discovering Emotiv devices...\n");
    let devices = raw::discover_devices().await?;

    if devices.is_empty() {
        info!("No devices found.");
        return Ok(());
    }

    println!("┌─────────────────────────────────────────────────────────────────────┐");
    println!("│ Found {} device(s):", devices.len());
    println!("├─────────────────────────────────────────────────────────────────────┤");

    for (i, device) in devices.iter().enumerate() {
        println!(
            "│ #{} - {} ({})",
            i + 1,
            device.model,
            device.transport
        );
        println!("│     Name:     {}", device.name);
        println!("│     BLE ID:   {}", device.ble_id);
        println!(
            "│     BLE MAC:  {}",
            device.ble_mac.as_deref().unwrap_or("(unavailable)")
        );
        println!("│     Address:  {}", device.address);
        println!("│     Serial:   {}", device.serial);
        println!("│     Battery:  {}%", device.battery_percent);
        println!("│     EEG Ch:   {}", device.model.channel_count());
    }
    println!("└─────────────────────────────────────────────────────────────────────┘");
    println!("\nUsage:");
    println!("  cargo run --bin emotiv-raw --features raw -- --connect \"MOCK-SN-000001\"");

    Ok(())
}

#[cfg(feature = "raw")]
async fn connect_to_first_device() -> Result<()> {
    info!("Discovering Emotiv devices...");
    let devices = raw::discover_devices().await?;

    if devices.is_empty() {
        error!("No devices found!");
        return Ok(());
    }

    let device = &devices[0];
    info!(
        "Connecting to {} ({}) [{} / {}] - {}",
        device.model.name(),
        device.serial,
        device.name,
        device.ble_mac.as_deref().unwrap_or("no-mac"),
        device.transport
    );

    stream_device(device.clone()).await?;
    Ok(())
}

#[cfg(feature = "raw")]
async fn connect_to_device(address: &str) -> Result<()> {
    let devices = raw::discover_devices().await?;
    let device = devices
        .iter()
        .find(|d| id_match(&d.address, address) || id_match(&d.serial, address))
        .ok_or_else(|| anyhow::anyhow!("Device not found: {}", address))?;

    info!(
        "Connecting to {} ({}) [{} / {}] - {}",
        device.model.name(),
        device.serial,
        device.name,
        device.ble_mac.as_deref().unwrap_or("no-mac"),
        device.transport
    );

    stream_device(device.clone()).await?;
    Ok(())
}

#[cfg(feature = "raw")]
async fn stream_device(device: raw::DeviceInfo) -> Result<()> {
    let (mut rx, _handle) = raw::RawDevice::from_info(device).connect().await?;

    info!("✅ Connected! Streaming EEG data...");
    info!("Press Ctrl-C to stop.\n");

    let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            if line.is_ok() {
                if line_tx.send(line.unwrap()).is_err() {
                    break;
                }
            }
        }
    });

    let mut packet_count = 0u64;
    let start_time = std::time::Instant::now();

    println!("┌────────────────────────────────────────────────────────────────────┐");
    println!("│ EEG Stream - Press Ctrl-C to quit                                  │");
    println!("├────────────────────────────────────────────────────────────────────┤");

    loop {
        tokio::select! {
            Some(_line) = line_rx.recv() => {
                // Check for quit command
                break;
            }
            Some(data) = rx.recv() => {
                packet_count += 1;

                // Show every 16th packet (roughly 2x per second at 128 Hz)
                if packet_count % 16 == 0 {
                    let elapsed = start_time.elapsed().as_secs_f64();
                    let rate = packet_count as f64 / elapsed;

                    println!(
                        "│ Packet {:6} | Counter {:5} | Avg Rate: {:.1} Hz | Battery: {}%",
                        packet_count, data.counter, rate, data.battery_percent
                    );

                    // Show first 5 channels
                    let display_ch = 5.min(data.eeg_uv.len());
                    print!("│ CH[0..{}]: [", display_ch);
                    for (i, uv) in data.eeg_uv[..display_ch].iter().enumerate() {
                        print!(" {:8.1}µV", uv);
                        if i < display_ch - 1 {
                            print!(",");
                        }
                    }
                    println!(" ]");
                }

                // Check signal quality
                if data.signal_quality < 2 {
                    eprintln!("⚠️  Low signal quality: {}", data.signal_quality);
                }
            }
            else => break,
        }
    }

    println!("├────────────────────────────────────────────────────────────────────┤");
    let elapsed = start_time.elapsed().as_secs_f64();
    let avg_rate = packet_count as f64 / elapsed;
    println!(
        "│ Streamed {} packets in {:.1}s ({:.1} Hz avg)",
        packet_count, elapsed, avg_rate
    );
    println!("└────────────────────────────────────────────────────────────────────┘");

    Ok(())
}

#[cfg(feature = "raw")]
fn id_match(candidate: &str, input: &str) -> bool {
    if candidate.eq_ignore_ascii_case(input) {
        return true;
    }
    let a = normalize_id(candidate);
    let b = normalize_id(input);
    !a.is_empty() && !b.is_empty() && (a == b || a.contains(&b) || b.contains(&a))
}

#[cfg(feature = "raw")]
fn normalize_id(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}
