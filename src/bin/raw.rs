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
//! cargo run --bin emotiv-raw --features raw -- --connect "<BLE_ID_OR_SERIAL>"
//!
//! # Connect to first discovered device
//! cargo run --bin emotiv-raw --features raw
//!
//! # Decode audit mode (live decode sanity scoring)
//! cargo run --bin emotiv-raw --features raw -- --decode-audit
//! ```

use anyhow::Result;
use log::{error, info};
use std::collections::VecDeque;
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
    let debug_view = args.contains(&"--debug-view".to_string());
    let decode_audit = args.contains(&"--decode-audit".to_string());

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
            Some(addr) => connect_to_device(&addr, debug_view, decode_audit).await?,
            None => connect_to_first_device(debug_view, decode_audit).await?,
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
    println!("  cargo run --bin emotiv-raw --features raw -- --connect \"<BLE_ID_OR_SERIAL>\"");

    Ok(())
}

#[cfg(feature = "raw")]
async fn connect_to_first_device(debug_view: bool, decode_audit: bool) -> Result<()> {
    info!("Discovering Emotiv devices...");
    let devices = raw::discover_devices().await?;

    if devices.is_empty() {
        error!("No devices found!");
        return Ok(());
    }

    let device = select_best_device(&devices)
        .ok_or_else(|| anyhow::anyhow!("No suitable BLE device candidate found"))?;
    info!(
        "Connecting to {} ({}) [{} / {}] - {}",
        device.model.name(),
        device.serial,
        device.name,
        device.ble_mac.as_deref().unwrap_or("no-mac"),
        device.transport
    );

    stream_device(device.clone(), debug_view, decode_audit).await?;
    Ok(())
}

#[cfg(feature = "raw")]
async fn connect_to_device(address: &str, debug_view: bool, decode_audit: bool) -> Result<()> {
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

    stream_device(device.clone(), debug_view, decode_audit).await?;
    Ok(())
}

#[cfg(feature = "raw")]
async fn stream_device(device: raw::DeviceInfo, debug_view: bool, decode_audit: bool) -> Result<()> {
    let (_, model_uv_max) = device.model.eeg_physical_range();
    let model_channel_count = device.model.channel_count();
    let (mut rx, handle) = raw::RawDevice::from_info(device).connect().await?;

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
    let mut low_quality_warn_count = 0u64;
    let start_time = std::time::Instant::now();

    println!("┌────────────────────────────────────────────────────────────────────┐");
    println!("│ EEG Stream - Press Ctrl-C to quit                                  │");
    println!("├────────────────────────────────────────────────────────────────────┤");
    if debug_view {
        println!("│ DEBUG view enabled: showing rx/decode counters every 2s            │");
        println!("├────────────────────────────────────────────────────────────────────┤");
    }
    if decode_audit {
        println!("│ DECODE AUDIT enabled: continuity/clip/rms/correlation checks       │");
        println!("├────────────────────────────────────────────────────────────────────┤");
    }

    let mut debug_tick = tokio::time::interval(std::time::Duration::from_secs(2));
    let mut audit_tick = tokio::time::interval(std::time::Duration::from_secs(3));

    let mut prev_counter: Option<u32> = None;
    let mut counter_jumps = 0u64;
    let mut sample_count = 0u64;
    let mut clipped_count = 0u64;
    let mut abs_sum = 0.0f64;
    let mut sq_sum = 0.0f64;
    let audit_ch = 4usize.min(model_channel_count).max(1);
    let mut corr_hist: Vec<VecDeque<f64>> = (0..audit_ch)
        .map(|_| VecDeque::with_capacity(512))
        .collect();

    loop {
        tokio::select! {
            Some(_line) = line_rx.recv() => {
                // Check for quit command
                break;
            }
            _ = debug_tick.tick(), if debug_view => {
                let stats = handle.debug_stats().await;
                println!(
                    "│ DBG rx={} dec={} fail={} timeout={} last_uuid={} len={} key={} writes={} subs={}",
                    stats.received_notifications,
                    stats.decoded_packets,
                    stats.decrypt_failures,
                    stats.timeout_count,
                    stats.last_notify_uuid.as_deref().unwrap_or("-"),
                    stats.last_payload_len,
                    stats.active_serial_candidate.as_deref().unwrap_or("-"),
                    stats.start_command_writes,
                    stats.subscribed_characteristics.len(),
                );
                if !stats.subscribed_characteristics.is_empty() {
                    println!(
                        "│ DBG subs: {}",
                        stats.subscribed_characteristics.join(",")
                    );
                }
            }
            _ = audit_tick.tick(), if decode_audit => {
                let continuity = if packet_count == 0 {
                    0.0
                } else {
                    1.0 - (counter_jumps as f64 / packet_count as f64)
                };

                let clip_rate = if sample_count == 0 {
                    0.0
                } else {
                    clipped_count as f64 / sample_count as f64
                };

                let mean_abs = if sample_count == 0 {
                    0.0
                } else {
                    abs_sum / sample_count as f64
                };

                let rms = if sample_count == 0 {
                    0.0
                } else {
                    (sq_sum / sample_count as f64).sqrt()
                };

                let corr = average_pairwise_corr(&corr_hist).unwrap_or(0.0);

                let verdict = if continuity > 0.99 && clip_rate < 0.02 && rms < 300.0 && corr.abs() > 0.02 {
                    "GOOD"
                } else if continuity > 0.95 && clip_rate < 0.10 && rms < 1200.0 {
                    "WARN"
                } else {
                    "BAD"
                };

                println!(
                    "│ AUDIT {verdict:<4} cont={:.3} clip={:.1}% mean|uV|={:.1} rms={:.1} corr={:.3}",
                    continuity,
                    clip_rate * 100.0,
                    mean_abs,
                    rms,
                    corr,
                );
            }
            maybe_data = rx.recv() => {
                let Some(data) = maybe_data else {
                    break;
                };

                packet_count += 1;

                if let Some(prev) = prev_counter {
                    let expected = (prev + 1) & 0xFFFF;
                    if data.counter != expected {
                        counter_jumps += 1;
                    }
                }
                prev_counter = Some(data.counter);

                for (ch_idx, value) in data.eeg_uv.iter().enumerate() {
                    if value.abs() >= model_uv_max * 0.98 {
                        clipped_count += 1;
                    }
                    abs_sum += value.abs();
                    sq_sum += value * value;
                    sample_count += 1;

                    if ch_idx < audit_ch {
                        let hist = &mut corr_hist[ch_idx];
                        hist.push_back(*value);
                        while hist.len() > 512 {
                            hist.pop_front();
                        }
                    }
                }

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
                let cq_unknown = !data.contact_quality.is_empty()
                    && data.contact_quality.iter().all(|&v| v == 0);
                if data.signal_quality < 2 && !cq_unknown && packet_count % 64 == 0 {
                    low_quality_warn_count += 1;
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
        "│ Streamed {} packets in {:.1}s ({:.1} Hz avg) | low-signal warns {}",
        packet_count, elapsed, avg_rate, low_quality_warn_count
    );
    println!("└────────────────────────────────────────────────────────────────────┘");

    Ok(())
}

#[cfg(feature = "raw")]
fn average_pairwise_corr(ch_hist: &[VecDeque<f64>]) -> Option<f64> {
    if ch_hist.len() < 2 {
        return None;
    }

    let min_len = ch_hist.iter().map(|h| h.len()).min().unwrap_or(0);
    if min_len < 64 {
        return None;
    }

    let mut total = 0.0f64;
    let mut pairs = 0u32;
    for i in 0..ch_hist.len() {
        for j in (i + 1)..ch_hist.len() {
            if let Some(c) = pearson_tail(&ch_hist[i], &ch_hist[j], min_len.min(256)) {
                total += c;
                pairs += 1;
            }
        }
    }

    if pairs == 0 {
        None
    } else {
        Some(total / pairs as f64)
    }
}

#[cfg(feature = "raw")]
fn pearson_tail(a: &VecDeque<f64>, b: &VecDeque<f64>, n: usize) -> Option<f64> {
    if n == 0 || a.len() < n || b.len() < n {
        return None;
    }

    let a_start = a.len() - n;
    let b_start = b.len() - n;
    let mut sum_a = 0.0;
    let mut sum_b = 0.0;
    for k in 0..n {
        sum_a += a[a_start + k];
        sum_b += b[b_start + k];
    }
    let mean_a = sum_a / n as f64;
    let mean_b = sum_b / n as f64;

    let mut num = 0.0;
    let mut den_a = 0.0;
    let mut den_b = 0.0;
    for k in 0..n {
        let da = a[a_start + k] - mean_a;
        let db = b[b_start + k] - mean_b;
        num += da * db;
        den_a += da * da;
        den_b += db * db;
    }

    let den = (den_a * den_b).sqrt();
    if den < 1e-9 {
        None
    } else {
        Some((num / den).clamp(-1.0, 1.0))
    }
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

#[cfg(feature = "raw")]
fn select_best_device(devices: &[raw::DeviceInfo]) -> Option<&raw::DeviceInfo> {
    let likely: Vec<&raw::DeviceInfo> = devices.iter().filter(|d| is_likely_emotiv(d)).collect();
    if !likely.is_empty() {
        likely.into_iter().max_by_key(|d| device_score(d))
    } else {
        devices.iter().max_by_key(|d| device_score(d))
    }
}

#[cfg(feature = "raw")]
fn device_score(d: &raw::DeviceInfo) -> i32 {
    let mut score = 0;
    let name = d.name.to_ascii_lowercase();
    if name != "(unknown)" {
        score += 20;
    }
    if name.contains("emotiv")
        || name.contains("epoc")
        || name.contains("insight")
        || name.contains("flex")
        || name.contains("mn8")
        || name.contains("xtrodes")
    {
        score += 50;
    }
    if d.ble_mac.as_deref().is_some_and(|m| !is_zero_mac(m)) {
        score += 15;
    }
    if !normalize_id(&d.ble_id).is_empty() {
        score += 10;
    }
    if d.is_connected {
        score += 10;
    }
    score
}

#[cfg(feature = "raw")]
fn is_likely_emotiv(d: &raw::DeviceInfo) -> bool {
    let name = d.name.to_ascii_lowercase();
    if name.contains("emotiv")
        || name.contains("epoc")
        || name.contains("insight")
        || name.contains("flex")
        || name.contains("mn8")
        || name.contains("xtrodes")
    {
        return true;
    }

    // Exclude clearly generic unknown/autogenerated names from auto-connect.
    if name == "(unknown)" || name.starts_with("gb-") || name.starts_with("ble") {
        return false;
    }

    // Keep manually paired/connected devices as likely candidates.
    d.is_connected
}

#[cfg(feature = "raw")]
fn is_zero_mac(mac: &str) -> bool {
    let only_hex: String = mac
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_lowercase();
    !only_hex.is_empty() && only_hex.chars().all(|c| c == '0')
}
