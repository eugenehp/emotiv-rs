//! Data types for raw device streaming.

use serde::{Deserialize, Serialize};

/// Emotiv headset model with BLE or USB transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeadsetModel {
    EpocX,
    EpocPlus,
    EpocFlex,
    EpocStd,
    Insight,
    Insight2,
    MN8,
    Xtrodes,
}

impl std::fmt::Display for HeadsetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl HeadsetModel {
    /// Create from model string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "EPOC X" | "epoc_x" | "EPOC_X" => Some(HeadsetModel::EpocX),
            "EPOC+" | "epoc_plus" | "EPOC_PLUS" => Some(HeadsetModel::EpocPlus),
            "EPOC Flex" | "epoc_flex" | "EPOC_FLEX" => Some(HeadsetModel::EpocFlex),
            "EPOC" | "epoc_std" | "EPOC_STD" => Some(HeadsetModel::EpocStd),
            "Insight" | "insight" => Some(HeadsetModel::Insight),
            "Insight 2" | "insight_2" | "INSIGHT_2" => Some(HeadsetModel::Insight2),
            "MN8" | "mn8" => Some(HeadsetModel::MN8),
            "Xtrodes" | "xtrodes" => Some(HeadsetModel::Xtrodes),
            _ => None,
        }
    }

    /// Human-readable name.
    pub fn name(&self) -> &str {
        match self {
            HeadsetModel::EpocX => "EPOC X",
            HeadsetModel::EpocPlus => "EPOC+",
            HeadsetModel::EpocFlex => "EPOC Flex",
            HeadsetModel::EpocStd => "EPOC",
            HeadsetModel::Insight => "Insight",
            HeadsetModel::Insight2 => "Insight 2",
            HeadsetModel::MN8 => "MN8",
            HeadsetModel::Xtrodes => "Xtrodes",
        }
    }

    /// Number of EEG channels.
    pub fn channel_count(&self) -> usize {
        match self {
            HeadsetModel::EpocX | HeadsetModel::EpocPlus | HeadsetModel::EpocStd => 14,
            HeadsetModel::EpocFlex => 32,
            HeadsetModel::Insight | HeadsetModel::Insight2 => 5,
            HeadsetModel::MN8 => 8,
            HeadsetModel::Xtrodes => 8,
        }
    }

    /// EEG channel names.
    pub fn channels(&self) -> Vec<&'static str> {
        match self {
            HeadsetModel::EpocX | HeadsetModel::EpocPlus | HeadsetModel::EpocStd => vec![
                "AF3", "F7", "F3", "FC5", "T7", "P7", "O1", "O2", "P8", "T8", "FC6", "F4", "F8", "AF4",
            ],
            HeadsetModel::EpocFlex => vec![
                "Fp1", "Fpz", "Fp2", "AF7", "AF3", "AFz", "AF4", "AF8",
                "F7", "F5", "F3", "F1", "Fz", "F2", "F4", "F6", "F8",
                "FT7", "FC5", "FC3", "FC1", "FCz", "FC2", "FC4", "FC6", "FT8",
                "T7", "C5", "C3", "C1", "Cz", "C2", "C4", "C6", "T8",
            ],
            HeadsetModel::Insight | HeadsetModel::Insight2 => {
                vec!["AF3", "AF4", "T7", "T8", "Pz"]
            }
            _ => vec![],
        }
    }

    /// Sampling rate (Hz).
    pub fn sampling_rate(&self) -> u32 {
        match self {
            HeadsetModel::EpocX | HeadsetModel::EpocPlus | HeadsetModel::EpocStd => 128,
            HeadsetModel::EpocFlex => 256,
            HeadsetModel::Insight | HeadsetModel::Insight2 => 128,
            HeadsetModel::MN8 => 128,
            HeadsetModel::Xtrodes => 250,
        }
    }

    /// Physical voltage range in microvolts.
    pub fn eeg_physical_range(&self) -> (f64, f64) {
        match self {
            HeadsetModel::EpocX | HeadsetModel::EpocPlus | HeadsetModel::EpocStd => {
                (-8399.0, 8399.0)
            }
            HeadsetModel::EpocFlex => (-8399.0, 8399.0),
            HeadsetModel::Insight | HeadsetModel::Insight2 => (-8192.0, 8192.0),
            HeadsetModel::MN8 => (-8192.0, 8192.0),
            HeadsetModel::Xtrodes => (-32767.0, 32767.0),
        }
    }
}

/// Raw decrypted EEG/motion packet data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecryptedData {
    /// Counter (packet sequence number).
    pub counter: u32,
    /// EEG samples in microvolts per channel.
    pub eeg_uv: Vec<f64>,
    /// Raw ADC values (before conversion).
    pub eeg_adc: Vec<i32>,
    /// Contact quality per channel (0-4 scale, 4=best).
    pub contact_quality: Vec<u8>,
    /// Overall signal quality (0-4).
    pub signal_quality: u8,
    /// Motion data: [counter, q0, q1, q2, q3, accel_x, accel_y, accel_z, mag_x, mag_y, mag_z].
    pub motion: Option<Vec<f64>>,
    /// Battery percentage (0-100).
    pub battery_percent: u8,
    /// Timestamp (Unix seconds with microsecond precision).
    pub timestamp: f64,
    /// Packet receive time.
    pub receive_time: f64,
}

impl DecryptedData {
    /// Create new decrypted data.
    pub fn new(
        counter: u32,
        eeg_uv: Vec<f64>,
        eeg_adc: Vec<i32>,
        contact_quality: Vec<u8>,
        signal_quality: u8,
        battery_percent: u8,
    ) -> Self {
        Self {
            counter,
            eeg_uv,
            eeg_adc,
            contact_quality,
            signal_quality,
            motion: None,
            battery_percent,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
            receive_time: 0.0,
        }
    }
}

/// Battery info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryInfo {
    pub percent: u8,
    pub voltage_mv: u16,
    pub is_low: bool,
}

/// Device connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    Disconnected,
    Connecting,
    Connected,
    Streaming,
    Disconnecting,
}

impl std::fmt::Display for DeviceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Connecting => write!(f, "Connecting"),
            Self::Connected => write!(f, "Connected"),
            Self::Streaming => write!(f, "Streaming"),
            Self::Disconnecting => write!(f, "Disconnecting"),
        }
    }
}
