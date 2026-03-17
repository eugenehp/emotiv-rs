//! All data types produced and consumed by the Emotiv Cortex client.
//!
//! The central type is [`CortexEvent`] — an enum covering every kind of message
//! the client can emit, from EEG samples to record lifecycle events. Consumers
//! receive these through the `mpsc::Receiver<CortexEvent>` returned by
//! [`CortexClient::connect`](crate::client::CortexClient::connect).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Stream data types ─────────────────────────────────────────────────────────

/// EEG data from a single notification.
///
/// Channel labels depend on the headset model. For EPOC X (14-channel):
/// AF3, F7, F3, FC5, T7, P7, O1, O2, P8, T8, FC6, F4, F8, AF4
///
/// For EPOC+ (14-channel) and Insight (5-channel): AF3, AF4, T7, T8, Pz
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EegData {
    /// Voltage samples per channel in µV.
    pub samples: Vec<f64>,
    /// Timestamp in seconds since epoch.
    pub time: f64,
}

/// Motion / IMU data (accelerometer + gyroscope + magnetometer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotionData {
    /// Raw motion values: [COUNTER_MEMS, INTERPOLATED_MEMS, Q0, Q1, Q2, Q3, ACCX, ACCY, ACCZ, MAGX, MAGY, MAGZ]
    pub samples: Vec<f64>,
    /// Timestamp in seconds since epoch.
    pub time: f64,
}

/// Device information data (contact quality per channel + battery).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevData {
    /// Overall signal quality (0-4 scale).
    pub signal: f64,
    /// Contact quality per channel (0=no contact, 4=good).
    pub contact_quality: Vec<f64>,
    /// Battery percentage (0-100).
    pub battery_percent: f64,
    /// Timestamp in seconds since epoch.
    pub time: f64,
}

/// Performance metrics data (engagement, excitement, stress, relaxation, interest, focus).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsData {
    /// Raw metrics array: [eng.isActive, eng, exc.isActive, exc, lex, str.isActive, str, rel.isActive, rel, int.isActive, int, foc.isActive, foc]
    pub values: Vec<f64>,
    /// Timestamp in seconds since epoch.
    pub time: f64,
}

/// Band power data (theta, alpha, betaL, betaH, gamma per channel).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandPowerData {
    /// Power values: [ch1/theta, ch1/alpha, ch1/betaL, ch1/betaH, ch1/gamma, ch2/theta, ...]
    pub powers: Vec<f64>,
    /// Timestamp in seconds since epoch.
    pub time: f64,
}

/// Mental command data (BCI output after training).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MentalCommandData {
    /// The detected action (e.g., "neutral", "push", "pull").
    pub action: String,
    /// Power/confidence of the detection (0.0 - 1.0).
    pub power: f64,
    /// Timestamp in seconds since epoch.
    pub time: f64,
}

/// Facial expression data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacialExpressionData {
    /// Eye action (e.g., "blink", "winkL", "winkR").
    pub eye_action: String,
    /// Upper face action.
    pub upper_action: String,
    /// Upper face action power.
    pub upper_power: f64,
    /// Lower face action.
    pub lower_action: String,
    /// Lower face action power.
    pub lower_power: f64,
    /// Timestamp in seconds since epoch.
    pub time: f64,
}

/// System / training events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SysData {
    /// Raw system event data.
    pub events: Vec<serde_json::Value>,
}

/// Data stream labels received after subscribing.
#[derive(Debug, Clone)]
pub struct DataLabels {
    /// Stream name (e.g., "eeg", "mot", "dev", "met", "pow").
    pub stream_name: String,
    /// Column labels for the stream.
    pub labels: Vec<String>,
}

// ── Record types ──────────────────────────────────────────────────────────────

/// A recording record returned by the Cortex API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub uuid: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "startDatetime")]
    pub start_datetime: String,
    #[serde(default, rename = "endDatetime")]
    pub end_datetime: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "licenseId")]
    pub license_id: String,
    #[serde(default, rename = "applicationId")]
    pub application_id: String,
    /// Flattened extra fields.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// A marker within a recording.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marker {
    pub uuid: String,
    #[serde(default, rename = "type")]
    pub marker_type: String,
    #[serde(default)]
    pub value: serde_json::Value,
    #[serde(default)]
    pub label: String,
    #[serde(default, rename = "startDatetime")]
    pub start_datetime: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// A headset discovered by the Cortex service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadsetInfo {
    pub id: String,
    pub status: String,
    #[serde(default, rename = "connectedBy")]
    pub connected_by: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Profile information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub name: String,
    #[serde(default, rename = "readOnly")]
    pub read_only: bool,
}

// ── Events ────────────────────────────────────────────────────────────────────

/// All events emitted by [`crate::client::CortexClient`].
///
/// Consumers receive these through the `mpsc::Receiver` returned by the client.
#[derive(Debug, Clone)]
pub enum CortexEvent {
    // ── Connection lifecycle ──────────────────────────────────────────────
    /// WebSocket connection established.
    Connected,
    /// Authorization completed; cortex token obtained.
    Authorized,
    /// A session was created with a headset.
    SessionCreated(String),
    /// WebSocket disconnected.
    Disconnected,
    /// An error occurred.
    Error(String),

    // ── Stream data ──────────────────────────────────────────────────────
    /// New EEG data.
    Eeg(EegData),
    /// New motion data.
    Motion(MotionData),
    /// New device info data.
    Dev(DevData),
    /// New performance metrics data.
    Metrics(MetricsData),
    /// New band power data.
    BandPower(BandPowerData),
    /// New mental command data.
    MentalCommand(MentalCommandData),
    /// New facial expression data.
    FacialExpression(FacialExpressionData),
    /// New system/training event.
    Sys(SysData),
    /// Data labels for a subscribed stream.
    DataLabels(DataLabels),
    /// Cortex service info (response to getCortexInfo health ping).
    CortexInfo(serde_json::Value),

    // ── Records & markers ────────────────────────────────────────────────
    /// Record created.
    RecordCreated(Record),
    /// Record stopped.
    RecordStopped(Record),
    /// Record export completed.
    RecordExported(Vec<String>),
    /// Record post-processing done (can export now).
    RecordPostProcessingDone(String),
    /// Query records result.
    QueryRecordsDone {
        records: Vec<Record>,
        count: u64,
    },
    /// Download records result.
    DownloadRecordsDone(serde_json::Value),

    // ── Markers ──────────────────────────────────────────────────────────
    /// Marker injected.
    MarkerInjected(Marker),
    /// Marker updated.
    MarkerUpdated(Marker),

    // ── Profiles ─────────────────────────────────────────────────────────
    /// Profile list received.
    ProfilesQueried(Vec<String>),
    /// Profile loaded or unloaded.
    ProfileLoaded(bool),
    /// Profile saved.
    ProfileSaved,

    // ── BCI ──────────────────────────────────────────────────────────────
    /// Active mental command actions.
    McActiveActions(serde_json::Value),
    /// Mental command sensitivity values.
    McSensitivity(serde_json::Value),
    /// Mental command brain map.
    McBrainMap(serde_json::Value),
    /// Mental command training threshold.
    McTrainingThreshold(serde_json::Value),

    // ── Warnings ─────────────────────────────────────────────────────────
    /// A warning from the Cortex service.
    Warning { code: i64, message: serde_json::Value },
    /// Headset clock sync done.
    HeadsetClockSynced(serde_json::Value),
}

// ── Headset channel names ─────────────────────────────────────────────────────

/// EPOC X / EPOC+ 14-channel EEG electrode names.
pub const EPOC_CHANNEL_NAMES: [&str; 14] = [
    "AF3", "F7", "F3", "FC5", "T7", "P7", "O1",
    "O2", "P8", "T8", "FC6", "F4", "F8", "AF4",
];

/// Insight 5-channel EEG electrode names.
pub const INSIGHT_CHANNEL_NAMES: [&str; 5] = ["AF3", "AF4", "T7", "T8", "Pz"];

/// Performance metric labels.
pub const METRIC_LABELS: [&str; 13] = [
    "eng.isActive", "eng", "exc.isActive", "exc", "lex",
    "str.isActive", "str", "rel.isActive", "rel",
    "int.isActive", "int", "foc.isActive", "foc",
];
