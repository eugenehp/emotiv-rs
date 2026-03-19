//! Example: Stream raw EEG data from BLE/USB devices.
//!
//! This example demonstrates direct device connection without the Cortex API.
//!
//! # Usage
//!
//! ```bash
//! # List available devices
//! cargo run --example raw_stream --features raw -- --list
//!
//! # Stream from first device
//! cargo run --example raw_stream --features raw
//!
//! # Stream from specific device by serial
//! cargo run --example raw_stream --features raw -- --device "MOCK-SN-000001"
//! ```

use anyhow::Result;
use log::info;

#[cfg(feature = "raw")]
fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();

    // Parse arguments
    let list_mode = args.contains(&"--list".to_string());
    let device_serial = args
        .windows(2)
        .find_map(|w| {
            if w[0] == "--device" {
                Some(w[1].clone())
            } else {
                None
            }
        });

    #[tokio::main]
    async fn run_client(list_mode: bool, device_serial: Option<String>) -> Result<()> {
        use emotiv::raw;

        if list_mode {
            list_devices().await?;
        } else {
            stream_from_device(device_serial).await?;
        }

        Ok(())
    }

    #[tokio::main]
    async fn list_devices() -> Result<()> {
        use emotiv::raw;

        info!("🔍 Discovering Emotiv devices...\n");
        let devices = raw::discover_devices().await?;

        if devices.is_empty() {
            info!("No devices found.");
            return Ok(());
        }

        println!("┌────────────────────────────────────────────────────────────┐");
        println!("│ Found {} device(s):", devices.len());
        println!("├────────────────────────────────────────────────────────────┤");

        for (i, device) in devices.iter().enumerate() {
            let transport = format!("{}", device.transport);
            let channels = device.model.channel_count();
            println!(
                "│ #{} │ {} │ {} channels │ {} │ {}% battery",
                i + 1,
                device.model,
                channels,
                transport,
                device.battery_percent
            );
            println!("│    └─ {}", device.serial);
        }
        println!("└────────────────────────────────────────────────────────────┘");

        Ok(())
    }

    #[tokio::main]
    async fn stream_from_device(device_serial: Option<String>) -> Result<()> {
        use emotiv::raw;

        info!("🔍 Discovering devices...");
        let devices = raw::discover_devices().await?;

        if devices.is_empty() {
            log::error!("❌ No devices found!");
            return Err(anyhow::anyhow!("No devices found"));
        }

        // Select device
        let device = if let Some(serial) = device_serial {
            devices
                .iter()
                .find(|d| d.serial.contains(&serial))
                .ok_or_else(|| anyhow::anyhow!("Device not found: {}", serial))?
        } else {
            &devices[0]
        };

        info!(
            "🎯 Connecting to {} ({}) via {}",
            device.model, device.serial, device.transport
        );

        let (mut rx, _handle) = raw::RawDevice::from_info(device.clone()).connect().await?;

        info!("✅ Connected! Streaming {}ch @ {} Hz\n", device.model.channel_count(), device.model.sampling_rate());
        info!(
            "Received │ Rate (Hz) │ Signal Quality │ Battery │ EEG (µV)\n\
            ──────────┼───────────┼────────────────┼─────────┼─────────────────────────────────────"
        );

        let start = std::time::Instant::now();
        let mut packet_count = 0u64;
        let mut first_batch = true;

        while let Some(data) = rx.recv().await {
            packet_count += 1;

            // Print status every 32 packets (roughly 4x per second at 128 Hz)
            if packet_count % 32 == 0 {
                let elapsed = start.elapsed().as_secs_f64();
                let rate = (packet_count as f64) / elapsed;
                let signal_symbol = match (data.signal_quality) {
                    4 => "████ Excellent",
                    3 => "███░ Good",
                    2 => "██░░ Fair",
                    1 => "█░░░ Poor",
                    _ => "░░░░ None",
                };

                let mut eeg_display = String::new();
                for (i, &uv) in data.eeg_uv.iter().take(3).enumerate() {
                    if i > 0 {
                        eeg_display.push_str(", ");
                    }
                    eeg_display.push_str(&format!("{: 8.1}", uv));
                }

                println!(
                    "  {pkt:6} │ {rate:9.2} │ {signal_symbol:<14} │ {bat:>3}%  │ {eeg}",
                    pkt = packet_count,
                    rate = rate,
                    signal_symbol = signal_symbol,
                    bat = data.battery_percent,
                    eeg = eeg_display
                );

                if first_batch && packet_count == 32 {
                    first_batch = false;
                    info!("(Streaming {} samples per second)", (rate / device.model.sampling_rate() as f64) as u32);
                }
            }

            // Safety check for signal quality
            if data.signal_quality < 1 {
                log::warn!("⚠️  Signal lost or very poor! Re-check electrode placement.");
            }
        }

        let elapsed = start.elapsed().as_secs_f64();
        let avg_rate = packet_count as f64 / elapsed;
        info!(
            "\n✓ Streamed {} packets in {:.1}s ({:.1} Hz)",
            packet_count, elapsed, avg_rate
        );

        Ok(())
    }

    run_client(list_mode, device_serial)
}

#[cfg(not(feature = "raw"))]
fn main() {
    eprintln!("Error: This example requires the `raw` feature.");
    eprintln!("  cargo run --example raw_stream --features raw");
    std::process::exit(1);
}
