//! Direct BLE/USB device connection and data streaming.
//!
//! This module provides raw hardware access to Emotiv headsets via BLE or USB,
//! bypassing the Cortex API and connecting directly to the device.
//!
//! # Features
//!
//! - **Device discovery**: Auto-detect connected Emotiv headsets
//! - **BLE support**: EPOC X, EPOC+, Insight 2
//! - **USB support**: EPOC+ and other USB-connected models
//! - **Decryption**: Full AES-ECB packet decryption (binary-compatible)
//! - **Streaming**: Async data stream via mpsc channel
//!
//! # Usage
//!
//! ```no_run
//! use emotiv::raw::{RawDevice, DeviceType};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Discover devices
//!     let devices = RawDevice::discover().await?;
//!     println!("Found {} devices", devices.len());
//!
//!     if let Some(device) = devices.first() {
//!         // Connect and stream
//!         let (mut rx, handle) = device.connect().await?;
//!         while let Some(data) = rx.recv().await {
//!             println!("EEG samples: {:?}", &data.eeg_uv[..5.min(data.eeg_uv.len())]);
//!         }
//!     }
//!     Ok(())
//! }
//! ```

pub mod device;
pub mod decryption;
pub mod types;

pub use device::{DeviceInfo, RawDevice, TransportType};
pub use decryption::Decryptor;
pub use types::*;

use anyhow::Result;

/// Discover all connected Emotiv devices (BLE and USB).
pub async fn discover_devices() -> Result<Vec<DeviceInfo>> {
    RawDevice::discover().await
}

/// Connect to a device by address/serial number.
pub async fn connect_device(address: &str) -> Result<(tokio::sync::mpsc::Receiver<DecryptedData>, RawDevice)> {
    let devices = discover_devices().await?;
    let device = devices
        .iter()
        .find(|d| id_match(&d.address, address) || id_match(&d.serial, address))
        .ok_or_else(|| anyhow::anyhow!("Device not found: {}", address))?;

    let raw = RawDevice::from_info(device.clone());
    let (rx, _handle) = raw.connect().await?;
    Ok((rx, raw))
}

fn id_match(candidate: &str, input: &str) -> bool {
    if candidate.eq_ignore_ascii_case(input) {
        return true;
    }
    let a = normalize_id(candidate);
    let b = normalize_id(input);
    !a.is_empty() && !b.is_empty() && (a == b || a.contains(&b) || b.contains(&a))
}

fn normalize_id(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}
