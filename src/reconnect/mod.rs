//! # Resilient Client
//!
//! Production-grade wrapper around [`CortexClient`] that adds automatic
//! reconnection, connection lifecycle events, and optional health monitoring.
//!
//! ## Usage
//!
//! ```no_run
//! use emotiv::config::CortexConfig;
//! use emotiv::reconnect::{ResilientClient, ConnectionEvent};
//!
//! # async fn demo() -> anyhow::Result<()> {
//! let config = CortexConfig::discover(None)?;
//! let (client, mut events) = ResilientClient::connect(config).await?;
//!
//! let mut conn_events = client.connection_event_receiver();
//! tokio::spawn(async move {
//!     while let Ok(event) = conn_events.recv().await {
//!         println!("Connection: {:?}", event);
//!     }
//! });
//!
//! while let Ok(event) = events.recv().await {
//!     println!("{:?}", event);
//! }
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::{RwLock, broadcast, mpsc};

use crate::client::{CortexClient, CortexHandle};
use crate::config::CortexConfig;
use crate::health::{HealthMonitor, HealthStatus};
use crate::types::CortexEvent;

/// Connection lifecycle events emitted by [`ResilientClient`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    /// Initially connected and ready.
    Connected,

    /// Connection was lost.
    Disconnected { reason: String },

    /// Attempting to reconnect (1-based attempt number).
    Reconnecting { attempt: u32 },

    /// Reconnected successfully. Re-subscribe to streams now (session ID changed).
    Reconnected,

    /// All reconnection attempts exhausted.
    ReconnectFailed { attempts: u32, last_error: String },
}

/// Production-grade Cortex API client with automatic reconnection
/// and optional health monitoring.
pub struct ResilientClient {
    config: CortexConfig,
    event_tx: broadcast::Sender<CortexEvent>,
    conn_event_tx: broadcast::Sender<ConnectionEvent>,
    current_handle: Arc<RwLock<Option<CortexHandle>>>,
    reconnecting: Arc<AtomicBool>,
    health_monitor: Arc<std::sync::Mutex<Option<HealthMonitor>>>,
}

impl ResilientClient {
    /// Connect to the Cortex API.
    ///
    /// Returns the `ResilientClient` and a broadcast receiver for [`CortexEvent`]s.
    pub async fn connect(config: CortexConfig) -> Result<(Self, broadcast::Receiver<CortexEvent>)> {
        let client = CortexClient::new(config.to_client_config());
        let (event_rx, handle) = client.connect().await?;

        let (event_tx, event_bcast_rx) = broadcast::channel(512);
        let (conn_event_tx, _) = broadcast::channel(64);
        let current_handle = Arc::new(RwLock::new(Some(handle.clone())));
        let reconnecting = Arc::new(AtomicBool::new(false));
        let health_monitor: Arc<std::sync::Mutex<Option<HealthMonitor>>> =
            Arc::new(std::sync::Mutex::new(None));

        let _ = conn_event_tx.send(ConnectionEvent::Connected);

        let resilient = Self {
            config,
            event_tx,
            conn_event_tx,
            current_handle,
            reconnecting,
            health_monitor,
        };

        if resilient.config.health.enabled {
            resilient.start_health_monitor_task(handle).await;
        }

        resilient.start_event_relay(event_rx);

        Ok((resilient, event_bcast_rx))
    }

    /// Subscribe to connection lifecycle events.
    pub fn connection_event_receiver(&self) -> broadcast::Receiver<ConnectionEvent> {
        self.conn_event_tx.subscribe()
    }

    /// Get an additional [`CortexEvent`] broadcast receiver.
    pub fn event_receiver(&self) -> broadcast::Receiver<CortexEvent> {
        self.event_tx.subscribe()
    }

    /// Returns `true` if a reconnection is currently in progress.
    pub fn is_reconnecting(&self) -> bool {
        self.reconnecting.load(Ordering::SeqCst)
    }

    // ─── Command delegation ──────────────────────────────────────────────────

    /// Subscribe to data streams.
    pub async fn subscribe(&self, streams: &[&str]) -> Result<()> {
        self.cloned_handle().await?.subscribe(streams).await
    }

    /// Unsubscribe from data streams.
    pub async fn unsubscribe(&self, streams: &[&str]) -> Result<()> {
        self.cloned_handle().await?.unsubscribe(streams).await
    }

    /// Create a recording.
    pub async fn create_record(&self, title: &str, description: &str) -> Result<()> {
        self.cloned_handle().await?.create_record(title, description).await
    }

    /// Stop the current recording.
    pub async fn stop_record(&self) -> Result<()> {
        self.cloned_handle().await?.stop_record().await
    }

    /// Inject a marker into the current session.
    pub async fn inject_marker(&self, time: f64, value: &str, label: &str) -> Result<()> {
        self.cloned_handle().await?.inject_marker(time, value, label).await
    }

    /// Send a training request.
    pub async fn train(&self, detection: &str, action: &str, status: &str) -> Result<()> {
        self.cloned_handle().await?.train(detection, action, status).await
    }

    /// Set up a profile.
    pub async fn setup_profile(&self, profile_name: &str, status: &str) -> Result<()> {
        self.cloned_handle().await?.setup_profile(profile_name, status).await
    }

    /// Close the current session.
    pub async fn close_session(&self) -> Result<()> {
        self.cloned_handle().await?.close_session().await
    }

    /// Get the current session ID.
    pub async fn session_id(&self) -> String {
        match self.cloned_handle().await {
            Ok(h) => h.session_id().await,
            Err(_) => String::new(),
        }
    }

    /// Get the current headset ID.
    pub async fn headset_id(&self) -> String {
        match self.cloned_handle().await {
            Ok(h) => h.headset_id().await,
            Err(_) => String::new(),
        }
    }

    /// Access the underlying [`CortexHandle`] directly for advanced use.
    pub async fn inner_handle(&self) -> Option<CortexHandle> {
        self.current_handle.read().await.clone()
    }

    // ─── Internal helpers ──────────────────────────────────────────────────

    async fn cloned_handle(&self) -> Result<CortexHandle> {
        self.current_handle
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Not connected to Cortex"))
    }

    async fn start_health_monitor_task(&self, handle: CortexHandle) {
        start_health_monitor(
            handle,
            &self.event_tx,
            &self.conn_event_tx,
            &self.health_monitor,
            &self.config,
            Arc::clone(&self.reconnecting),
        );
    }

    fn start_event_relay(&self, event_rx: mpsc::Receiver<CortexEvent>) {
        tokio::spawn(run_event_relay(
            event_rx,
            self.event_tx.clone(),
            self.conn_event_tx.clone(),
            Arc::clone(&self.current_handle),
            Arc::clone(&self.health_monitor),
            self.config.clone(),
            Arc::clone(&self.reconnecting),
        ));
    }
}

impl Drop for ResilientClient {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.health_monitor.lock() {
            drop(guard.take());
        }
    }
}

// ─── Background event relay + reconnect loop ─────────────────────────────────

async fn run_event_relay(
    mut event_rx: mpsc::Receiver<CortexEvent>,
    event_tx: broadcast::Sender<CortexEvent>,
    conn_event_tx: broadcast::Sender<ConnectionEvent>,
    current_handle: Arc<RwLock<Option<CortexHandle>>>,
    health_monitor: Arc<std::sync::Mutex<Option<HealthMonitor>>>,
    config: CortexConfig,
    reconnecting: Arc<AtomicBool>,
) {
    loop {
        // Forward events until channel closes or explicit disconnect event.
        while let Some(event) = event_rx.recv().await {
            let is_disconnect = matches!(event, CortexEvent::Disconnected);
            let _ = event_tx.send(event);
            if is_disconnect {
                break;
            }
        }

        if !config.reconnect.enabled {
            break;
        }

        reconnecting.store(true, Ordering::SeqCst);
        let _ = conn_event_tx.send(ConnectionEvent::Disconnected {
            reason: "Connection lost".into(),
        });

        // Stop health monitor before changing the connection.
        {
            if let Ok(mut guard) = health_monitor.lock() {
                drop(guard.take());
            }
        }

        let rc = &config.reconnect;
        let base = Duration::from_secs(rc.base_delay_secs);
        let max_delay = Duration::from_secs(rc.max_delay_secs);
        let max_attempts = if rc.max_attempts == 0 { u32::MAX } else { rc.max_attempts };
        let mut delay = base;
        let mut reconnected = false;

        for attempt in 1..=max_attempts {
            let _ = conn_event_tx.send(ConnectionEvent::Reconnecting { attempt });
            tracing::info!(attempt, "Reconnecting to Cortex");

            match CortexClient::new(config.to_client_config()).connect().await {
                Ok((new_rx, new_handle)) => {
                    {
                        let mut guard = current_handle.write().await;
                        *guard = Some(new_handle.clone());
                    }
                    event_rx = new_rx;
                    reconnecting.store(false, Ordering::SeqCst);
                    let _ = conn_event_tx.send(ConnectionEvent::Reconnected);
                    tracing::info!(attempt, "Reconnected successfully");

                    if config.health.enabled {
                        start_health_monitor(
                            new_handle,
                            &event_tx,
                            &conn_event_tx,
                            &health_monitor,
                            &config,
                            Arc::clone(&reconnecting),
                        );
                    }
                    reconnected = true;
                    break;
                }
                Err(e) => {
                    tracing::warn!(attempt, error = %e, "Reconnection attempt failed");
                    if attempt < max_attempts {
                        tokio::time::sleep(delay).await;
                        delay = std::cmp::min(delay * 2, max_delay);
                    }
                }
            }
        }

        if !reconnected {
            let _ = conn_event_tx.send(ConnectionEvent::ReconnectFailed {
                attempts: max_attempts,
                last_error: "All reconnection attempts exhausted".into(),
            });
            break;
        }
    }
}

/// Start a health monitor and wire its status update into connection events.
fn start_health_monitor(
    handle: CortexHandle,
    event_tx: &broadcast::Sender<CortexEvent>,
    conn_event_tx: &broadcast::Sender<ConnectionEvent>,
    slot: &Arc<std::sync::Mutex<Option<HealthMonitor>>>,
    config: &CortexConfig,
    reconnecting: Arc<AtomicBool>,
) {
    let event_rx = event_tx.subscribe();
    let (monitor, mut status_rx) = HealthMonitor::start(handle, event_rx, &config.health);

    let conn_tx = conn_event_tx.clone();
    tokio::spawn(async move {
        while let Some(status) = status_rx.recv().await {
            if let HealthStatus::Unhealthy { .. } = status {
                if !reconnecting.load(Ordering::SeqCst) {
                    tracing::warn!("Health monitor: unhealthy — triggering reconnect");
                    let _ = conn_tx.send(ConnectionEvent::Disconnected {
                        reason: "Health check failures exceeded threshold".into(),
                    });
                }
            }
        }
    });

    if let Ok(mut guard) = slot.lock() {
        *guard = Some(monitor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_event_equality() {
        assert_eq!(ConnectionEvent::Connected, ConnectionEvent::Connected);
        assert_ne!(ConnectionEvent::Connected, ConnectionEvent::Reconnected);
        assert_eq!(
            ConnectionEvent::Reconnecting { attempt: 1 },
            ConnectionEvent::Reconnecting { attempt: 1 }
        );
        assert_ne!(
            ConnectionEvent::Reconnecting { attempt: 1 },
            ConnectionEvent::Reconnecting { attempt: 2 }
        );
        assert_eq!(
            ConnectionEvent::Disconnected { reason: "x".into() },
            ConnectionEvent::Disconnected { reason: "x".into() }
        );
    }
}
