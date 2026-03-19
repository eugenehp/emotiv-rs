//! BLE and USB device discovery and connection handling.

use crate::raw::decryption::Decryptor;
use crate::raw::types::{DecryptedData, HeadsetModel, DeviceState};
use anyhow::{anyhow, Result};
#[cfg(feature = "raw")]
use btleplug::api::{
    Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter,
};
#[cfg(feature = "raw")]
use btleplug::platform::{Manager, Peripheral};
#[cfg(feature = "raw")]
use futures_util::StreamExt;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
#[cfg(feature = "raw")]
use uuid::Uuid;

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
#[allow(dead_code)]
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
    #[cfg(not(feature = "raw"))]
    {
        let _ = info;
        let _ = tx;
        let _ = state;
        return Err(anyhow!("raw feature is disabled"));
    }

    #[cfg(feature = "raw")]
    {
    let mut device_state = state.write().await;
    *device_state = DeviceState::Connecting;
    drop(device_state);

    let adapter = default_adapter().await?;
    adapter.start_scan(ScanFilter::default()).await.ok();
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    let peripheral = find_peripheral_by_address(&adapter, &info.address)
        .await?
        .ok_or_else(|| anyhow!("BLE device not found: {}", info.address))?;

    if !peripheral.is_connected().await.unwrap_or(false) {
        peripheral.connect().await?;
    }
    peripheral.discover_services().await?;

    let mut decryptor = Decryptor::new(info.model, normalize_serial_12(&info.serial))?;

    let notify_chars = select_notify_characteristics(&peripheral);
    if notify_chars.is_empty() {
        return Err(anyhow!("No notifiable BLE characteristics found for device"));
    }

    for ch in &notify_chars {
        if let Err(err) = peripheral.subscribe(ch).await {
            log::warn!("Failed to subscribe {}: {}", ch.uuid, err);
        }
    }

    let mut notifications = peripheral.notifications().await?;

    let mut device_state = state.write().await;
    *device_state = DeviceState::Streaming;
    drop(device_state);

    loop {
        if matches!(
            *state.read().await,
            DeviceState::Disconnecting | DeviceState::Disconnected
        ) {
            break;
        }

        match notifications.next().await {
            Some(notification) => {
                let payload = notification.value;
                if payload.is_empty() {
                    continue;
                }

                match decryptor.decrypt_eeg_packet(&payload) {
                    Ok(data) => {
                        if tx.send(data).await.is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        log::debug!("Failed to decrypt packet: {}", err);
                    }
                }
            }
            None => break,
        }
    }

    let _ = peripheral.disconnect().await;

    let mut device_state = state.write().await;
    *device_state = DeviceState::Disconnected;

    Ok(())
    }
}

/// Discover BLE devices (returns empty if feature disabled).
#[cfg(feature = "raw")]
async fn discover_ble_devices() -> Result<Vec<DeviceInfo>> {
    let adapter = default_adapter().await?;
    adapter.start_scan(ScanFilter::default()).await?;
    let scan_secs = std::env::var("EMOTIV_RAW_SCAN_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(8)
        .max(3);
    tokio::time::sleep(tokio::time::Duration::from_secs(scan_secs)).await;

    let mut matched_devices = Vec::new();
    let mut fallback_devices = Vec::new();
    for peripheral in adapter.peripherals().await? {
        let Some(props) = peripheral.properties().await? else {
            continue;
        };

        let name = props.local_name.unwrap_or_default();
        let address = peripheral.id().to_string();
        let is_connected = peripheral.is_connected().await.unwrap_or(false);

        let model = infer_model_from_name(&name).unwrap_or(HeadsetModel::EpocX);
        let serial = infer_serial(&name, &address);
        let info = DeviceInfo {
            address,
            serial,
            model,
            transport: TransportType::Ble,
            battery_percent: 0,
            is_connected,
        };

        if is_emotiv_candidate(&name, &props.services, is_connected) {
            matched_devices.push(info);
        } else {
            let has_any_signal = !name.is_empty() || !props.services.is_empty() || is_connected;
            if has_any_signal {
                fallback_devices.push(info);
            }
        }
    }

    adapter.stop_scan().await.ok();
    if matched_devices.is_empty() {
        log::warn!(
            "No explicit Emotiv BLE advertisements found; returning all visible BLE peripherals ({})",
            fallback_devices.len()
        );
        Ok(fallback_devices)
    } else {
        Ok(matched_devices)
    }
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

#[cfg(feature = "raw")]
async fn default_adapter() -> Result<btleplug::platform::Adapter> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    adapters
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No BLE adapter available"))
}

#[cfg(feature = "raw")]
async fn find_peripheral_by_address(
    adapter: &btleplug::platform::Adapter,
    address: &str,
) -> Result<Option<Peripheral>> {
    for p in adapter.peripherals().await? {
        if p.id().to_string() == address {
            return Ok(Some(p));
        }
    }
    Ok(None)
}

#[cfg(feature = "raw")]
fn select_notify_characteristics(peripheral: &Peripheral) -> Vec<btleplug::api::Characteristic> {
    const EMOTIV_DATA_SERVICE: &str = "00001100-d102-11e1-9b23-00025b00a5a5";

    let data_uuid = Uuid::parse_str(EMOTIV_DATA_SERVICE).ok();
    let all: Vec<_> = peripheral.characteristics().into_iter().collect();

    let mut preferred: Vec<_> = all
        .iter()
        .filter(|ch| {
            ch.properties.contains(CharPropFlags::NOTIFY)
                && data_uuid
                    .map(|svc| ch.service_uuid == svc)
                    .unwrap_or(false)
        })
        .cloned()
        .collect();

    if preferred.is_empty() {
        preferred = all
            .into_iter()
            .filter(|ch| ch.properties.contains(CharPropFlags::NOTIFY))
            .collect();
    }

    preferred
}

#[cfg(feature = "raw")]
fn is_emotiv_candidate(name: &str, services: &[Uuid], is_connected: bool) -> bool {
    let lname = name.to_ascii_lowercase();
    if lname.contains("emotiv")
        || lname.contains("epoc")
        || lname.contains("insight")
        || lname.contains("flex")
        || lname.contains("mn8")
        || lname.contains("xtrodes")
    {
        return true;
    }

    if is_connected {
        return true;
    }

    services.iter().any(|u| {
        let s = u.to_string().to_ascii_lowercase();
        s.starts_with("00001100-d102-11e1-9b23-00025b00a5a5")
            || s.starts_with("00001101-d102-11e1-9b23-00025b00a5a5")
            || s.starts_with("00001102-d102-11e1-9b23-00025b00a5a5")
    })
}

#[cfg(feature = "raw")]
fn infer_model_from_name(name: &str) -> Option<HeadsetModel> {
    let n = name.to_ascii_lowercase();
    if n.contains("insight 2") || n.contains("insight2") {
        Some(HeadsetModel::Insight2)
    } else if n.contains("insight") {
        Some(HeadsetModel::Insight)
    } else if n.contains("epoc flex") || n.contains("flex") {
        Some(HeadsetModel::EpocFlex)
    } else if n.contains("epoc+") || n.contains("epoc plus") {
        Some(HeadsetModel::EpocPlus)
    } else if n.contains("epoc x") {
        Some(HeadsetModel::EpocX)
    } else if n.contains("epoc") {
        Some(HeadsetModel::EpocStd)
    } else if n.contains("mn8") {
        Some(HeadsetModel::MN8)
    } else if n.contains("xtrodes") {
        Some(HeadsetModel::Xtrodes)
    } else {
        None
    }
}

#[cfg(feature = "raw")]
fn infer_serial(name: &str, address: &str) -> String {
    let suffix_from_name: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .rev()
        .take(12)
        .collect::<String>()
        .chars()
        .rev()
        .collect();

    if suffix_from_name.len() >= 12 {
        return suffix_from_name;
    }

    normalize_serial_12(address)
}

#[cfg(feature = "raw")]
fn normalize_serial_12(input: &str) -> String {
    let mut chars: String = input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();

    if chars.len() >= 12 {
        chars.truncate(12);
        return chars;
    }

    while chars.len() < 12 {
        chars.push('0');
    }
    chars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_discover_does_not_fail() {
        let _ = RawDevice::discover().await;
    }

    #[tokio::test]
    async fn test_model_inference() {
        #[cfg(feature = "raw")]
        {
            assert_eq!(infer_model_from_name("EMOTIV EPOC X"), Some(HeadsetModel::EpocX));
            assert_eq!(infer_model_from_name("Insight 2"), Some(HeadsetModel::Insight2));
        }
    }
}
