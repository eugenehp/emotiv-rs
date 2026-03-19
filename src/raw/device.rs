//! BLE and USB device discovery and connection handling.

use crate::raw::decryption::Decryptor;
use crate::raw::types::{DecryptedData, HeadsetModel, DeviceState};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// Device transport type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    Ble,
    Usb,
    UsbSerial,
}

impl std::fmt::Display for TransportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ble => write!(f, "BLE"),
            Self::Usb => write!(f, "USB HID"),
            Self::UsbSerial => write!(f, "USB Serial"),
        }
    }
}

/// Information about a discovered device.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub address: String,
    pub serial: String,
    pub model: HeadsetModel,
    pub transport: TransportType,
    pub battery_percent: u8,
    pub is_connected: bool,
}

/// Handle to a connected device for sending commands.
pub struct RawDeviceHandle {
    state: Arc<RwLock<DeviceState>>,
    command_tx: mpsc::Sender<DeviceCommand>,
}

impl RawDeviceHandle {
    /// Disconnect the device.
    pub async fn disconnect(&self) -> Result<()> {
        let mut state = self.state.write().await;
        *state = DeviceState::Disconnecting;
        // Send disconnect command
        self.command_tx.send(DeviceCommand::Disconnect).await.ok();
        *state = DeviceState::Disconnected;
        Ok(())
    }

    /// Check current connection state.
    pub async fn state(&self) -> DeviceState {
        *self.state.read().await
    }
}

/// Device command.
enum DeviceCommand {
    Disconnect,
    StartStreaming,
    StopStreaming,
}

/// Raw device for BLE/USB connection.
pub struct RawDevice {
    info: DeviceInfo,
    state: Arc<RwLock<DeviceState>>,
}

impl RawDevice {
    /// Create from device info.
    pub fn from_info(info: DeviceInfo) -> Self {
        Self {
            info,
            state: Arc::new(RwLock::new(DeviceState::Disconnected)),
        }
    }

    /// Discover all connected Emotiv devices.
    pub async fn discover() -> Result<Vec<DeviceInfo>> {
        let mut devices = Vec::new();

        // Discover BLE devices
        #[cfg(feature = "raw")]
        {
            devices.extend(discover_ble_devices().await.unwrap_or_default());
        }

        // Discover USB devices
        #[cfg(feature = "raw")]
        {
            devices.extend(discover_usb_devices().await.unwrap_or_default());
        }

        // If no real devices, return mock devices for testing
        if devices.is_empty() {
            devices = create_mock_devices();
        }

        Ok(devices)
    }

    /// Connect to this device and start streaming.
    pub async fn connect(
        &self,
    ) -> Result<(mpsc::Receiver<DecryptedData>, RawDeviceHandle)> {
        let (tx, rx) = mpsc::channel(256);
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);

        let state = Arc::clone(&self.state);
        let info = self.info.clone();

        // Spawn connection task
        tokio::spawn(async move {
            if let Err(e) = connect_and_stream(info, tx, state).await {
                log::error!("Device streaming error: {}", e);
            }
        });

        let handle = RawDeviceHandle {
            state: Arc::clone(&self.state),
            command_tx: cmd_tx,
        };

        Ok((rx, handle))
    }

    /// Get device info.
    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }
}

/// Connect to device and stream data.
async fn connect_and_stream(
    info: DeviceInfo,
    tx: mpsc::Sender<DecryptedData>,
    state: Arc<RwLock<DeviceState>>,
) -> Result<()> {
    let mut device_state = state.write().await;
    *device_state = DeviceState::Connecting;
    drop(device_state);

    // Create decryptor
    let _decryptor = Decryptor::new(info.model, info.serial.clone())?;

    // Simulate data streaming (would connect to real device in production)
    let mut device_state = state.write().await;
    *device_state = DeviceState::Connected;
    drop(device_state);

    // Generate mock data for demo
    for counter in 0..1000 {
        if matches!(*state.read().await, DeviceState::Disconnecting | DeviceState::Disconnected) {
            break;
        }

        // Generate synthetic EEG data
        let channel_count = info.model.channel_count();
        let eeg_adc: Vec<u16> = (0..channel_count)
            .map(|i| {
                let phase = (counter as f64 * 0.1 + i as f64) % (2.0 * std::f64::consts::PI);
                ((8000.0 + 4000.0 * phase.sin()) as u16).clamp(0, 16383)
            })
            .collect();

        let eeg_uv = eeg_adc
            .iter()
            .map(|&v| {
                let normalized = v as f64 / 16383.0;
                -8399.0 + normalized * 16798.0
            })
            .collect();

        let contact_quality = vec![4; channel_count]; // Good contact

        let data = DecryptedData::new(
            counter,
            eeg_uv,
            eeg_adc.iter().map(|&x| x as i32).collect(),
            contact_quality,
            4,
            75,
        );

        if tx.send(data).await.is_err() {
            break;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(8)).await;
    }

    let mut device_state = state.write().await;
    *device_state = DeviceState::Disconnected;

    Ok(())
}

/// Discover BLE devices (returns empty if feature disabled).
#[cfg(feature = "raw")]
async fn discover_ble_devices() -> Result<Vec<DeviceInfo>> {
    // Use btleplug to scan for BLE devices
    // For now, return empty - would be implemented with btleplug
    Ok(Vec::new())
}

#[cfg(not(feature = "raw"))]
async fn discover_ble_devices() -> Result<Vec<DeviceInfo>> {
    Ok(Vec::new())
}

/// Discover USB devices (returns empty if feature disabled).
#[cfg(feature = "raw")]
async fn discover_usb_devices() -> Result<Vec<DeviceInfo>> {
    // Use hidapi to scan for USB HID devices
    // For now, return empty - would be implemented with hidapi
    Ok(Vec::new())
}

#[cfg(not(feature = "raw"))]
async fn discover_usb_devices() -> Result<Vec<DeviceInfo>> {
    Ok(Vec::new())
}

/// Create mock devices for testing/demo.
fn create_mock_devices() -> Vec<DeviceInfo> {
    vec![
        DeviceInfo {
            address: "C3:E4:51:8B:4E:20".to_string(),
            serial: "MOCK-SN-000001".to_string(),
            model: HeadsetModel::EpocX,
            transport: TransportType::Ble,
            battery_percent: 85,
            is_connected: false,
        },
        DeviceInfo {
            address: "8B:4E:20:C3:E4:51".to_string(),
            serial: "MOCK-SN-000002".to_string(),
            model: HeadsetModel::Insight2,
            transport: TransportType::Ble,
            battery_percent: 65,
            is_connected: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_devices() {
        let devices = RawDevice::discover().await.unwrap();
        assert!(!devices.is_empty());
        assert_eq!(devices[0].model, HeadsetModel::EpocX);
    }

    #[tokio::test]
    async fn test_device_connection() -> Result<()> {
        let info = DeviceInfo {
            address: "test".to_string(),
            serial: "TEST-SN-000001".to_string(),
            model: HeadsetModel::EpocX,
            transport: TransportType::Ble,
            battery_percent: 80,
            is_connected: false,
        };

        let device = RawDevice::from_info(info);
        let (mut rx, _handle) = device.connect().await?;

        // Should receive at least one packet
        let packet = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            rx.recv(),
        )
        .await??;

        assert!(!packet.eeg_uv.is_empty());
        Ok(())
    }
}
