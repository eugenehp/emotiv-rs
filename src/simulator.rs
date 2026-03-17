//! Signal simulator for testing without a real Emotiv headset.
//!
//! Enabled by the `simulate` cargo feature. Generates synthetic EEG, motion,
//! performance metrics, band power, device info, and mental command data at
//! realistic rates through the same [`CortexEvent`]
//! channel used by the real client.
//!
//! No API credentials, EMOTIV Launcher, or headset hardware are needed.
//!
//! # Example
//!
//! ```no_run
//! use emotiv::simulator::{SimulatorConfig, spawn_simulator};
//! use emotiv::types::CortexEvent;
//! use tokio::sync::mpsc;
//!
//! # #[tokio::main] async fn main() {
//! let (tx, mut rx) = mpsc::channel(256);
//! spawn_simulator(SimulatorConfig::default(), tx);
//! while let Some(event) = rx.recv().await {
//!     if let CortexEvent::Eeg(data) = event {
//!         println!("{:?}", &data.samples[..5]);
//!     }
//! }
//! # }
//! ```

use std::f64::consts::PI;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

use crate::types::*;

/// Configuration for the signal simulator.
#[derive(Debug, Clone)]
pub struct SimulatorConfig {
    /// Number of EEG channels to simulate.
    pub num_eeg_channels: usize,
    /// EEG sample rate in Hz.
    pub eeg_rate_hz: f64,
    /// Whether to generate motion data.
    pub enable_motion: bool,
    /// Whether to generate performance metrics.
    pub enable_metrics: bool,
    /// Whether to generate band power data.
    pub enable_band_power: bool,
    /// Whether to generate device info data.
    pub enable_dev: bool,
    /// Whether to generate mental command data.
    pub enable_mental_command: bool,
    /// Simulated battery percentage.
    pub battery_percent: f64,
}

impl Default for SimulatorConfig {
    fn default() -> Self {
        Self {
            num_eeg_channels: 14,
            eeg_rate_hz: 128.0,
            enable_motion: true,
            enable_metrics: true,
            enable_band_power: true,
            enable_dev: true,
            enable_mental_command: true,
            battery_percent: 85.0,
        }
    }
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Generate a single synthetic EEG sample at time `t` for channel `ch`.
///
/// Superposition of alpha (10 Hz), beta (22 Hz), theta (6 Hz), and noise.
pub fn sim_eeg_sample(t: f64, ch: usize) -> f64 {
    let phi = ch as f64 * PI / 2.5;
    let alpha = 20.0 * (2.0 * PI * 10.0 * t + phi).sin();
    let beta = 6.0 * (2.0 * PI * 22.0 * t + phi * 1.7).sin();
    let theta = 10.0 * (2.0 * PI * 6.0 * t + phi * 0.9).sin();
    let nx = t * 1000.7 + ch as f64 * 137.508;
    let noise = ((nx.sin() * 9973.1).fract() - 0.5) * 8.0;
    alpha + beta + theta + noise
}

/// Generate simulated motion data at time `t`.
pub fn sim_motion(t: f64) -> Vec<f64> {
    vec![
        0.0, 0.0, // COUNTER_MEMS, INTERPOLATED_MEMS
        (2.0 * PI * 0.1 * t).sin() * 0.5,  // Q0
        (2.0 * PI * 0.15 * t).cos() * 0.3, // Q1
        (2.0 * PI * 0.2 * t).sin() * 0.2,  // Q2
        (2.0 * PI * 0.05 * t).cos() * 0.1, // Q3
        (2.0 * PI * 0.3 * t).sin() * 0.01,  // ACCX
        (2.0 * PI * 0.5 * t).cos() * 0.02,  // ACCY
        -1.0 + (2.0 * PI * 0.1 * t).sin() * 0.005, // ACCZ
        (2.0 * PI * 0.02 * t).sin() * 50.0, // MAGX
        (2.0 * PI * 0.03 * t).cos() * 30.0, // MAGY
        (2.0 * PI * 0.01 * t).sin() * 20.0, // MAGZ
    ]
}

/// Generate simulated performance metrics.
pub fn sim_metrics(t: f64) -> Vec<f64> {
    let eng = 0.5 + 0.3 * (2.0 * PI * 0.05 * t).sin();
    let exc = 0.4 + 0.2 * (2.0 * PI * 0.03 * t).cos();
    let lex = 0.3 + 0.1 * (2.0 * PI * 0.02 * t).sin();
    let str_val = 0.2 + 0.15 * (2.0 * PI * 0.04 * t).cos();
    let rel = 0.6 + 0.2 * (2.0 * PI * 0.06 * t).sin();
    let int = 0.5 + 0.25 * (2.0 * PI * 0.035 * t).cos();
    let foc = 0.55 + 0.3 * (2.0 * PI * 0.045 * t).sin();
    vec![
        1.0, eng.clamp(0.0, 1.0),
        1.0, exc.clamp(0.0, 1.0),
        lex.clamp(0.0, 1.0),
        1.0, str_val.clamp(0.0, 1.0),
        1.0, rel.clamp(0.0, 1.0),
        1.0, int.clamp(0.0, 1.0),
        1.0, foc.clamp(0.0, 1.0),
    ]
}

/// Generate simulated band power for `n_channels` channels.
pub fn sim_band_power(t: f64, n_channels: usize) -> Vec<f64> {
    let mut powers = Vec::with_capacity(n_channels * 5);
    for ch in 0..n_channels {
        let phi = ch as f64 * 0.7;
        let theta = 5.0 + 2.0 * (2.0 * PI * 0.1 * t + phi).sin();
        let alpha = 4.0 + 3.0 * (2.0 * PI * 0.08 * t + phi * 1.2).sin();
        let beta_l = 2.0 + 1.0 * (2.0 * PI * 0.12 * t + phi * 0.8).sin();
        let beta_h = 1.0 + 0.5 * (2.0 * PI * 0.15 * t + phi * 1.5).sin();
        let gamma = 0.5 + 0.3 * (2.0 * PI * 0.2 * t + phi * 2.0).sin();
        powers.extend_from_slice(&[
            theta.max(0.0), alpha.max(0.0), beta_l.max(0.0),
            beta_h.max(0.0), gamma.max(0.0),
        ]);
    }
    powers
}

/// Spawn a simulator task that sends synthetic data to the event channel.
///
/// Returns immediately. The task runs until the receiver is dropped.
pub fn spawn_simulator(config: SimulatorConfig, tx: mpsc::Sender<CortexEvent>) {
    tokio::spawn(async move {
        let _ = tx.send(CortexEvent::Connected).await;
        let _ = tx.send(CortexEvent::Authorized).await;
        let _ = tx.send(CortexEvent::SessionCreated("sim-session-001".into())).await;

        // Send data labels
        let eeg_labels: Vec<String> = (0..config.num_eeg_channels)
            .map(|i| {
                if config.num_eeg_channels == 14 {
                    crate::types::EPOC_CHANNEL_NAMES.get(i).unwrap_or(&"?").to_string()
                } else {
                    crate::types::INSIGHT_CHANNEL_NAMES.get(i).unwrap_or(&"?").to_string()
                }
            })
            .collect();
        let _ = tx.send(CortexEvent::DataLabels(DataLabels {
            stream_name: "eeg".into(),
            labels: eeg_labels,
        })).await;

        let eeg_interval = std::time::Duration::from_secs_f64(1.0 / config.eeg_rate_hz);
        let mut ticker = tokio::time::interval(eeg_interval);
        let mut t = 0.0_f64;
        let dt = 1.0 / config.eeg_rate_hz;
        let mut seq: u64 = 0;

        loop {
            ticker.tick().await;
            let now = now_secs();

            // EEG
            let samples: Vec<f64> = (0..config.num_eeg_channels)
                .map(|ch| sim_eeg_sample(t, ch))
                .collect();
            if tx.send(CortexEvent::Eeg(EegData { samples, time: now })).await.is_err() {
                break;
            }

            // Other streams at lower rates
            if seq % 16 == 0 {
                // Motion at ~8 Hz
                if config.enable_motion {
                    let _ = tx.send(CortexEvent::Motion(MotionData {
                        samples: sim_motion(t),
                        time: now,
                    })).await;
                }
            }

            if seq % 128 == 0 {
                // Metrics at ~1 Hz
                if config.enable_metrics {
                    let _ = tx.send(CortexEvent::Metrics(MetricsData {
                        values: sim_metrics(t),
                        time: now,
                    })).await;
                }

                // Band power at ~1 Hz
                if config.enable_band_power {
                    let _ = tx.send(CortexEvent::BandPower(BandPowerData {
                        powers: sim_band_power(t, config.num_eeg_channels),
                        time: now,
                    })).await;
                }

                // Device info at ~1 Hz
                if config.enable_dev {
                    let cq: Vec<f64> = (0..config.num_eeg_channels).map(|_| 4.0).collect();
                    let bat = (config.battery_percent - t / 300.0).clamp(0.0, 100.0);
                    let _ = tx.send(CortexEvent::Dev(DevData {
                        signal: 1.0,
                        contact_quality: cq,
                        battery_percent: bat,
                        time: now,
                    })).await;
                }

                // Mental command at ~1 Hz
                if config.enable_mental_command {
                    let actions = ["neutral", "push", "pull", "lift"];
                    let idx = (seq / 128) as usize % actions.len();
                    let power = 0.5 + 0.4 * (2.0 * PI * 0.1 * t).sin();
                    let _ = tx.send(CortexEvent::MentalCommand(MentalCommandData {
                        action: actions[idx].to_string(),
                        power: power.clamp(0.0, 1.0),
                        time: now,
                    })).await;
                }
            }

            t += dt;
            seq += 1;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sim_eeg_sample_range() {
        // EEG samples should be roughly in ±50 µV range
        for ch in 0..14 {
            for i in 0..1000 {
                let t = i as f64 / 128.0;
                let v = sim_eeg_sample(t, ch);
                assert!(v.abs() < 100.0, "EEG sample out of range: {v} at t={t}, ch={ch}");
            }
        }
    }

    #[test]
    fn test_sim_eeg_varies_by_channel() {
        let t = 1.0;
        let s0 = sim_eeg_sample(t, 0);
        let s1 = sim_eeg_sample(t, 1);
        assert!((s0 - s1).abs() > 0.001, "Different channels should produce different values");
    }

    #[test]
    fn test_sim_motion_length() {
        let m = sim_motion(1.0);
        assert_eq!(m.len(), 12);
    }

    #[test]
    fn test_sim_metrics_length() {
        let m = sim_metrics(1.0);
        assert_eq!(m.len(), 13);
    }

    #[test]
    fn test_sim_metrics_range() {
        for i in 0..100 {
            let t = i as f64 * 0.5;
            let m = sim_metrics(t);
            for (idx, &v) in m.iter().enumerate() {
                assert!(v >= 0.0 && v <= 1.0, "Metric {idx} out of range: {v} at t={t}");
            }
        }
    }

    #[test]
    fn test_sim_band_power_length() {
        let bp = sim_band_power(1.0, 14);
        assert_eq!(bp.len(), 14 * 5);
        let bp5 = sim_band_power(1.0, 5);
        assert_eq!(bp5.len(), 5 * 5);
    }

    #[test]
    fn test_sim_band_power_positive() {
        for i in 0..100 {
            let t = i as f64 * 0.1;
            let bp = sim_band_power(t, 14);
            for (idx, &v) in bp.iter().enumerate() {
                assert!(v >= 0.0, "Band power {idx} negative: {v} at t={t}");
            }
        }
    }

    #[tokio::test]
    async fn test_simulator_produces_events() {
        let (tx, mut rx) = mpsc::channel(256);
        let config = SimulatorConfig {
            num_eeg_channels: 5,
            eeg_rate_hz: 128.0,
            ..Default::default()
        };
        spawn_simulator(config, tx);

        let mut eeg_count = 0;
        let mut connected = false;
        let mut authorized = false;
        let mut session_created = false;

        let timeout = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while let Some(event) = rx.recv().await {
                match event {
                    CortexEvent::Connected => connected = true,
                    CortexEvent::Authorized => authorized = true,
                    CortexEvent::SessionCreated(_) => session_created = true,
                    CortexEvent::Eeg(data) => {
                        assert_eq!(data.samples.len(), 5);
                        eeg_count += 1;
                        if eeg_count >= 50 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        });

        let _ = timeout.await;
        assert!(connected);
        assert!(authorized);
        assert!(session_created);
        assert!(eeg_count >= 10, "Expected >=10 EEG events, got {eeg_count}");
    }
}
