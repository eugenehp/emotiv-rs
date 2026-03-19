//! BLE and USB device discovery and connection handling.

use crate::raw::decryption::Decryptor;
use crate::raw::types::{DecryptedData, HeadsetModel, DeviceState};
use anyhow::{anyhow, Result};
#[cfg(feature = "raw")]
use btleplug::api::{
    Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
#[cfg(feature = "raw")]
use btleplug::platform::{Manager, Peripheral};
#[cfg(feature = "raw")]
use futures_util::StreamExt;
use std::sync::Arc;
#[cfg(feature = "raw")]
use std::collections::HashMap;
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
    pub name: String,
    pub address: String,
    pub ble_id: String,
    pub ble_mac: Option<String>,
    pub serial: String,
    pub model: HeadsetModel,
    pub transport: TransportType,
    pub battery_percent: u8,
    pub is_connected: bool,
}

/// Runtime debug statistics for raw BLE stream processing.
#[derive(Debug, Clone, Default)]
pub struct StreamDebugStats {
    pub received_notifications: u64,
    pub decoded_packets: u64,
    pub decrypt_failures: u64,
    pub timeout_count: u64,
    pub last_notify_uuid: Option<String>,
    pub last_payload_len: usize,
    pub active_serial_candidate: Option<String>,
    pub subscribed_characteristics: Vec<String>,
    pub start_command_writes: u64,
    pub active_notify_uuid: Option<String>,
}

/// Handle to a connected device for sending commands.
pub struct RawDeviceHandle {
    state: Arc<RwLock<DeviceState>>,
    command_tx: mpsc::Sender<DeviceCommand>,
    debug_stats: Arc<RwLock<StreamDebugStats>>,
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

    /// Snapshot of live BLE receive/decode stats.
    pub async fn debug_stats(&self) -> StreamDebugStats {
        self.debug_stats.read().await.clone()
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
        let debug_stats = Arc::new(RwLock::new(StreamDebugStats::default()));

        let state = Arc::clone(&self.state);
        let debug_stats_task = Arc::clone(&debug_stats);
        let info = self.info.clone();

        // Spawn connection task
        tokio::spawn(async move {
            if let Err(e) = connect_and_stream(info, tx, state, debug_stats_task).await {
                log::error!("Device streaming error: {}", e);
            }
        });

        let handle = RawDeviceHandle {
            state: Arc::clone(&self.state),
            command_tx: cmd_tx,
            debug_stats,
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
    debug_stats: Arc<RwLock<StreamDebugStats>>,
) -> Result<()> {
    #[cfg(not(feature = "raw"))]
    {
        let _ = info;
        let _ = tx;
        let _ = state;
        let _ = debug_stats;
        return Err(anyhow!("raw feature is disabled"));
    }

    #[cfg(feature = "raw")]
    {
    let mut device_state = state.write().await;
    *device_state = DeviceState::Connecting;
    drop(device_state);

    let adapter = default_adapter().await?;
    let peripheral = if let Some(p) = find_peripheral(&adapter, &info).await? {
        p
    } else {
        adapter.start_scan(ScanFilter::default()).await.ok();
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        find_peripheral(&adapter, &info)
            .await?
            .ok_or_else(|| anyhow!("BLE device not found: {}", info.address))?
    };

    if !peripheral.is_connected().await.unwrap_or(false) {
        peripheral.connect().await?;
    }
    peripheral.discover_services().await?;

    let serial_candidates = serial_candidates(&info);
    let mut decryptors = build_decryptors(info.model, &serial_candidates)?;
    if decryptors.is_empty() {
        return Err(anyhow!("No usable serial candidates for decryption"));
    }
    let mut active_decryptor_idx: Option<usize> = None;

    let notify_chars = select_notify_characteristics(&peripheral);
    if notify_chars.is_empty() {
        return Err(anyhow!("No notifiable BLE characteristics found for device"));
    }

    for ch in &notify_chars {
        if let Err(err) = peripheral.subscribe(ch).await {
            log::warn!("Failed to subscribe {}: {}", ch.uuid, err);
        }
    }

    {
        let mut s = debug_stats.write().await;
        s.subscribed_characteristics = notify_chars.iter().map(|c| c.uuid.to_string()).collect();
    }

    let writes = match send_start_stream_command(&peripheral).await {
        Ok(w) => w,
        Err(err) => {
        log::warn!("Failed to send BLE start-stream command: {}", err);
            0
        }
    };
    {
        let mut s = debug_stats.write().await;
        s.start_command_writes = writes as u64;
    }

    let mut notifications = peripheral.notifications().await?;
    let mut silence_timeouts = 0u64;

    let mut device_state = state.write().await;
    *device_state = DeviceState::Streaming;
    drop(device_state);

    let mut received_packets = 0u64;
    let mut decrypted_packets = 0u64;
    let mut active_notify_uuid: Option<Uuid> = None;
    let min_payload_len = min_payload_len_for_model(info.model);
    let expected_packet_len = expected_packet_len_for_model(info.model);
    let mut notification_buffers: HashMap<Uuid, Vec<u8>> = HashMap::new();
    let required_channels = required_channels_for_model(info.model);
    let fallback_required_channels = fallback_required_channels_for_model(info.model);
    let mut partial_hits = vec![0u8; decryptors.len()];
    loop {
        if matches!(
            *state.read().await,
            DeviceState::Disconnecting | DeviceState::Disconnected
        ) {
            break;
        }

        match tokio::time::timeout(tokio::time::Duration::from_secs(5), notifications.next()).await {
            Ok(Some(notification)) => {
                silence_timeouts = 0;
                received_packets += 1;
                let payload = notification.value;
                if payload.is_empty() || payload.len() < min_payload_len {
                    continue;
                }

                if let Some(active_uuid) = active_notify_uuid {
                    if notification.uuid != active_uuid {
                        continue;
                    }
                }

                let mut candidate_packets: Vec<Vec<u8>> = Vec::new();
                {
                    let buffer = notification_buffers.entry(notification.uuid).or_default();
                    buffer.extend_from_slice(&payload);

                    while buffer.len() >= expected_packet_len {
                        candidate_packets.push(buffer.drain(..expected_packet_len).collect());
                    }

                    if buffer.len() > expected_packet_len * 4 {
                        let keep = expected_packet_len * 2;
                        let drop_len = buffer.len().saturating_sub(keep);
                        buffer.drain(..drop_len);
                    }
                }

                if candidate_packets.is_empty() {
                    continue;
                }

                for packet in candidate_packets {
                    let mut decoded: Option<DecryptedData> = None;

                    if let Some(idx) = active_decryptor_idx {
                        if let Ok(data) = decryptors[idx].1.decrypt_eeg_packet(&packet) {
                            if data.eeg_uv.len() >= fallback_required_channels {
                                decoded = Some(data);
                            }
                        }
                    }

                    if decoded.is_none() {
                        for (idx, (_, decryptor, _)) in decryptors.iter_mut().enumerate() {
                            if Some(idx) == active_decryptor_idx {
                                continue;
                            }

                            if let Ok(data) = decryptor.decrypt_eeg_packet(&packet) {
                                let channels = data.eeg_uv.len();
                                if channels < fallback_required_channels {
                                    continue;
                                }

                                let is_partial = if channels < required_channels {
                                    partial_hits[idx] = partial_hits[idx].saturating_add(1);
                                    if partial_hits[idx] < 6 {
                                        continue;
                                    }
                                    true
                                } else {
                                    false
                                };

                                active_decryptor_idx = Some(idx);
                                log::info!(
                                    "Decryption synchronized with serial/model candidate: {}/{}{}",
                                    decryptors[idx].0,
                                    decryptors[idx].2.name(),
                                    if is_partial { " (partial mode)" } else { "" }
                                );
                                {
                                    let mut s = debug_stats.write().await;
                                    s.active_serial_candidate = Some(format!(
                                        "{}/{}{}",
                                        decryptors[idx].0,
                                        decryptors[idx].2.name(),
                                        if is_partial { ":partial" } else { "" }
                                    ));
                                }
                                decoded = Some(data);
                                break;
                            }
                        }
                    }

                    if let Some(data) = decoded {
                        decrypted_packets += 1;
                        let is_full_decode = data.eeg_uv.len() >= required_channels;
                        if active_notify_uuid.is_none() && is_full_decode {
                            active_notify_uuid = Some(notification.uuid);
                            log::info!("Locked active EEG notify UUID: {}", notification.uuid);
                            if let Some(idx) = active_decryptor_idx {
                                let mut s = debug_stats.write().await;
                                s.active_serial_candidate = Some(format!(
                                    "{}/{}",
                                    decryptors[idx].0,
                                    decryptors[idx].2.name()
                                ));
                            }
                        }

                        {
                            let mut s = debug_stats.write().await;
                            s.received_notifications = received_packets;
                            s.decoded_packets = decrypted_packets;
                            s.last_notify_uuid = Some(notification.uuid.to_string());
                            s.last_payload_len = payload.len();
                            s.active_notify_uuid = active_notify_uuid.map(|u| u.to_string());
                        }

                        if tx.send(data).await.is_err() {
                            break;
                        }
                    } else if received_packets % 50 == 0 {
                        {
                            let mut s = debug_stats.write().await;
                            s.received_notifications = received_packets;
                            s.decoded_packets = decrypted_packets;
                            s.decrypt_failures += 1;
                            s.last_notify_uuid = Some(notification.uuid.to_string());
                            s.last_payload_len = payload.len();
                            s.active_notify_uuid = active_notify_uuid.map(|u| u.to_string());
                        }
                        log::warn!(
                            "Receiving BLE notifications but cannot decrypt yet (received={}, decrypted=0). Check serial source.",
                            received_packets
                        );
                    }
                }
            }
            Ok(None) => break,
            Err(_) => {
                silence_timeouts += 1;
                {
                    let mut s = debug_stats.write().await;
                    s.received_notifications = received_packets;
                    s.decoded_packets = decrypted_packets;
                    s.timeout_count += 1;
                }

                if silence_timeouts >= 1 {
                    for ch in &notify_chars {
                        let _ = peripheral.subscribe(ch).await;
                    }
                    if let Ok(w) = send_start_stream_command(&peripheral).await {
                        let mut s = debug_stats.write().await;
                        s.start_command_writes += w as u64;
                    }
                }
                log::warn!(
                    "No BLE notifications for 5s (received={}, decrypted={}, silence_timeouts={}); re-issued subscribe/start",
                    received_packets,
                    decrypted_packets,
                    silence_timeouts,
                );
            }
        }
    }

    let _ = peripheral.disconnect().await;

    let mut device_state = state.write().await;
    *device_state = DeviceState::Disconnected;

    {
        let mut s = debug_stats.write().await;
        s.received_notifications = received_packets;
        s.decoded_packets = decrypted_packets;
    }

    Ok(())
    }
}

#[cfg(feature = "raw")]
fn required_channels_for_model(model: HeadsetModel) -> usize {
    model.channel_count()
}

#[cfg(feature = "raw")]
fn fallback_required_channels_for_model(model: HeadsetModel) -> usize {
    match model {
        HeadsetModel::Insight | HeadsetModel::Insight2 => 5,
        HeadsetModel::MN8 | HeadsetModel::Xtrodes => 8,
        HeadsetModel::EpocX
        | HeadsetModel::EpocPlus
        | HeadsetModel::EpocStd
        | HeadsetModel::EpocFlex => 10,
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

        let raw_name = props.local_name.unwrap_or_default();
        let name = if raw_name.trim().is_empty() {
            "(unknown)".to_string()
        } else {
            raw_name.clone()
        };
        let ble_id = peripheral.id().to_string();
        let mac_raw = props.address.to_string();
        let ble_mac = if is_zero_mac(&mac_raw) { None } else { Some(mac_raw) };
        let address = ble_id.clone();
        let is_connected = peripheral.is_connected().await.unwrap_or(false);
        let emotiv_candidate = is_emotiv_candidate(
            &name,
            &props.services,
            &props.manufacturer_data,
            is_connected,
        );
        let has_any_signal = !raw_name.trim().is_empty()
            || !props.services.is_empty()
            || is_connected
            || ble_mac.is_some();

        let model = infer_model_from_name(&name).unwrap_or(HeadsetModel::EpocX);
        let serial = infer_serial(&name, ble_mac.as_deref().unwrap_or(&address));
        let info = DeviceInfo {
            name,
            address,
            ble_id,
            ble_mac,
            serial,
            model,
            transport: TransportType::Ble,
            battery_percent: 0,
            is_connected,
        };

        if emotiv_candidate {
            matched_devices.push(info);
        } else {
            if has_any_signal {
                fallback_devices.push(info);
            }
        }
    }

    adapter.stop_scan().await.ok();
    if matched_devices.is_empty() {
        let allow_fallback = std::env::var("EMOTIV_RAW_ALLOW_FALLBACK")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if allow_fallback {
            log::warn!(
                "No explicit Emotiv BLE advertisements found; fallback enabled, returning all visible BLE peripherals ({})",
                fallback_devices.len()
            );
            Ok(fallback_devices)
        } else {
            log::warn!(
                "No explicit Emotiv BLE advertisements found; returning 0 devices (set EMOTIV_RAW_ALLOW_FALLBACK=1 to inspect all)",
            );
            Ok(Vec::new())
        }
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
async fn find_peripheral(
    adapter: &btleplug::platform::Adapter,
    info: &DeviceInfo,
) -> Result<Option<Peripheral>> {
    let target_id = normalize_id(&info.address);
    let target_ble_id = normalize_id(&info.ble_id);
    let target_mac = info
        .ble_mac
        .as_deref()
        .map(normalize_id)
        .unwrap_or_default();
    let target_name = normalize_id(&info.name);

    let mut scored: Vec<(i32, Peripheral)> = Vec::new();

    for p in adapter.peripherals().await? {
        let pid = p.id().to_string();
        let pid_norm = normalize_id(&pid);
        let props = p.properties().await.ok().flatten();
        let mac_norm = props
            .as_ref()
            .map(|x| normalize_id(&x.address.to_string()))
            .unwrap_or_default();
        let name_norm = props
            .as_ref()
            .and_then(|x| x.local_name.as_ref())
            .map(|n| normalize_id(n))
            .unwrap_or_default();

        let exact = (!target_id.is_empty() && pid_norm == target_id)
            || (!target_ble_id.is_empty() && pid_norm == target_ble_id)
            || (!target_mac.is_empty() && !mac_norm.is_empty() && target_mac == mac_norm)
            || (!target_name.is_empty() && !name_norm.is_empty() && target_name == name_norm);

        if exact {
            return Ok(Some(p));
        }

        let mut score = 0i32;

        if !target_name.is_empty() && !name_norm.is_empty() {
            if target_name == name_norm {
                score += 80;
            } else if name_norm.contains(&target_name) || target_name.contains(&name_norm) {
                score += 45;
            }
        }

        if !target_id.is_empty() && !pid_norm.is_empty() {
            if pid_norm.contains(&target_id) || target_id.contains(&pid_norm) {
                score += 25;
            }
        }

        if !target_ble_id.is_empty() && !pid_norm.is_empty() {
            if pid_norm.contains(&target_ble_id) || target_ble_id.contains(&pid_norm) {
                score += 20;
            }
        }

        if !target_mac.is_empty() && !mac_norm.is_empty() {
            if target_mac == mac_norm {
                score += 90;
            } else if mac_norm.contains(&target_mac) || target_mac.contains(&mac_norm) {
                score += 30;
            }
        }

        if let Some(pr) = &props {
            if pr.services.iter().any(|u| {
                let s = u.to_string().to_ascii_lowercase();
                s.starts_with("00001100-d102-11e1-9b23-00025b00a5a5")
                    || s.starts_with("00001101-d102-11e1-9b23-00025b00a5a5")
                    || s.starts_with("00001102-d102-11e1-9b23-00025b00a5a5")
            }) {
                score += 35;
            }
        }

        if p.is_connected().await.unwrap_or(false) {
            score += 20;
        }

        if score > 0 {
            scored.push((score, p));
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(scored.into_iter().next().map(|(_, p)| p))
}

#[cfg(feature = "raw")]
async fn send_start_stream_command(peripheral: &Peripheral) -> Result<usize> {
    const EMOTIV_CONTROL_SERVICE: &str = "00001101-d102-11e1-9b23-00025b00a5a5";
    let control_service_uuid = Uuid::parse_str(EMOTIV_CONTROL_SERVICE)?;

    let control_chars: Vec<_> = peripheral
        .characteristics()
        .into_iter()
        .filter(|ch| {
            ch.service_uuid == control_service_uuid
                && (ch.properties.contains(CharPropFlags::WRITE)
                    || ch
                        .properties
                        .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE))
        })
        .collect();

    let mut writes = 0usize;
    let payloads: [&[u8]; 2] = [&[0x01_u8], &[0x01_u8, 0x00_u8]];

    for ch in &control_chars {
        let write_type = if ch
            .properties
            .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
        {
            WriteType::WithoutResponse
        } else {
            WriteType::WithResponse
        };
        for payload in payloads {
            if peripheral.write(ch, payload, write_type).await.is_ok() {
                writes += 1;
            }
        }
    }

    Ok(writes)
}

#[cfg(feature = "raw")]
fn select_notify_characteristics(peripheral: &Peripheral) -> Vec<btleplug::api::Characteristic> {
    const EMOTIV_DATA_SERVICE: &str = "00001100-d102-11e1-9b23-00025b00a5a5";

    let data_uuid = Uuid::parse_str(EMOTIV_DATA_SERVICE).ok();
    let all: Vec<_> = peripheral.characteristics().into_iter().collect();

    let mut preferred: Vec<_> = all
        .iter()
        .filter(|ch| {
            (ch.properties.contains(CharPropFlags::NOTIFY)
                || ch.properties.contains(CharPropFlags::INDICATE))
                && data_uuid
                    .map(|svc| ch.service_uuid == svc)
                    .unwrap_or(false)
        })
        .cloned()
        .collect();

    if preferred.is_empty() {
        preferred = all
            .into_iter()
            .filter(|ch| {
                ch.properties.contains(CharPropFlags::NOTIFY)
                    || ch.properties.contains(CharPropFlags::INDICATE)
            })
            .filter(|ch| ch.uuid != Uuid::from_u128(0x00002a19_0000_1000_8000_00805f9b34fb))
            .collect();
    }

    preferred
}

#[cfg(feature = "raw")]
fn min_payload_len_for_model(model: HeadsetModel) -> usize {
    expected_packet_len_for_model(model).min(20)
}

#[cfg(feature = "raw")]
fn expected_packet_len_for_model(model: HeadsetModel) -> usize {
    match model {
        HeadsetModel::Insight | HeadsetModel::Insight2 | HeadsetModel::MN8 | HeadsetModel::Xtrodes => 16,
        HeadsetModel::EpocX
        | HeadsetModel::EpocPlus
        | HeadsetModel::EpocStd
        | HeadsetModel::EpocFlex
            => 32,
    }
}

#[cfg(feature = "raw")]
fn is_emotiv_candidate(
    name: &str,
    services: &[Uuid],
    manufacturer_data: &std::collections::HashMap<u16, Vec<u8>>,
    is_connected: bool,
) -> bool {
    let lname = name.to_ascii_lowercase();
    let name_match = lname.starts_with("emotiv-")
        || lname.contains("emotiv")
        || lname.starts_with("epoc")
        || lname.starts_with("insight")
        || lname.starts_with("mn8")
        || lname.starts_with("xtrodes")
        || lname.starts_with("flex");

    if name_match {
        return true;
    }

    // 0x0422 from benchmark mock advertisement (EMOTIV vendor id)
    if manufacturer_data.contains_key(&0x0422) {
        return true;
    }

    let service_match = services.iter().any(|u| {
        let s = u.to_string().to_ascii_lowercase();
        s.starts_with("00001100-d102-11e1-9b23-00025b00a5a5")
            || s.starts_with("00001101-d102-11e1-9b23-00025b00a5a5")
            || s.starts_with("00001102-d102-11e1-9b23-00025b00a5a5")
            || s.starts_with("00001103-d102-11e1-9b23-00025b00a5a5")
    });

    if service_match {
        return true;
    }

    // Connected devices are not necessarily Emotiv; do not accept by connected-only.
    let _ = is_connected;
    false
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

#[cfg(feature = "raw")]
fn normalize_id(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
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

#[cfg(feature = "raw")]
fn serial_candidates(info: &DeviceInfo) -> Vec<String> {
    let mut out = Vec::new();

    let mut push_unique = |s: String| {
        if !s.is_empty() && !out.iter().any(|x| x == &s) {
            out.push(s);
        }
    };

    push_unique(normalize_serial_12(&info.serial));
    push_unique(normalize_serial_12(&info.address));
    push_unique(normalize_serial_12(&info.ble_id));
    if let Some(mac) = &info.ble_mac {
        push_unique(normalize_serial_12(mac));
    }
    push_unique(normalize_serial_12(&info.name));

    out
}

#[cfg(feature = "raw")]
fn build_decryptors(
    model: HeadsetModel,
    serials: &[String],
) -> Result<Vec<(String, Decryptor, HeadsetModel)>> {
    let mut models = vec![model];
    let fallback_models = [
        HeadsetModel::EpocX,
        HeadsetModel::EpocPlus,
        HeadsetModel::EpocStd,
        HeadsetModel::Insight,
        HeadsetModel::Insight2,
        HeadsetModel::MN8,
        HeadsetModel::Xtrodes,
    ];
    for m in fallback_models {
        if !models.contains(&m) {
            models.push(m);
        }
    }

    let mut out = Vec::new();
    for serial in serials {
        for m in &models {
            if let Ok(decryptor) = Decryptor::new(*m, serial.clone()) {
                out.push((serial.clone(), decryptor, *m));
            }
        }
    }
    Ok(out)
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
