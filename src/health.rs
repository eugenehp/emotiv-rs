//! # Connection Health Monitor
//!
//! Background task that periodically sends a `getCortexInfo` ping to the
//! Cortex API to detect connection staleness before it causes user-visible
//! failures.
//!
//! Used internally by [`ResilientClient`](crate::reconnect::ResilientClient)
//! to trigger proactive reconnection when the health check fails.
//!
//! ## How it works
//!
//! 1. Every `interval_secs`, sends `getCortexInfo` via the stored [`crate::client::CortexHandle`].
//! 2. Waits up to `interval_secs / 2` for a [`crate::types::CortexEvent::CortexInfo`] response.
//! 3. On success: resets failure counter, emits [`HealthStatus::Healthy`].
//! 4. On timeout or send failure: increments failure counter, emits
//!    [`HealthStatus::Degraded`] or [`HealthStatus::Unhealthy`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

use crate::client::CortexHandle;
use crate::config::HealthConfig;
use crate::types::CortexEvent;

/// Signals emitted by the health monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    /// The Cortex API responded successfully.
    Healthy,

    /// A health check failed. Contains the consecutive failure count.
    Degraded { consecutive_failures: u32 },

    /// Too many consecutive failures — the connection is likely dead.
    Unhealthy { consecutive_failures: u32 },
}

/// Background health monitor that periodically checks the Cortex connection.
///
/// Polls `getCortexInfo()` at a configurable interval and emits
/// [`HealthStatus`] events via an `mpsc` channel. After
/// `max_consecutive_failures` failures, emits [`HealthStatus::Unhealthy`]
/// to signal that a reconnection should be triggered.
///
/// The monitor is stopped by calling [`HealthMonitor::stop()`] or
/// by dropping this struct.
pub struct HealthMonitor {
    handle: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

impl HealthMonitor {
    /// Start the health monitor.
    ///
    /// Returns the monitor handle and a receiver for health status events.
    /// The monitor runs until [`stop()`](Self::stop) is called.
    pub fn start(
        cortex_handle: CortexHandle,
        event_rx: broadcast::Receiver<CortexEvent>,
        config: &HealthConfig,
    ) -> (Self, mpsc::Receiver<HealthStatus>) {
        let interval = Duration::from_secs(config.interval_secs);
        let max_failures = config.max_consecutive_failures;
        let running = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::channel(16);

        let handle = {
            let running = Arc::clone(&running);
            let mut event_rx = event_rx;

            tokio::spawn(async move {
                let mut consecutive_failures: u32 = 0;

                while running.load(Ordering::SeqCst) {
                    tokio::time::sleep(interval).await;

                    if !running.load(Ordering::SeqCst) {
                        break;
                    }

                    // Send a getCortexInfo ping
                    if let Err(e) = cortex_handle.get_cortex_info().await {
                        consecutive_failures += 1;
                        tracing::warn!(
                            consecutive_failures,
                            error = %e,
                            "Health check: failed to send ping"
                        );
                        emit_status(&tx, consecutive_failures, max_failures).await;
                        continue;
                    }

                    // Wait for CortexInfo response (half the check interval as timeout)
                    let ping_timeout = interval / 2;
                    let got_response = tokio::time::timeout(ping_timeout, async {
                        loop {
                            match event_rx.recv().await {
                                Ok(CortexEvent::CortexInfo(_)) => return true,
                                Ok(_) => continue, // other event, keep waiting
                                Err(broadcast::error::RecvError::Lagged(_)) => {
                                    // Buffer overflow — keep trying
                                    continue;
                                }
                                Err(broadcast::error::RecvError::Closed) => return false,
                            }
                        }
                    })
                    .await;

                    match got_response {
                        Ok(true) => {
                            if consecutive_failures > 0 {
                                tracing::info!(
                                    previous_failures = consecutive_failures,
                                    "Health check recovered"
                                );
                            }
                            consecutive_failures = 0;
                            let _ = tx.try_send(HealthStatus::Healthy);
                        }
                        _ => {
                            consecutive_failures += 1;
                            tracing::warn!(
                                consecutive_failures,
                                "Health check: no response to ping"
                            );
                            emit_status(&tx, consecutive_failures, max_failures).await;
                        }
                    }
                }

                tracing::debug!("Health monitor stopped");
            })
        };

        (Self { handle: Some(handle), running }, rx)
    }

    /// Stop the health monitor gracefully.
    pub async fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
        }
    }

    /// Returns whether the monitor is still running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

impl Drop for HealthMonitor {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

async fn emit_status(
    tx: &mpsc::Sender<HealthStatus>,
    consecutive_failures: u32,
    max_failures: u32,
) {
    let status = if consecutive_failures >= max_failures {
        HealthStatus::Unhealthy { consecutive_failures }
    } else {
        HealthStatus::Degraded { consecutive_failures }
    };
    let _ = tx.try_send(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_status_variants() {
        let healthy = HealthStatus::Healthy;
        let degraded = HealthStatus::Degraded { consecutive_failures: 2 };
        let unhealthy = HealthStatus::Unhealthy { consecutive_failures: 5 };

        assert_eq!(healthy, HealthStatus::Healthy);
        assert_eq!(degraded, HealthStatus::Degraded { consecutive_failures: 2 });
        assert_ne!(healthy, unhealthy);
    }
}
