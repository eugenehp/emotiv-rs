//! Cortex WebSocket client for Emotiv headsets.
//!
//! Handles the full lifecycle of a Cortex API session:
//!
//! 1. **Connect** to `wss://localhost:6868` (the local Cortex service shipped with EMOTIV Launcher).
//! 2. **Authenticate** using your Client ID / Client Secret (see [API credentials](#api-credentials)).
//! 3. **Discover and connect** a headset (or use the first available one).
//! 4. **Create a session** and subscribe to data streams.
//! 5. **Receive events** ([`CortexEvent`]) through an async channel.
//!
//! # API credentials
//!
//! You need a **Client ID** and **Client Secret** from the Emotiv developer portal.
//! Create them at <https://www.emotiv.com/my-account/cortex-apps/> after logging in
//! with your EmotivID, then pass them via [`CortexClientConfig::client_id`] and
//! [`CortexClientConfig::client_secret`], or set the `EMOTIV_CLIENT_ID` and
//! `EMOTIV_CLIENT_SECRET` environment variables.
//!
//! On the very first connection the Cortex service will show an approval dialog in
//! the EMOTIV Launcher — click **Approve** (one-time only per Client ID).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use log::{debug, info, warn};
use serde_json::Value;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;

use crate::protocol::*;
use crate::types::*;

// ── Client Configuration ──────────────────────────────────────────────────────

/// Configuration for [`CortexClient`].
///
/// At minimum you must provide [`client_id`](Self::client_id) and
/// [`client_secret`](Self::client_secret). Obtain them by creating a Cortex
/// App at <https://www.emotiv.com/my-account/cortex-apps/>.
///
/// # Example
///
/// ```no_run
/// use emotiv::client::CortexClientConfig;
///
/// let config = CortexClientConfig {
///     client_id: std::env::var("EMOTIV_CLIENT_ID").unwrap(),
///     client_secret: std::env::var("EMOTIV_CLIENT_SECRET").unwrap(),
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone)]
pub struct CortexClientConfig {
    /// **Required.** Your Cortex App client ID.
    ///
    /// Create one at <https://www.emotiv.com/my-account/cortex-apps/>.
    /// Can also be read from the `EMOTIV_CLIENT_ID` environment variable.
    pub client_id: String,

    /// **Required.** Your Cortex App client secret.
    ///
    /// Shown once when you create the app. Keep it private — do not commit to
    /// version control. Can also be read from `EMOTIV_CLIENT_SECRET`.
    pub client_secret: String,

    /// Optional license key. Leave empty for the default (free-tier) license.
    pub license: String,

    /// Session debit — the number of sessions to debit from your license.
    /// Default: `10`.
    pub debit: i64,

    /// Target headset ID (e.g. `"EPOCX-ABCDEF12"`).
    ///
    /// If empty, the first headset returned by `queryHeadsets` is used.
    pub headset_id: String,

    /// Automatically create a session after authorization completes.
    ///
    /// Set to `false` if you only need to query records or profiles without
    /// connecting to a headset. Default: `true`.
    pub auto_create_session: bool,

    /// WebSocket URL of the Cortex service.
    ///
    /// The EMOTIV Launcher listens on `wss://localhost:6868` by default.
    /// Override this only for custom proxy setups.
    pub ws_url: String,

    /// Log every outgoing and incoming WebSocket message at `debug` level.
    ///
    /// Useful for protocol debugging. Default: `false`.
    pub debug_mode: bool,
}

impl Default for CortexClientConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: String::new(),
            license: String::new(),
            debit: 10,
            headset_id: String::new(),
            auto_create_session: true,
            ws_url: CORTEX_WS_URL.to_string(),
            debug_mode: false,
        }
    }
}

// ── Internal shared state ─────────────────────────────────────────────────────

struct ClientState {
    auth_token: String,
    session_id: String,
    headset_id: String,
}

// ── CortexClient ──────────────────────────────────────────────────────────────

/// Async client for the Emotiv Cortex API.
///
/// Connects to the Cortex WebSocket service, handles the auth flow,
/// and dispatches stream data as [`CortexEvent`]s.
pub struct CortexClient {
    config: CortexClientConfig,
    state: Arc<Mutex<ClientState>>,
    /// Sender half for writing commands to the WebSocket.
    ws_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
}

impl CortexClient {
    /// Create a new client with the given configuration.
    pub fn new(config: CortexClientConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(ClientState {
                auth_token: String::new(),
                session_id: String::new(),
                headset_id: String::new(),
            })),
            ws_tx: Arc::new(Mutex::new(None)),
        }
    }

    /// Connect to the Cortex service and start the authentication flow.
    ///
    /// Returns an event receiver and a handle for sending commands.
    pub async fn connect(&self) -> Result<(mpsc::Receiver<CortexEvent>, CortexHandle)> {
        let (event_tx, event_rx) = mpsc::channel::<CortexEvent>(512);
        let (cmd_tx, cmd_rx) = mpsc::channel::<String>(64);

        {
            let mut ws = self.ws_tx.lock().await;
            *ws = Some(cmd_tx.clone());
        }

        let url = self.config.ws_url.clone();
        let config = self.config.clone();
        let state = Arc::clone(&self.state);
        let ws_tx_arc = Arc::clone(&self.ws_tx);

        // Spawn the WebSocket connection task
        tokio::spawn(async move {
            if let Err(e) = run_ws_loop(url, config, state, event_tx, cmd_rx, ws_tx_arc).await {
                warn!("WebSocket loop exited with error: {e}");
            }
        });

        let handle = CortexHandle {
            state: Arc::clone(&self.state),
            ws_tx: Arc::clone(&self.ws_tx),
        };

        Ok((event_rx, handle))
    }

    /// Get the current auth token.
    pub async fn auth_token(&self) -> String {
        self.state.lock().await.auth_token.clone()
    }

    /// Get the current session ID.
    pub async fn session_id(&self) -> String {
        self.state.lock().await.session_id.clone()
    }

    /// Get the current headset ID.
    pub async fn headset_id(&self) -> String {
        self.state.lock().await.headset_id.clone()
    }
}

// ── CortexHandle ──────────────────────────────────────────────────────────────

/// Handle for sending commands to an active Cortex connection.
#[derive(Clone)]
pub struct CortexHandle {
    state: Arc<Mutex<ClientState>>,
    ws_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
}

impl CortexHandle {
    /// Send a raw JSON-RPC request.
    pub async fn send_raw(&self, request: Value) -> Result<()> {
        let ws = self.ws_tx.lock().await;
        if let Some(tx) = ws.as_ref() {
            tx.send(request.to_string()).await
                .map_err(|e| anyhow!("Failed to send: {e}"))?;
        }
        Ok(())
    }

    /// Subscribe to data streams.
    pub async fn subscribe(&self, streams: &[&str]) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(subscribe(&s.auth_token, &s.session_id, streams)).await
    }

    /// Unsubscribe from data streams.
    pub async fn unsubscribe(&self, streams: &[&str]) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(unsubscribe(&s.auth_token, &s.session_id, streams)).await
    }

    /// Create a recording.
    pub async fn create_record(&self, title: &str, description: &str) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(create_record(&s.auth_token, &s.session_id, title, description)).await
    }

    /// Stop the current recording.
    pub async fn stop_record(&self) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(stop_record(&s.auth_token, &s.session_id)).await
    }

    /// Export a record.
    pub async fn export_record(
        &self, folder: &str, format: &str, stream_types: &[&str],
        record_ids: &[&str], version: &str,
    ) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(export_record(&s.auth_token, folder, format, stream_types, record_ids, version)).await
    }

    /// Inject a marker into the current session.
    pub async fn inject_marker(&self, time: f64, value: &str, label: &str) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(inject_marker(&s.auth_token, &s.session_id, time, value, label)).await
    }

    /// Update an existing marker.
    pub async fn update_marker(&self, marker_id: &str, time: f64) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(update_marker(&s.auth_token, &s.session_id, marker_id, time)).await
    }

    /// Query profiles.
    pub async fn query_profile(&self) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(query_profile(&s.auth_token)).await
    }

    /// Get current profile.
    pub async fn get_current_profile(&self) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(get_current_profile(&s.auth_token, &s.headset_id)).await
    }

    /// Setup (create/load/unload/save) a profile.
    pub async fn setup_profile(&self, profile_name: &str, status: &str) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(setup_profile(&s.auth_token, &s.headset_id, profile_name, status)).await
    }

    /// Send a training request.
    pub async fn train(&self, detection: &str, action: &str, status: &str) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(train_request(&s.auth_token, &s.session_id, detection, action, status)).await
    }

    /// Get mental command active actions.
    pub async fn get_mc_active_action(&self, profile_name: &str) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(get_mental_command_active_action(&s.auth_token, profile_name)).await
    }

    /// Get mental command sensitivity.
    pub async fn get_mc_sensitivity(&self, profile_name: &str) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(get_mental_command_sensitivity(&s.auth_token, profile_name)).await
    }

    /// Set mental command sensitivity.
    pub async fn set_mc_sensitivity(&self, profile_name: &str, values: &[i32]) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(set_mental_command_sensitivity(&s.auth_token, profile_name, &s.session_id, values)).await
    }

    /// Get mental command brain map.
    pub async fn get_mc_brain_map(&self, profile_name: &str) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(get_mental_command_brain_map(&s.auth_token, profile_name, &s.session_id)).await
    }

    /// Get mental command training threshold.
    pub async fn get_mc_training_threshold(&self) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(get_mental_command_training_threshold(&s.auth_token, &s.session_id)).await
    }

    /// Query records.
    pub async fn query_records(&self, query: Value) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(query_records(&s.auth_token, query)).await
    }

    /// Request to download records.
    pub async fn request_download_records(&self, record_ids: &[&str]) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(request_download_records(&s.auth_token, record_ids)).await
    }

    /// Sync with headset clock.
    pub async fn sync_headset_clock(&self) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(sync_with_headset_clock(&s.headset_id)).await
    }

    /// Close the session.
    pub async fn close_session(&self) -> Result<()> {
        let s = self.state.lock().await;
        self.send_raw(close_session(&s.auth_token, &s.session_id)).await
    }

    /// Query available headsets.
    ///
    /// The result is emitted as a [`CortexEvent::HeadsetsQueried`] event
    /// containing a list of [`HeadsetInfo`] structs.  When
    /// [`CortexClientConfig::auto_create_session`] is `true`, the client
    /// also automatically connects to the target headset and creates a
    /// session.
    pub async fn query_headsets(&self) -> Result<()> {
        self.send_raw(query_headsets()).await
    }

    /// Get Cortex service info.
    pub async fn get_cortex_info(&self) -> Result<()> {
        self.send_raw(get_cortex_info()).await
    }

    /// Get the current auth token.
    pub async fn auth_token(&self) -> String {
        self.state.lock().await.auth_token.clone()
    }

    /// Get the current session ID.
    pub async fn session_id(&self) -> String {
        self.state.lock().await.session_id.clone()
    }

    /// Get the current headset ID.
    pub async fn headset_id(&self) -> String {
        self.state.lock().await.headset_id.clone()
    }
}

// ── WebSocket loop ────────────────────────────────────────────────────────────

async fn run_ws_loop(
    url: String,
    config: CortexClientConfig,
    state: Arc<Mutex<ClientState>>,
    event_tx: mpsc::Sender<CortexEvent>,
    mut cmd_rx: mpsc::Receiver<String>,
    _ws_tx_arc: Arc<Mutex<Option<mpsc::Sender<String>>>>,
) -> Result<()> {
    info!("Connecting to Cortex service at {url}");

    // Build a TLS connector that accepts the Emotiv self-signed certificate
    let tls_connector = build_tls_connector()?;
    let connector = tokio_tungstenite::Connector::Rustls(Arc::new(tls_connector));

    let (ws_stream, _response) = tokio_tungstenite::connect_async_tls_with_config(
        &url,
        None,
        false,
        Some(connector),
    ).await.map_err(|e| anyhow!("WebSocket connection failed: {e}"))?;

    info!("WebSocket connected to {url}");
    let _ = event_tx.send(CortexEvent::Connected).await;

    let (mut write, mut read) = ws_stream.split();

    // Start the authentication flow
    let auth_msg = has_access_right(&config.client_id, &config.client_secret);
    write.send(Message::Text(auth_msg.to_string().into())).await?;

    loop {
        tokio::select! {
            // Handle incoming WS messages
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let text_str: &str = text.as_ref();
                        if config.debug_mode {
                            eprintln!("[emotiv-ws] recv: {text_str}");
                        }
                        match serde_json::from_str::<Value>(text_str) {
                            Ok(recv) => {
                                let responses = handle_message(
                                    &recv, &config, &state, &event_tx,
                                ).await;
                                for resp in responses {
                                    write.send(Message::Text(resp.into())).await?;
                                }
                            }
                            Err(e) => {
                                warn!("Failed to parse WS message: {e}");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("WebSocket closed by server");
                        let _ = event_tx.send(CortexEvent::Disconnected).await;
                        break;
                    }
                    Some(Err(e)) => {
                        warn!("WebSocket error: {e}");
                        let _ = event_tx.send(CortexEvent::Error(e.to_string())).await;
                        let _ = event_tx.send(CortexEvent::Disconnected).await;
                        break;
                    }
                    None => {
                        info!("WebSocket stream ended");
                        let _ = event_tx.send(CortexEvent::Disconnected).await;
                        break;
                    }
                    _ => {}
                }
            }
            // Handle outgoing commands
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(msg) => {
                        if config.debug_mode {
                            eprintln!("[emotiv-ws] send: {msg}");
                        }
                        write.send(Message::Text(msg.into())).await?;
                    }
                    None => {
                        info!("Command channel closed");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

fn build_tls_connector() -> Result<rustls::ClientConfig> {
    use rustls::ClientConfig;

    // Accept all certificates (Emotiv uses a self-signed cert on localhost)
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
        .with_no_client_auth();

    Ok(config)
}

/// Certificate verifier that accepts any certificate (for localhost self-signed).
#[derive(Debug)]
struct AcceptAnyCert;

impl rustls::client::danger::ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ED448,
        ]
    }
}

// ── Message handler ───────────────────────────────────────────────────────────

async fn handle_message(
    recv: &Value,
    config: &CortexClientConfig,
    state: &Arc<Mutex<ClientState>>,
    event_tx: &mpsc::Sender<CortexEvent>,
) -> Vec<String> {
    let mut responses = Vec::new();

    // Stream data (has "sid" field)
    if recv.get("sid").is_some() {
        handle_stream_data(recv, event_tx).await;
        return responses;
    }

    // Result message
    if let Some(result) = recv.get("result") {
        if let Some(id) = recv.get("id").and_then(|v| v.as_i64()) {
            let resps = handle_result(id, result, config, state, event_tx).await;
            responses.extend(resps);
        }
        return responses;
    }

    // Error message
    if let Some(error) = recv.get("error") {
        let msg = error.get("message").and_then(|v| v.as_str()).unwrap_or("unknown");
        let code = error.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
        let req_id = recv.get("id").and_then(|v| v.as_i64()).unwrap_or(-1);
        eprintln!("[emotiv-ws] ERROR (req_id={req_id}, code={code}): {msg}");
        let cortex_err = crate::error::CortexError::from_api_error(code as i32, msg);
        let _ = event_tx.send(CortexEvent::Error(
            format!("[req_id={req_id}] {cortex_err}")
        )).await;
        return responses;
    }

    // Warning message
    if let Some(warning) = recv.get("warning") {
        let code = warning.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        let message = warning.get("message").cloned().unwrap_or(Value::Null);
        let _ = event_tx.send(CortexEvent::Warning { code, message: message.clone() }).await;

        match code {
            ACCESS_RIGHT_GRANTED => {
                // Only authorize if we don't already have a token.
                // Re-authorizing while a session is active would invalidate
                // the current cortex token and kill the running session.
                let already_authed = !state.lock().await.auth_token.is_empty();
                if !already_authed {
                    responses.push(authorize(&config.client_id, &config.client_secret, &config.license, config.debit).to_string());
                } else {
                    info!("ACCESS_RIGHT_GRANTED received but already authorized — skipping re-auth");
                }
            }
            HEADSET_CONNECTED => {
                // Only query headsets if we don't already have an active session.
                // Re-querying would trigger create_session again, potentially
                // disrupting the running stream.
                let has_session = !state.lock().await.session_id.is_empty();
                if !has_session {
                    responses.push(query_headsets().to_string());
                } else {
                    info!("HEADSET_CONNECTED received but session already active — skipping query");
                }
            }
            CORTEX_STOP_ALL_STREAMS => {
                let mut s = state.lock().await;
                s.session_id.clear();
            }
            CORTEX_RECORD_POST_PROCESSING_DONE => {
                if let Some(record_id) = message.get("recordId").and_then(|v| v.as_str()) {
                    let _ = event_tx.send(CortexEvent::RecordPostProcessingDone(record_id.to_string())).await;
                }
            }
            HEADSET_SCANNING_FINISHED => {
                let has_session = !state.lock().await.session_id.is_empty();
                if !has_session {
                    responses.push(refresh_headset_list().to_string());
                }
            }
            _ => {}
        }
        return responses;
    }

    responses
}

async fn handle_result(
    req_id: i64,
    result: &Value,
    config: &CortexClientConfig,
    state: &Arc<Mutex<ClientState>>,
    event_tx: &mpsc::Sender<CortexEvent>,
) -> Vec<String> {
    let mut responses = Vec::new();

    match req_id {
        HAS_ACCESS_RIGHT_ID => {
            let granted = result.get("accessGranted").and_then(|v| v.as_bool()).unwrap_or(false);
            info!("hasAccessRight: granted={granted}");
            if granted {
                responses.push(authorize(&config.client_id, &config.client_secret, &config.license, config.debit).to_string());
            } else {
                responses.push(request_access(&config.client_id, &config.client_secret).to_string());
            }
        }

        REQUEST_ACCESS_ID => {
            let granted = result.get("accessGranted").and_then(|v| v.as_bool()).unwrap_or(false);
            if granted {
                responses.push(authorize(&config.client_id, &config.client_secret, &config.license, config.debit).to_string());
            } else {
                let msg = result.get("message").and_then(|v| v.as_str()).unwrap_or("Access not granted");
                warn!("Access not granted: {msg}");
                let _ = event_tx.send(CortexEvent::Error(format!("Access not granted: {msg}"))).await;
            }
        }

        AUTHORIZE_ID => {
            if let Some(token) = result.get("cortexToken").and_then(|v| v.as_str()) {
                info!("Authorized successfully");
                state.lock().await.auth_token = token.to_string();
                let _ = event_tx.send(CortexEvent::Authorized).await;

                if config.auto_create_session {
                    responses.push(refresh_headset_list().to_string());
                    responses.push(query_headsets().to_string());
                }
            }
        }

        QUERY_HEADSET_ID => {
            if let Some(headsets) = result.as_array() {
                // Always emit the headset list so callers can enumerate
                // available devices (e.g. for a device-selection UI).
                let infos: Vec<crate::types::HeadsetInfo> = headsets.iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect();
                let _ = event_tx.send(CortexEvent::HeadsetsQueried(infos)).await;

                // When auto_create_session is disabled, stop here — the
                // caller only wants the headset list, not the side effects
                // of connecting to a headset and creating a session.
                if !config.auto_create_session {
                    return responses;
                }

                let mut s = state.lock().await;

                if headsets.is_empty() {
                    warn!("No headsets available");
                    return responses;
                }

                let target_id = if config.headset_id.is_empty() {
                    headsets[0].get("id").and_then(|v| v.as_str()).unwrap_or("").to_string()
                } else {
                    config.headset_id.clone()
                };

                s.headset_id = target_id.clone();

                // Find the headset and check status
                for hs in headsets {
                    let id = hs.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let status = hs.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    info!("Headset: {id}, status: {status}");

                    if id == target_id {
                        match status {
                            "connected" => {
                                responses.push(create_session(&s.auth_token, &s.headset_id).to_string());
                            }
                            "discovered" => {
                                responses.push(connect_headset(&s.headset_id).to_string());
                            }
                            "connecting" => {
                                // Will retry via timer
                                tokio::spawn({
                                    let headset_id = s.headset_id.clone();
                                    async move {
                                        tokio::time::sleep(Duration::from_secs(3)).await;
                                        info!("Would re-query headset {headset_id}");
                                    }
                                });
                            }
                            _ => {
                                warn!("Unknown headset status: {status}");
                            }
                        }
                        break;
                    }
                }
            }
        }

        CREATE_SESSION_ID => {
            if let Some(session_id) = result.get("id").and_then(|v| v.as_str()) {
                info!("Session created: {session_id}");
                state.lock().await.session_id = session_id.to_string();
                let _ = event_tx.send(CortexEvent::SessionCreated(session_id.to_string())).await;
            }
        }

        SUB_REQUEST_ID => {
            if let Some(success) = result.get("success").and_then(|v| v.as_array()) {
                for stream in success {
                    let name = stream.get("streamName").and_then(|v| v.as_str()).unwrap_or("");
                    let cols = stream.get("cols").and_then(|v| v.as_array());
                    info!("Subscribed to stream: {name}");

                    if let Some(cols) = cols {
                        if name != "com" && name != "fac" {
                            let labels: Vec<String> = cols.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect();
                            let _ = event_tx.send(CortexEvent::DataLabels(DataLabels {
                                stream_name: name.to_string(),
                                labels,
                            })).await;
                        }
                    }
                }
            }
            // Log and emit errors for failed stream subscriptions.
            if let Some(failure) = result.get("failure").and_then(|v| v.as_array()) {
                for stream in failure {
                    let name = stream.get("streamName").and_then(|v| v.as_str()).unwrap_or("");
                    let code = stream.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
                    let message = stream.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    warn!("Failed to subscribe to stream '{name}': code={code} {message}");
                    let _ = event_tx.send(CortexEvent::Error(
                        format!("Subscribe '{name}' failed: code={code} {message}")
                    )).await;
                }
            }
        }

        UNSUB_REQUEST_ID => {
            if let Some(success) = result.get("success").and_then(|v| v.as_array()) {
                for stream in success {
                    let name = stream.get("streamName").and_then(|v| v.as_str()).unwrap_or("");
                    info!("Unsubscribed from stream: {name}");
                }
            }
        }

        QUERY_PROFILE_ID => {
            if let Some(profiles) = result.as_array() {
                let names: Vec<String> = profiles.iter()
                    .filter_map(|p| p.get("name").and_then(|v| v.as_str()).map(String::from))
                    .collect();
                let _ = event_tx.send(CortexEvent::ProfilesQueried(names)).await;
            }
        }

        SETUP_PROFILE_ID => {
            let action = result.get("action").and_then(|v| v.as_str()).unwrap_or("");
            match action {
                "load" => {
                    info!("Profile loaded");
                    let _ = event_tx.send(CortexEvent::ProfileLoaded(true)).await;
                }
                "unload" => {
                    info!("Profile unloaded");
                    let _ = event_tx.send(CortexEvent::ProfileLoaded(false)).await;
                }
                "save" => {
                    info!("Profile saved");
                    let _ = event_tx.send(CortexEvent::ProfileSaved).await;
                }
                "create" => {
                    if let Some(name) = result.get("name").and_then(|v| v.as_str()) {
                        info!("Profile created: {name}");
                    }
                }
                _ => {}
            }
        }

        CREATE_RECORD_REQUEST_ID => {
            if let Some(record) = result.get("record") {
                if let Ok(rec) = serde_json::from_value::<Record>(record.clone()) {
                    let _ = event_tx.send(CortexEvent::RecordCreated(rec)).await;
                }
            }
        }

        STOP_RECORD_REQUEST_ID => {
            if let Some(record) = result.get("record") {
                if let Ok(rec) = serde_json::from_value::<Record>(record.clone()) {
                    let _ = event_tx.send(CortexEvent::RecordStopped(rec)).await;
                }
            }
        }

        EXPORT_RECORD_ID => {
            let mut success_ids = Vec::new();
            if let Some(success) = result.get("success").and_then(|v| v.as_array()) {
                for r in success {
                    if let Some(id) = r.get("recordId").and_then(|v| v.as_str()) {
                        success_ids.push(id.to_string());
                    }
                }
            }
            let _ = event_tx.send(CortexEvent::RecordExported(success_ids)).await;
        }

        INJECT_MARKER_REQUEST_ID => {
            if let Some(marker) = result.get("marker") {
                if let Ok(m) = serde_json::from_value::<Marker>(marker.clone()) {
                    let _ = event_tx.send(CortexEvent::MarkerInjected(m)).await;
                }
            }
        }

        UPDATE_MARKER_REQUEST_ID => {
            if let Some(marker) = result.get("marker") {
                if let Ok(m) = serde_json::from_value::<Marker>(marker.clone()) {
                    let _ = event_tx.send(CortexEvent::MarkerUpdated(m)).await;
                }
            }
        }

        MENTAL_COMMAND_ACTIVE_ACTION_ID => {
            let _ = event_tx.send(CortexEvent::McActiveActions(result.clone())).await;
        }

        SENSITIVITY_REQUEST_ID => {
            let _ = event_tx.send(CortexEvent::McSensitivity(result.clone())).await;
        }

        MENTAL_COMMAND_BRAIN_MAP_ID => {
            let _ = event_tx.send(CortexEvent::McBrainMap(result.clone())).await;
        }

        MENTAL_COMMAND_TRAINING_THRESHOLD => {
            let _ = event_tx.send(CortexEvent::McTrainingThreshold(result.clone())).await;
        }

        QUERY_RECORDS_ID => {
            let count = result.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            let records_val = result.get("records").and_then(|v| v.as_array());
            let records: Vec<Record> = records_val
                .map(|arr| arr.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect())
                .unwrap_or_default();
            let _ = event_tx.send(CortexEvent::QueryRecordsDone { records, count }).await;
        }

        REQUEST_DOWNLOAD_RECORDS_ID => {
            let _ = event_tx.send(CortexEvent::DownloadRecordsDone(result.clone())).await;
        }

        GET_CORTEX_INFO_ID => {
            let _ = event_tx.send(CortexEvent::CortexInfo(result.clone())).await;
        }

        SYNC_WITH_HEADSET_CLOCK_ID => {
            let _ = event_tx.send(CortexEvent::HeadsetClockSynced(result.clone())).await;
        }

        _ => {
            debug!("Unhandled result for request id={req_id}");
        }
    }

    responses
}

async fn handle_stream_data(recv: &Value, event_tx: &mpsc::Sender<CortexEvent>) {
    let time = recv.get("time").and_then(|v| v.as_f64()).unwrap_or(0.0);

    if let Some(eeg) = recv.get("eeg").and_then(|v| v.as_array()) {
        // Use NaN for non-numeric values (e.g. marker strings "0:0:0") to
        // preserve array positions.  Consumers use DataLabels to know which
        // indices are electrodes — filter_map would shift indices and break
        // the electrode mapping.
        let samples: Vec<f64> = eeg.iter()
            .map(|v| v.as_f64().unwrap_or(f64::NAN))
            .collect();
        let _ = event_tx.send(CortexEvent::Eeg(EegData { samples, time })).await;
    } else if let Some(mot) = recv.get("mot").and_then(|v| v.as_array()) {
        let samples: Vec<f64> = mot.iter()
            .map(|v| v.as_f64().unwrap_or(f64::NAN))
            .collect();
        let _ = event_tx.send(CortexEvent::Motion(MotionData { samples, time })).await;
    } else if let Some(dev) = recv.get("dev").and_then(|v| v.as_array()) {
        let signal = dev.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let cq = dev.get(2).and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
            .unwrap_or_default();
        let bat = dev.get(3).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let _ = event_tx.send(CortexEvent::Dev(DevData {
            signal, contact_quality: cq, battery_percent: bat, time,
        })).await;
    } else if let Some(met) = recv.get("met").and_then(|v| v.as_array()) {
        let values: Vec<f64> = met.iter().filter_map(|v| v.as_f64()).collect();
        let _ = event_tx.send(CortexEvent::Metrics(MetricsData { values, time })).await;
    } else if let Some(pow) = recv.get("pow").and_then(|v| v.as_array()) {
        let powers: Vec<f64> = pow.iter().filter_map(|v| v.as_f64()).collect();
        let _ = event_tx.send(CortexEvent::BandPower(BandPowerData { powers, time })).await;
    } else if let Some(com) = recv.get("com").and_then(|v| v.as_array()) {
        let action = com.first().and_then(|v| v.as_str()).unwrap_or("neutral").to_string();
        let power = com.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let _ = event_tx.send(CortexEvent::MentalCommand(MentalCommandData {
            action, power, time,
        })).await;
    } else if let Some(fac) = recv.get("fac").and_then(|v| v.as_array()) {
        let eye = fac.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
        let u_act = fac.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let u_pow = fac.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let l_act = fac.get(3).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let l_pow = fac.get(4).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let _ = event_tx.send(CortexEvent::FacialExpression(FacialExpressionData {
            eye_action: eye, upper_action: u_act, upper_power: u_pow,
            lower_action: l_act, lower_power: l_pow, time,
        })).await;
    } else if let Some(sys) = recv.get("sys").and_then(|v| v.as_array()) {
        let events: Vec<Value> = sys.clone();
        let _ = event_tx.send(CortexEvent::Sys(SysData { events })).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = CortexClientConfig::default();
        assert_eq!(config.ws_url, "wss://localhost:6868");
        assert_eq!(config.debit, 10);
        assert!(config.auto_create_session);
    }

    #[tokio::test]
    async fn test_handle_stream_eeg() {
        let (tx, mut rx) = mpsc::channel(16);
        let msg = serde_json::json!({
            "sid": "test-session",
            "eeg": [1.0, 2.0, 3.0, 4.0, 5.0],
            "time": 1234567890.123
        });
        handle_stream_data(&msg, &tx).await;
        if let Some(CortexEvent::Eeg(data)) = rx.recv().await {
            assert_eq!(data.samples.len(), 5);
            assert!((data.time - 1234567890.123).abs() < 0.001);
        } else {
            panic!("Expected Eeg event");
        }
    }

    #[tokio::test]
    async fn test_handle_stream_motion() {
        let (tx, mut rx) = mpsc::channel(16);
        let msg = serde_json::json!({
            "sid": "test-session",
            "mot": [0.0, 0.0, 0.5, 0.3, 0.2, 0.1, 0.01, 0.02, -1.0, 50.0, 30.0, 20.0],
            "time": 100.0
        });
        handle_stream_data(&msg, &tx).await;
        if let Some(CortexEvent::Motion(data)) = rx.recv().await {
            assert_eq!(data.samples.len(), 12);
        } else {
            panic!("Expected Motion event");
        }
    }

    #[tokio::test]
    async fn test_handle_stream_dev() {
        let (tx, mut rx) = mpsc::channel(16);
        let msg = serde_json::json!({
            "sid": "test-session",
            "dev": [0, 1.0, [4, 4, 4, 4, 4], 85.0],
            "time": 100.0
        });
        handle_stream_data(&msg, &tx).await;
        if let Some(CortexEvent::Dev(data)) = rx.recv().await {
            assert!((data.signal - 1.0).abs() < 0.001);
            assert_eq!(data.contact_quality.len(), 5);
            assert!((data.battery_percent - 85.0).abs() < 0.001);
        } else {
            panic!("Expected Dev event");
        }
    }

    #[tokio::test]
    async fn test_handle_stream_metrics() {
        let (tx, mut rx) = mpsc::channel(16);
        let msg = serde_json::json!({
            "sid": "test-session",
            "met": [1.0, 0.5, 1.0, 0.4, 0.3, 1.0, 0.2, 1.0, 0.6, 1.0, 0.5, 1.0, 0.55],
            "time": 100.0
        });
        handle_stream_data(&msg, &tx).await;
        if let Some(CortexEvent::Metrics(data)) = rx.recv().await {
            assert_eq!(data.values.len(), 13);
        } else {
            panic!("Expected Metrics event");
        }
    }

    #[tokio::test]
    async fn test_handle_stream_com() {
        let (tx, mut rx) = mpsc::channel(16);
        let msg = serde_json::json!({
            "sid": "test-session",
            "com": ["push", 0.85],
            "time": 100.0
        });
        handle_stream_data(&msg, &tx).await;
        if let Some(CortexEvent::MentalCommand(data)) = rx.recv().await {
            assert_eq!(data.action, "push");
            assert!((data.power - 0.85).abs() < 0.001);
        } else {
            panic!("Expected MentalCommand event");
        }
    }

    #[tokio::test]
    async fn test_handle_stream_fac() {
        let (tx, mut rx) = mpsc::channel(16);
        let msg = serde_json::json!({
            "sid": "test-session",
            "fac": ["blink", "surprise", 0.7, "smile", 0.5],
            "time": 100.0
        });
        handle_stream_data(&msg, &tx).await;
        if let Some(CortexEvent::FacialExpression(data)) = rx.recv().await {
            assert_eq!(data.eye_action, "blink");
            assert_eq!(data.upper_action, "surprise");
            assert!((data.upper_power - 0.7).abs() < 0.001);
        } else {
            panic!("Expected FacialExpression event");
        }
    }

    #[tokio::test]
    async fn test_handle_stream_pow() {
        let (tx, mut rx) = mpsc::channel(16);
        let msg = serde_json::json!({
            "sid": "test-session",
            "pow": [5.0, 4.0, 3.0, 1.0, 0.5],
            "time": 100.0
        });
        handle_stream_data(&msg, &tx).await;
        if let Some(CortexEvent::BandPower(data)) = rx.recv().await {
            assert_eq!(data.powers.len(), 5);
        } else {
            panic!("Expected BandPower event");
        }
    }

    #[tokio::test]
    async fn test_handle_stream_sys() {
        let (tx, mut rx) = mpsc::channel(16);
        let msg = serde_json::json!({
            "sid": "test-session",
            "sys": ["mentalCommand", "MC_Succeeded"]
        });
        handle_stream_data(&msg, &tx).await;
        if let Some(CortexEvent::Sys(data)) = rx.recv().await {
            assert_eq!(data.events.len(), 2);
        } else {
            panic!("Expected Sys event");
        }
    }

    // ── Message handler tests ─────────────────────────────────────────────

    fn test_config() -> CortexClientConfig {
        CortexClientConfig {
            client_id: "test_id".into(),
            client_secret: "test_secret".into(),
            ..Default::default()
        }
    }

    fn test_state() -> Arc<Mutex<ClientState>> {
        Arc::new(Mutex::new(ClientState {
            auth_token: "test_token".into(),
            session_id: "test_session".into(),
            headset_id: "test_headset".into(),
        }))
    }

    #[tokio::test]
    async fn test_handle_has_access_right_granted() {
        let (tx, _rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!({"accessGranted": true});
        let responses = handle_result(HAS_ACCESS_RIGHT_ID, &result, &config, &state, &tx).await;
        // Should produce an authorize request
        assert_eq!(responses.len(), 1);
        let resp: serde_json::Value = serde_json::from_str(&responses[0]).unwrap();
        assert_eq!(resp["method"], "authorize");
    }

    #[tokio::test]
    async fn test_handle_has_access_right_denied() {
        let (tx, _rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!({"accessGranted": false});
        let responses = handle_result(HAS_ACCESS_RIGHT_ID, &result, &config, &state, &tx).await;
        // Should produce a requestAccess
        assert_eq!(responses.len(), 1);
        let resp: serde_json::Value = serde_json::from_str(&responses[0]).unwrap();
        assert_eq!(resp["method"], "requestAccess");
    }

    #[tokio::test]
    async fn test_handle_authorize() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!({"cortexToken": "new_token_abc"});
        let responses = handle_result(AUTHORIZE_ID, &result, &config, &state, &tx).await;

        // Should store the token
        assert_eq!(state.lock().await.auth_token, "new_token_abc");

        // Should emit Authorized event
        if let Some(CortexEvent::Authorized) = rx.recv().await {
            // good
        } else {
            panic!("Expected Authorized event");
        }

        // auto_create_session=true should produce refresh + query headsets
        assert_eq!(responses.len(), 2);
    }

    #[tokio::test]
    async fn test_handle_create_session() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!({"id": "session-xyz"});
        let _responses = handle_result(CREATE_SESSION_ID, &result, &config, &state, &tx).await;

        assert_eq!(state.lock().await.session_id, "session-xyz");

        if let Some(CortexEvent::SessionCreated(id)) = rx.recv().await {
            assert_eq!(id, "session-xyz");
        } else {
            panic!("Expected SessionCreated event");
        }
    }

    #[tokio::test]
    async fn test_handle_subscribe() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!({
            "success": [
                {"streamName": "eeg", "cols": ["AF3", "F7", "F3"]},
                {"streamName": "mot", "cols": ["ACCX", "ACCY", "ACCZ"]}
            ],
            "failure": []
        });
        let _responses = handle_result(SUB_REQUEST_ID, &result, &config, &state, &tx).await;

        // Should emit 2 DataLabels events (eeg and mot)
        let ev1 = rx.recv().await.unwrap();
        let ev2 = rx.recv().await.unwrap();
        let mut labels_received = vec![];
        for ev in [ev1, ev2] {
            if let CortexEvent::DataLabels(l) = ev {
                labels_received.push(l.stream_name);
            }
        }
        labels_received.sort();
        assert_eq!(labels_received, vec!["eeg", "mot"]);
    }

    #[tokio::test]
    async fn test_handle_query_profile() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!([
            {"name": "profile_a", "readOnly": false},
            {"name": "profile_b", "readOnly": true}
        ]);
        let _responses = handle_result(QUERY_PROFILE_ID, &result, &config, &state, &tx).await;

        if let Some(CortexEvent::ProfilesQueried(names)) = rx.recv().await {
            assert_eq!(names, vec!["profile_a", "profile_b"]);
        } else {
            panic!("Expected ProfilesQueried event");
        }
    }

    #[tokio::test]
    async fn test_handle_setup_profile_load() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!({"action": "load"});
        let _responses = handle_result(SETUP_PROFILE_ID, &result, &config, &state, &tx).await;

        if let Some(CortexEvent::ProfileLoaded(true)) = rx.recv().await {
            // good
        } else {
            panic!("Expected ProfileLoaded(true)");
        }
    }

    #[tokio::test]
    async fn test_handle_setup_profile_save() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!({"action": "save"});
        let _responses = handle_result(SETUP_PROFILE_ID, &result, &config, &state, &tx).await;

        if let Some(CortexEvent::ProfileSaved) = rx.recv().await {
            // good
        } else {
            panic!("Expected ProfileSaved");
        }
    }

    #[tokio::test]
    async fn test_handle_create_record() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!({
            "record": {
                "uuid": "rec-123",
                "title": "Test",
                "startDatetime": "2026-01-01T00:00:00Z"
            }
        });
        let _responses = handle_result(CREATE_RECORD_REQUEST_ID, &result, &config, &state, &tx).await;

        if let Some(CortexEvent::RecordCreated(rec)) = rx.recv().await {
            assert_eq!(rec.uuid, "rec-123");
            assert_eq!(rec.title, "Test");
        } else {
            panic!("Expected RecordCreated");
        }
    }

    #[tokio::test]
    async fn test_handle_stop_record() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!({
            "record": {
                "uuid": "rec-123",
                "title": "Test",
                "startDatetime": "2026-01-01T00:00:00Z",
                "endDatetime": "2026-01-01T00:01:00Z"
            }
        });
        let _responses = handle_result(STOP_RECORD_REQUEST_ID, &result, &config, &state, &tx).await;

        if let Some(CortexEvent::RecordStopped(rec)) = rx.recv().await {
            assert_eq!(rec.uuid, "rec-123");
        } else {
            panic!("Expected RecordStopped");
        }
    }

    #[tokio::test]
    async fn test_handle_export_record() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!({
            "success": [{"recordId": "rec-1"}, {"recordId": "rec-2"}],
            "failure": []
        });
        let _responses = handle_result(EXPORT_RECORD_ID, &result, &config, &state, &tx).await;

        if let Some(CortexEvent::RecordExported(ids)) = rx.recv().await {
            assert_eq!(ids, vec!["rec-1", "rec-2"]);
        } else {
            panic!("Expected RecordExported");
        }
    }

    #[tokio::test]
    async fn test_handle_inject_marker() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!({
            "marker": {
                "uuid": "mk-001",
                "type": "instance",
                "label": "test_label",
                "value": "test_val",
                "startDatetime": "2026-01-01T00:00:00Z"
            }
        });
        let _responses = handle_result(INJECT_MARKER_REQUEST_ID, &result, &config, &state, &tx).await;

        if let Some(CortexEvent::MarkerInjected(m)) = rx.recv().await {
            assert_eq!(m.uuid, "mk-001");
            assert_eq!(m.label, "test_label");
        } else {
            panic!("Expected MarkerInjected");
        }
    }

    #[tokio::test]
    async fn test_handle_query_records() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!({
            "count": 2,
            "limit": 100,
            "offset": 0,
            "records": [
                {"uuid": "r1", "title": "First"},
                {"uuid": "r2", "title": "Second"}
            ]
        });
        let _responses = handle_result(QUERY_RECORDS_ID, &result, &config, &state, &tx).await;

        if let Some(CortexEvent::QueryRecordsDone { records, count }) = rx.recv().await {
            assert_eq!(count, 2);
            assert_eq!(records.len(), 2);
            assert_eq!(records[0].uuid, "r1");
            assert_eq!(records[1].title, "Second");
        } else {
            panic!("Expected QueryRecordsDone");
        }
    }

    #[tokio::test]
    async fn test_handle_mc_active_actions() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!(["neutral", "push", "pull"]);
        let _responses = handle_result(MENTAL_COMMAND_ACTIVE_ACTION_ID, &result, &config, &state, &tx).await;

        if let Some(CortexEvent::McActiveActions(data)) = rx.recv().await {
            assert!(data.is_array());
            assert_eq!(data.as_array().unwrap().len(), 3);
        } else {
            panic!("Expected McActiveActions");
        }
    }

    #[tokio::test]
    async fn test_handle_mc_sensitivity() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!([7, 8, 5, 5]);
        let _responses = handle_result(SENSITIVITY_REQUEST_ID, &result, &config, &state, &tx).await;

        if let Some(CortexEvent::McSensitivity(data)) = rx.recv().await {
            assert_eq!(data[0], 7);
        } else {
            panic!("Expected McSensitivity");
        }
    }

    #[tokio::test]
    async fn test_handle_warning_message() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let msg = serde_json::json!({
            "warning": {
                "code": 30,
                "message": {"recordId": "rec-done"}
            }
        });
        let responses = handle_message(&msg, &config, &state, &tx).await;

        // Warning 30 = CORTEX_RECORD_POST_PROCESSING_DONE
        // Should emit Warning + RecordPostProcessingDone
        let mut found_warning = false;
        let mut found_ppd = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                CortexEvent::Warning { code, .. } => {
                    assert_eq!(code, 30);
                    found_warning = true;
                }
                CortexEvent::RecordPostProcessingDone(rid) => {
                    assert_eq!(rid, "rec-done");
                    found_ppd = true;
                }
                _ => {}
            }
        }
        assert!(found_warning, "Expected Warning event");
        assert!(found_ppd, "Expected RecordPostProcessingDone event");
        let _ = responses; // no response messages needed for this warning
    }

    #[tokio::test]
    async fn test_handle_error_message() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let msg = serde_json::json!({
            "id": 999,
            "error": {
                "code": -32046,
                "message": "Profile access denied"
            }
        });
        let _responses = handle_message(&msg, &config, &state, &tx).await;

        if let Some(CortexEvent::Error(e)) = rx.recv().await {
            assert!(e.contains("-32046"));
            assert!(e.contains("Profile access denied"));
        } else {
            panic!("Expected Error event");
        }
    }

    #[tokio::test]
    async fn test_handle_stream_data_routing() {
        // Verify that messages with "sid" are routed to stream handler
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let msg = serde_json::json!({
            "sid": "ses-001",
            "eeg": [10.0, 20.0],
            "time": 500.0
        });
        let responses = handle_message(&msg, &config, &state, &tx).await;
        assert!(responses.is_empty()); // stream data produces no responses

        if let Some(CortexEvent::Eeg(data)) = rx.recv().await {
            assert_eq!(data.samples, vec![10.0, 20.0]);
        } else {
            panic!("Expected Eeg from stream routing");
        }
    }

    #[tokio::test]
    async fn test_handle_headset_query_connected() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = CortexClientConfig {
            client_id: "test".into(),
            client_secret: "test".into(),
            headset_id: "EPOCX-001".into(),
            ..Default::default()
        };
        let state = Arc::new(Mutex::new(ClientState {
            auth_token: "tok".into(),
            session_id: String::new(),
            headset_id: String::new(),
        }));
        let result = serde_json::json!([
            {"id": "EPOCX-001", "status": "connected", "connectedBy": "dongle"}
        ]);
        let responses = handle_result(QUERY_HEADSET_ID, &result, &config, &state, &tx).await;

        // Should emit HeadsetsQueried event
        if let Some(CortexEvent::HeadsetsQueried(headsets)) = rx.recv().await {
            assert_eq!(headsets.len(), 1);
            assert_eq!(headsets[0].id, "EPOCX-001");
            assert_eq!(headsets[0].status, "connected");
        } else {
            panic!("Expected HeadsetsQueried event");
        }

        // Should produce a createSession request
        assert_eq!(responses.len(), 1);
        let resp: serde_json::Value = serde_json::from_str(&responses[0]).unwrap();
        assert_eq!(resp["method"], "createSession");
        assert_eq!(state.lock().await.headset_id, "EPOCX-001");
    }

    #[tokio::test]
    async fn test_handle_headset_query_discovered() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = CortexClientConfig {
            client_id: "test".into(),
            client_secret: "test".into(),
            headset_id: "INSIGHT-002".into(),
            ..Default::default()
        };
        let state = Arc::new(Mutex::new(ClientState {
            auth_token: "tok".into(),
            session_id: String::new(),
            headset_id: String::new(),
        }));
        let result = serde_json::json!([
            {"id": "INSIGHT-002", "status": "discovered", "connectedBy": ""}
        ]);
        let responses = handle_result(QUERY_HEADSET_ID, &result, &config, &state, &tx).await;

        // Should emit HeadsetsQueried event
        if let Some(CortexEvent::HeadsetsQueried(headsets)) = rx.recv().await {
            assert_eq!(headsets.len(), 1);
            assert_eq!(headsets[0].id, "INSIGHT-002");
            assert_eq!(headsets[0].status, "discovered");
        } else {
            panic!("Expected HeadsetsQueried event");
        }

        // Should produce a controlDevice connect
        assert_eq!(responses.len(), 1);
        let resp: serde_json::Value = serde_json::from_str(&responses[0]).unwrap();
        assert_eq!(resp["method"], "controlDevice");
        assert_eq!(resp["params"]["command"], "connect");
    }

    #[tokio::test]
    async fn test_handle_headset_query_multiple_headsets() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = CortexClientConfig {
            client_id: "test".into(),
            client_secret: "test".into(),
            ..Default::default() // no headset_id → picks first
        };
        let state = Arc::new(Mutex::new(ClientState {
            auth_token: "tok".into(),
            session_id: String::new(),
            headset_id: String::new(),
        }));
        let result = serde_json::json!([
            {"id": "EPOCX-AAA", "status": "connected", "connectedBy": "dongle"},
            {"id": "INSIGHT-BBB", "status": "discovered", "connectedBy": ""}
        ]);
        let responses = handle_result(QUERY_HEADSET_ID, &result, &config, &state, &tx).await;

        // HeadsetsQueried should list both headsets
        if let Some(CortexEvent::HeadsetsQueried(headsets)) = rx.recv().await {
            assert_eq!(headsets.len(), 2);
            assert_eq!(headsets[0].id, "EPOCX-AAA");
            assert_eq!(headsets[1].id, "INSIGHT-BBB");
        } else {
            panic!("Expected HeadsetsQueried event");
        }

        // With no headset_id configured, picks first → createSession for EPOCX-AAA
        assert_eq!(responses.len(), 1);
        assert_eq!(state.lock().await.headset_id, "EPOCX-AAA");
    }

    #[tokio::test]
    async fn test_handle_headset_query_empty() {
        let (tx, mut rx) = mpsc::channel(16);
        let config = test_config();
        let state = test_state();
        let result = serde_json::json!([]);
        let responses = handle_result(QUERY_HEADSET_ID, &result, &config, &state, &tx).await;

        // Should still emit HeadsetsQueried with empty list
        if let Some(CortexEvent::HeadsetsQueried(headsets)) = rx.recv().await {
            assert!(headsets.is_empty());
        } else {
            panic!("Expected HeadsetsQueried event");
        }

        // No connect/create responses when no headsets
        assert!(responses.is_empty());
    }

    #[tokio::test]
    async fn test_handle_headset_query_no_auto_connect() {
        // When auto_create_session is false, queryHeadsets should only emit
        // HeadsetsQueried and NOT produce connect_headset / create_session.
        let (tx, mut rx) = mpsc::channel(16);
        let config = CortexClientConfig {
            client_id: "test".into(),
            client_secret: "test".into(),
            auto_create_session: false,
            ..Default::default()
        };
        let state = Arc::new(Mutex::new(ClientState {
            auth_token: "tok".into(),
            session_id: String::new(),
            headset_id: String::new(),
        }));
        let result = serde_json::json!([
            {"id": "EPOCX-AAA", "status": "connected", "connectedBy": "dongle"},
            {"id": "INSIGHT-BBB", "status": "discovered", "connectedBy": ""}
        ]);
        let responses = handle_result(QUERY_HEADSET_ID, &result, &config, &state, &tx).await;

        // Should emit HeadsetsQueried with both headsets
        if let Some(CortexEvent::HeadsetsQueried(headsets)) = rx.recv().await {
            assert_eq!(headsets.len(), 2);
        } else {
            panic!("Expected HeadsetsQueried event");
        }

        // No connect/create_session responses — enumeration only
        assert!(responses.is_empty(), "expected no side-effect responses, got {responses:?}");
        // headset_id should NOT be set
        assert!(state.lock().await.headset_id.is_empty());
    }
}
