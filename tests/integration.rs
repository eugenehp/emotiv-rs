//! Integration tests — exercises the public API surface as a library consumer would.

use emotiv::prelude::*;
#[cfg(feature = "simulate")]
use emotiv::simulator::{SimulatorConfig, spawn_simulator};
#[cfg(feature = "simulate")]
use tokio::sync::mpsc;

mod common;
use common::mock_cortex::spawn_mock_cortex_server;

// ── Simulator integration ─────────────────────────────────────────────────────

#[cfg(feature = "simulate")]
#[tokio::test]
async fn simulator_14ch_produces_all_event_types() {
    let (tx, mut rx) = mpsc::channel(512);
    spawn_simulator(SimulatorConfig::default(), tx);

    let mut seen_connected = false;
    let mut seen_authorized = false;
    let mut seen_session = false;
    let mut seen_labels = false;
    let mut seen_eeg = false;
    let mut seen_motion = false;
    let mut seen_metrics = false;
    let mut seen_band_power = false;
    let mut seen_dev = false;
    let mut seen_mc = false;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            ev = rx.recv() => {
                match ev {
                    Some(CortexEvent::Connected) => seen_connected = true,
                    Some(CortexEvent::Authorized) => seen_authorized = true,
                    Some(CortexEvent::SessionCreated(_)) => seen_session = true,
                    Some(CortexEvent::DataLabels(_)) => seen_labels = true,
                    Some(CortexEvent::Eeg(d)) => {
                        assert_eq!(d.samples.len(), 14);
                        seen_eeg = true;
                    }
                    Some(CortexEvent::Motion(d)) => {
                        assert_eq!(d.samples.len(), 12);
                        seen_motion = true;
                    }
                    Some(CortexEvent::Metrics(d)) => {
                        assert_eq!(d.values.len(), 13);
                        seen_metrics = true;
                    }
                    Some(CortexEvent::BandPower(d)) => {
                        assert_eq!(d.powers.len(), 14 * 5);
                        seen_band_power = true;
                    }
                    Some(CortexEvent::Dev(d)) => {
                        assert_eq!(d.contact_quality.len(), 14);
                        seen_dev = true;
                    }
                    Some(CortexEvent::MentalCommand(d)) => {
                        assert!(!d.action.is_empty());
                        assert!(d.power >= 0.0 && d.power <= 1.0);
                        seen_mc = true;
                    }
                    None => break,
                    _ => {}
                }
                if seen_connected && seen_authorized && seen_session && seen_labels
                    && seen_eeg && seen_motion && seen_metrics && seen_band_power
                    && seen_dev && seen_mc
                {
                    break;
                }
            }
        }
    }

    assert!(seen_connected, "Missing Connected event");
    assert!(seen_authorized, "Missing Authorized event");
    assert!(seen_session, "Missing SessionCreated event");
    assert!(seen_labels, "Missing DataLabels event");
    assert!(seen_eeg, "Missing Eeg event");
    assert!(seen_motion, "Missing Motion event");
    assert!(seen_metrics, "Missing Metrics event");
    assert!(seen_band_power, "Missing BandPower event");
    assert!(seen_dev, "Missing Dev event");
    assert!(seen_mc, "Missing MentalCommand event");
}

#[cfg(feature = "simulate")]
#[tokio::test]
async fn simulator_5ch_insight_mode() {
    let (tx, mut rx) = mpsc::channel(512);
    spawn_simulator(SimulatorConfig {
        num_eeg_channels: 5,
        ..Default::default()
    }, tx);

    let mut eeg_count = 0;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            ev = rx.recv() => {
                match ev {
                    Some(CortexEvent::Eeg(d)) => {
                        assert_eq!(d.samples.len(), 5, "Insight mode should have 5 channels");
                        eeg_count += 1;
                        if eeg_count >= 50 { break; }
                    }
                    Some(CortexEvent::BandPower(d)) => {
                        assert_eq!(d.powers.len(), 5 * 5, "Band power should be 5ch × 5bands");
                    }
                    Some(CortexEvent::Dev(d)) => {
                        assert_eq!(d.contact_quality.len(), 5);
                    }
                    None => break,
                    _ => {}
                }
            }
        }
    }
    assert!(eeg_count >= 50, "Expected >=50 EEG events, got {eeg_count}");
}

#[cfg(feature = "simulate")]
#[tokio::test]
async fn simulator_disabled_streams() {
    let (tx, mut rx) = mpsc::channel(512);
    spawn_simulator(SimulatorConfig {
        num_eeg_channels: 5,
        enable_motion: false,
        enable_metrics: false,
        enable_band_power: false,
        enable_dev: false,
        enable_mental_command: false,
        ..Default::default()
    }, tx);

    let mut eeg_count = 0;
    let mut motion_count = 0;
    let mut metrics_count = 0;
    let mut bp_count = 0;
    let mut dev_count = 0;
    let mut mc_count = 0;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            ev = rx.recv() => {
                match ev {
                    Some(CortexEvent::Eeg(_)) => eeg_count += 1,
                    Some(CortexEvent::Motion(_)) => motion_count += 1,
                    Some(CortexEvent::Metrics(_)) => metrics_count += 1,
                    Some(CortexEvent::BandPower(_)) => bp_count += 1,
                    Some(CortexEvent::Dev(_)) => dev_count += 1,
                    Some(CortexEvent::MentalCommand(_)) => mc_count += 1,
                    None => break,
                    _ => {}
                }
            }
        }
    }

    assert!(eeg_count > 0, "EEG should still be produced");
    assert_eq!(motion_count, 0, "Motion should be disabled");
    assert_eq!(metrics_count, 0, "Metrics should be disabled");
    assert_eq!(bp_count, 0, "BandPower should be disabled");
    assert_eq!(dev_count, 0, "Dev should be disabled");
    assert_eq!(mc_count, 0, "MentalCommand should be disabled");
}

#[cfg(feature = "simulate")]
#[tokio::test]
async fn simulator_timestamps_increase() {
    let (tx, mut rx) = mpsc::channel(512);
    spawn_simulator(SimulatorConfig {
        num_eeg_channels: 5,
        enable_motion: false,
        enable_metrics: false,
        enable_band_power: false,
        enable_dev: false,
        enable_mental_command: false,
        ..Default::default()
    }, tx);

    let mut last_time = 0.0_f64;
    let mut count = 0;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            ev = rx.recv() => {
                match ev {
                    Some(CortexEvent::Eeg(d)) => {
                        assert!(d.time >= last_time, "Timestamps must not decrease: {} < {}", d.time, last_time);
                        last_time = d.time;
                        count += 1;
                        if count >= 200 { break; }
                    }
                    None => break,
                    _ => {}
                }
            }
        }
    }
    assert!(count >= 100);
}

// ── Type serde round-trip ─────────────────────────────────────────────────────

#[test]
fn record_deserialize_from_cortex_json() {
    let json = serde_json::json!({
        "uuid": "a1b2c3d4",
        "title": "My Recording",
        "startDatetime": "2026-01-15T10:30:00.000Z",
        "endDatetime": "2026-01-15T10:31:00.000Z",
        "description": "test desc",
        "licenseId": "lic-001",
        "applicationId": "app-001",
        "tags": ["eeg", "training"],
        "syncStatus": {"status": "downloaded"}
    });
    let rec: Record = serde_json::from_value(json).unwrap();
    assert_eq!(rec.uuid, "a1b2c3d4");
    assert_eq!(rec.title, "My Recording");
    assert_eq!(rec.start_datetime, "2026-01-15T10:30:00.000Z");
    assert_eq!(rec.end_datetime, "2026-01-15T10:31:00.000Z");
    assert_eq!(rec.description, "test desc");
    assert_eq!(rec.license_id, "lic-001");
    assert_eq!(rec.application_id, "app-001");
    // Extra fields captured in the HashMap
    assert!(rec.extra.contains_key("tags"));
    assert!(rec.extra.contains_key("syncStatus"));
}

#[test]
fn record_deserialize_minimal() {
    let json = serde_json::json!({"uuid": "x"});
    let rec: Record = serde_json::from_value(json).unwrap();
    assert_eq!(rec.uuid, "x");
    assert_eq!(rec.title, "");
    assert_eq!(rec.start_datetime, "");
}

#[test]
fn marker_deserialize_from_cortex_json() {
    let json = serde_json::json!({
        "uuid": "mk-001",
        "type": "instance",
        "value": "stimulus_A",
        "label": "trial_1",
        "startDatetime": "2026-01-15T10:30:05.123Z",
        "port": "python_app"
    });
    let m: Marker = serde_json::from_value(json).unwrap();
    assert_eq!(m.uuid, "mk-001");
    assert_eq!(m.marker_type, "instance");
    assert_eq!(m.label, "trial_1");
    assert_eq!(m.value, "stimulus_A");
    assert!(m.extra.contains_key("port"));
}

#[tokio::test]
async fn client_can_run_against_mock_server() {
    let ws_url = spawn_mock_cortex_server(false).await;

    let client = CortexClient::new(CortexClientConfig {
        client_id: "mock-id".into(),
        client_secret: "mock-secret".into(),
        ws_url,
        ..Default::default()
    });

    let (mut rx, handle) = client.connect().await.unwrap();

    let mut seen_authorized = false;
    let mut seen_session = false;
    let mut seen_labels = false;
    let mut seen_eeg = false;
    let mut seen_cortex_info = false;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            ev = rx.recv() => {
                match ev {
                    Some(CortexEvent::Authorized) => seen_authorized = true,
                    Some(CortexEvent::SessionCreated(_)) => {
                        seen_session = true;
                        handle.subscribe(&["eeg"]).await.unwrap();
                        handle.get_cortex_info().await.unwrap();
                    }
                    Some(CortexEvent::DataLabels(_)) => seen_labels = true,
                    Some(CortexEvent::Eeg(_)) => seen_eeg = true,
                    Some(CortexEvent::CortexInfo(_)) => {
                        seen_cortex_info = true;
                        break;
                    }
                    Some(_) => {}
                    None => break,
                }
            }
        }
    }

    assert!(seen_authorized, "mock flow should authorize");
    assert!(seen_session, "mock flow should create session");
    assert!(seen_labels, "mock flow should emit stream labels");
    assert!(seen_eeg, "mock flow should emit eeg data");
    assert!(seen_cortex_info, "mock flow should emit CortexInfo response");
}

#[tokio::test]
async fn resilient_client_reconnects_with_mock_server() {
    let ws_url = spawn_mock_cortex_server(true).await;

    let config = CortexConfig {
        client_id: "mock-id".into(),
        client_secret: "mock-secret".into(),
        cortex_url: ws_url,
        auto_create_session: true,
        reconnect: emotiv::config::ReconnectConfig {
            enabled: true,
            base_delay_secs: 0,
            max_delay_secs: 1,
            max_attempts: 3,
        },
        health: emotiv::config::HealthConfig {
            enabled: false,
            interval_secs: 30,
            max_consecutive_failures: 3,
        },
        ..CortexConfig::new("mock-id", "mock-secret")
    };

    let (client, mut events) = ResilientClient::connect(config).await.unwrap();
    let mut conn_events = client.connection_event_receiver();

    let mut saw_reconnecting = false;
    let mut saw_reconnected = false;
    let mut saw_second_session = false;
    let mut session_count = 0_u32;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            Ok(conn_ev) = conn_events.recv() => {
                match conn_ev {
                    emotiv::reconnect::ConnectionEvent::Reconnecting { .. } => saw_reconnecting = true,
                    emotiv::reconnect::ConnectionEvent::Reconnected => saw_reconnected = true,
                    _ => {}
                }
            }
            Ok(ev) = events.recv() => {
                if let CortexEvent::SessionCreated(_) = ev {
                    session_count += 1;
                    if session_count >= 2 {
                        saw_second_session = true;
                        break;
                    }
                }
            }
        }
    }

    assert!(saw_reconnecting, "should emit reconnecting event");
    assert!(saw_reconnected, "should emit reconnected event");
    assert!(saw_second_session, "should establish a second session after reconnect");
}

#[test]
fn headset_info_deserialize() {
    let json = serde_json::json!({
        "id": "EPOCX-ABCDEF12",
        "status": "connected",
        "connectedBy": "dongle",
        "firmware": "3.1.2",
        "sensors": {"AF3": 4, "AF4": 4}
    });
    let hs: HeadsetInfo = serde_json::from_value(json).unwrap();
    assert_eq!(hs.id, "EPOCX-ABCDEF12");
    assert_eq!(hs.status, "connected");
    assert_eq!(hs.connected_by, "dongle");
    assert!(hs.extra.contains_key("firmware"));
    assert!(hs.extra.contains_key("sensors"));
}

#[test]
fn eeg_data_serialize_roundtrip() {
    let data = EegData {
        samples: vec![1.5, -2.3, 0.0, 100.0],
        time: 1234567890.123,
    };
    let json = serde_json::to_value(&data).unwrap();
    let back: EegData = serde_json::from_value(json).unwrap();
    assert_eq!(back.samples, data.samples);
    assert!((back.time - data.time).abs() < 1e-6);
}

#[test]
fn mental_command_data_serialize_roundtrip() {
    let data = MentalCommandData {
        action: "push".into(),
        power: 0.85,
        time: 999.0,
    };
    let json = serde_json::to_value(&data).unwrap();
    let back: MentalCommandData = serde_json::from_value(json).unwrap();
    assert_eq!(back.action, "push");
    assert!((back.power - 0.85).abs() < 1e-6);
}

#[test]
fn facial_expression_data_serialize_roundtrip() {
    let data = FacialExpressionData {
        eye_action: "blink".into(),
        upper_action: "surprise".into(),
        upper_power: 0.7,
        lower_action: "smile".into(),
        lower_power: 0.9,
        time: 100.0,
    };
    let json = serde_json::to_value(&data).unwrap();
    let back: FacialExpressionData = serde_json::from_value(json).unwrap();
    assert_eq!(back.eye_action, "blink");
    assert_eq!(back.lower_action, "smile");
    assert!((back.lower_power - 0.9).abs() < 1e-6);
}

// ── Edge cases ────────────────────────────────────────────────────────────────

#[test]
fn channel_name_constants() {
    assert_eq!(EPOC_CHANNEL_NAMES.len(), 14);
    assert_eq!(INSIGHT_CHANNEL_NAMES.len(), 5);
    assert_eq!(METRIC_LABELS.len(), 13);
    assert_eq!(EPOC_CHANNEL_NAMES[0], "AF3");
    assert_eq!(EPOC_CHANNEL_NAMES[13], "AF4");
    assert_eq!(INSIGHT_CHANNEL_NAMES[4], "Pz");
}

#[test]
fn stream_constants() {
    assert_eq!(STREAM_EEG, "eeg");
    assert_eq!(STREAM_MOT, "mot");
    assert_eq!(STREAM_DEV, "dev");
    assert_eq!(STREAM_MET, "met");
    assert_eq!(STREAM_POW, "pow");
    assert_eq!(STREAM_COM, "com");
    assert_eq!(STREAM_FAC, "fac");
    assert_eq!(STREAM_SYS, "sys");
}

#[test]
fn eeg_frequency_constant() {
    assert_eq!(EEG_FREQUENCY, 128.0);
}

#[test]
fn record_with_empty_extra_fields() {
    let json = serde_json::json!({
        "uuid": "r1",
        "title": "T",
        "startDatetime": "",
        "endDatetime": "",
        "description": "",
        "licenseId": "",
        "applicationId": ""
    });
    let rec: Record = serde_json::from_value(json).unwrap();
    assert!(rec.extra.is_empty());
}

#[test]
fn marker_with_numeric_value() {
    let json = serde_json::json!({
        "uuid": "mk",
        "type": "interval",
        "value": 42,
        "label": "num_marker",
        "startDatetime": ""
    });
    let m: Marker = serde_json::from_value(json).unwrap();
    assert_eq!(m.value, 42);
}

#[test]
fn marker_with_object_value() {
    let json = serde_json::json!({
        "uuid": "mk",
        "type": "interval",
        "value": {"key": "val"},
        "label": "obj_marker",
        "startDatetime": ""
    });
    let m: Marker = serde_json::from_value(json).unwrap();
    assert!(m.value.is_object());
}
