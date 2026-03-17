//! # Configuration
//!
//! [`CortexConfig`] is the rich configuration type for connecting to the Cortex API.
//! It can be built programmatically, loaded from environment variables, or loaded
//! from a TOML config file.
//!
//! ## Loading Priority
//!
//! Configuration is loaded from the first source that provides a value:
//!
//! 1. Explicit struct fields (programmatic construction)
//! 2. Environment variables (`EMOTIV_CLIENT_ID`, `EMOTIV_CLIENT_SECRET`, …)
//! 3. TOML config file at an explicit path
//! 4. `./cortex.toml` in the current directory
//! 5. `~/.config/emotiv/cortex.toml`
//!
//! Individual fields can always be overridden by environment variables,
//! even when loading from a file.
//!
//! ## Example
//!
//! ```no_run
//! use emotiv::config::CortexConfig;
//!
//! // Minimal programmatic config
//! let config = CortexConfig::new("my-client-id", "my-client-secret");
//!
//! // From env vars (EMOTIV_CLIENT_ID + EMOTIV_CLIENT_SECRET)
//! let config = CortexConfig::from_env().expect("missing env vars");
//!
//! // Auto-discover: file → env → defaults
//! let config = CortexConfig::discover(None).expect("no credentials found");
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::{CortexError, CortexResult};
use crate::protocol::CORTEX_WS_URL;

// ─── Defaults ────────────────────────────────────────────────────────────────

const DEFAULT_RPC_TIMEOUT_SECS: u64 = 10;
const DEFAULT_SUBSCRIBE_TIMEOUT_SECS: u64 = 15;
const DEFAULT_HEADSET_CONNECT_TIMEOUT_SECS: u64 = 30;
const DEFAULT_RECONNECT_BASE_DELAY_SECS: u64 = 1;
const DEFAULT_RECONNECT_MAX_DELAY_SECS: u64 = 60;
const DEFAULT_RECONNECT_MAX_ATTEMPTS: u32 = 0; // 0 = unlimited
const DEFAULT_HEALTH_INTERVAL_SECS: u64 = 30;
const DEFAULT_HEALTH_MAX_FAILURES: u32 = 3;

// ─── Config types ────────────────────────────────────────────────────────────

/// Full configuration for connecting to the Emotiv Cortex API.
///
/// Use [`CortexConfig::new`] for a quick minimal setup,
/// [`CortexConfig::from_env`] for 12-factor-style configuration, or
/// [`CortexConfig::discover`] in CLI tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CortexConfig {
    /// Cortex App client ID from the [Emotiv Developer Portal](https://www.emotiv.com/developer/).
    pub client_id: String,

    /// Cortex App client secret.
    pub client_secret: String,

    /// WebSocket URL of the Cortex service.
    ///
    /// The EMOTIV Launcher listens on `wss://localhost:6868` by default.
    #[serde(default = "default_cortex_url")]
    pub cortex_url: String,

    /// Optional Emotiv license key for commercial / premium features.
    #[serde(default)]
    pub license: Option<String>,

    /// Number of sessions to debit from the license per connection.
    #[serde(default = "default_debit")]
    pub debit: i64,

    /// Target headset ID (e.g. `"EPOCX-ABCDEF12"`).
    ///
    /// If empty, the first discovered headset is used.
    #[serde(default)]
    pub headset_id: String,

    /// Automatically create a session after authorization.
    ///
    /// Set to `false` if you only need records/profiles without a live headset.
    #[serde(default = "default_true")]
    pub auto_create_session: bool,

    /// Log every outgoing and incoming WebSocket message at `debug` level.
    #[serde(default)]
    pub debug_mode: bool,

    /// Timeout configuration.
    #[serde(default)]
    pub timeouts: TimeoutConfig,

    /// Auto-reconnect configuration.
    #[serde(default)]
    pub reconnect: ReconnectConfig,

    /// Health monitoring configuration.
    #[serde(default)]
    pub health: HealthConfig,
}

/// Timeout settings for various Cortex operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutConfig {
    /// Timeout for individual JSON-RPC calls, in seconds.
    #[serde(default = "default_rpc_timeout")]
    pub rpc_timeout_secs: u64,

    /// Timeout for stream subscribe operations, in seconds.
    #[serde(default = "default_subscribe_timeout")]
    pub subscribe_timeout_secs: u64,

    /// Timeout for headset connection, in seconds.
    #[serde(default = "default_headset_connect_timeout")]
    pub headset_connect_timeout_secs: u64,
}

/// Auto-reconnect behavior when the WebSocket connection drops.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconnectConfig {
    /// Enable auto-reconnect on connection loss.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Initial delay before the first reconnect attempt, in seconds.
    #[serde(default = "default_reconnect_base_delay")]
    pub base_delay_secs: u64,

    /// Maximum delay between reconnect attempts (exponential backoff cap), in seconds.
    #[serde(default = "default_reconnect_max_delay")]
    pub max_delay_secs: u64,

    /// Maximum number of reconnect attempts. `0` means unlimited.
    #[serde(default = "default_reconnect_max_attempts")]
    pub max_attempts: u32,
}

/// Health monitoring configuration (periodic liveness heartbeat).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthConfig {
    /// Enable periodic health checks.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Interval between health check calls, in seconds.
    #[serde(default = "default_health_interval")]
    pub interval_secs: u64,

    /// Number of consecutive health check failures before triggering reconnect.
    #[serde(default = "default_health_max_failures")]
    pub max_consecutive_failures: u32,
}

// ─── Default fns (required by `#[serde(default = "...")]`) ──────────────────

fn default_cortex_url() -> String {
    CORTEX_WS_URL.to_string()
}
fn default_debit() -> i64 {
    10
}
fn default_true() -> bool {
    true
}
fn default_rpc_timeout() -> u64 {
    DEFAULT_RPC_TIMEOUT_SECS
}
fn default_subscribe_timeout() -> u64 {
    DEFAULT_SUBSCRIBE_TIMEOUT_SECS
}
fn default_headset_connect_timeout() -> u64 {
    DEFAULT_HEADSET_CONNECT_TIMEOUT_SECS
}
fn default_reconnect_base_delay() -> u64 {
    DEFAULT_RECONNECT_BASE_DELAY_SECS
}
fn default_reconnect_max_delay() -> u64 {
    DEFAULT_RECONNECT_MAX_DELAY_SECS
}
fn default_reconnect_max_attempts() -> u32 {
    DEFAULT_RECONNECT_MAX_ATTEMPTS
}
fn default_health_interval() -> u64 {
    DEFAULT_HEALTH_INTERVAL_SECS
}
fn default_health_max_failures() -> u32 {
    DEFAULT_HEALTH_MAX_FAILURES
}

// ─── Default trait impls ─────────────────────────────────────────────────────

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            rpc_timeout_secs: DEFAULT_RPC_TIMEOUT_SECS,
            subscribe_timeout_secs: DEFAULT_SUBSCRIBE_TIMEOUT_SECS,
            headset_connect_timeout_secs: DEFAULT_HEADSET_CONNECT_TIMEOUT_SECS,
        }
    }
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_delay_secs: DEFAULT_RECONNECT_BASE_DELAY_SECS,
            max_delay_secs: DEFAULT_RECONNECT_MAX_DELAY_SECS,
            max_attempts: DEFAULT_RECONNECT_MAX_ATTEMPTS,
        }
    }
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: DEFAULT_HEALTH_INTERVAL_SECS,
            max_consecutive_failures: DEFAULT_HEALTH_MAX_FAILURES,
        }
    }
}

// ─── CortexConfig impl ───────────────────────────────────────────────────────

impl CortexConfig {
    /// Create a config with just client credentials; all other fields use defaults.
    ///
    /// # Example
    ///
    /// ```
    /// use emotiv::config::CortexConfig;
    ///
    /// let config = CortexConfig::new("my-client-id", "my-client-secret");
    /// assert_eq!(config.cortex_url, "wss://localhost:6868");
    /// assert!(config.reconnect.enabled);
    /// ```
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            cortex_url: default_cortex_url(),
            license: None,
            debit: 10,
            headset_id: String::new(),
            auto_create_session: true,
            debug_mode: false,
            timeouts: TimeoutConfig::default(),
            reconnect: ReconnectConfig::default(),
            health: HealthConfig::default(),
        }
    }

    /// Load config from environment variables.
    ///
    /// **Required:** `EMOTIV_CLIENT_ID`, `EMOTIV_CLIENT_SECRET`
    ///
    /// **Optional:** `EMOTIV_CORTEX_URL`, `EMOTIV_LICENSE`, `EMOTIV_HEADSET_ID`
    ///
    /// # Errors
    ///
    /// Returns [`CortexError::ConfigError`] if `EMOTIV_CLIENT_ID` or
    /// `EMOTIV_CLIENT_SECRET` are not set.
    pub fn from_env() -> CortexResult<Self> {
        let client_id =
            std::env::var("EMOTIV_CLIENT_ID").map_err(|_| CortexError::ConfigError {
                reason: "EMOTIV_CLIENT_ID environment variable not set".into(),
            })?;
        let client_secret =
            std::env::var("EMOTIV_CLIENT_SECRET").map_err(|_| CortexError::ConfigError {
                reason: "EMOTIV_CLIENT_SECRET environment variable not set".into(),
            })?;

        let mut config = Self::new(client_id, client_secret);

        if let Ok(url) = std::env::var("EMOTIV_CORTEX_URL") {
            config.cortex_url = url;
        }
        if let Ok(license) = std::env::var("EMOTIV_LICENSE") {
            config.license = Some(license);
        }
        if let Ok(headset) = std::env::var("EMOTIV_HEADSET_ID") {
            config.headset_id = headset;
        }

        Ok(config)
    }

    /// Load config from a TOML file, with environment variable overrides.
    ///
    /// Environment variables take precedence over file values for
    /// `client_id`, `client_secret`, `cortex_url`, `license`, and `headset_id`.
    ///
    /// # Errors
    ///
    /// Returns [`CortexError::ConfigError`] on file read failure or
    /// TOML parse failure (requires `config-toml` feature).
    pub fn from_file(path: impl AsRef<Path>) -> CortexResult<Self> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path).map_err(|e| CortexError::ConfigError {
            reason: format!("Failed to read config file '{}': {e}", path.display()),
        })?;

        #[cfg(feature = "config-toml")]
        let mut config: Self = toml::from_str(&contents).map_err(|e| CortexError::ConfigError {
            reason: format!("Failed to parse config file '{}': {e}", path.display()),
        })?;

        #[cfg(not(feature = "config-toml"))]
        let mut config: Self = serde_json::from_str(&contents).map_err(|e| CortexError::ConfigError {
            reason: format!("Failed to parse config file (JSON) '{}': {e}. Enable the `config-toml` feature for TOML support.", path.display()),
        })?;

        // Environment variable overrides
        if let Ok(id) = std::env::var("EMOTIV_CLIENT_ID") {
            config.client_id = id;
        }
        if let Ok(secret) = std::env::var("EMOTIV_CLIENT_SECRET") {
            config.client_secret = secret;
        }
        if let Ok(url) = std::env::var("EMOTIV_CORTEX_URL") {
            config.cortex_url = url;
        }
        if let Ok(license) = std::env::var("EMOTIV_LICENSE") {
            config.license = Some(license);
        }
        if let Ok(headset) = std::env::var("EMOTIV_HEADSET_ID") {
            config.headset_id = headset;
        }

        Ok(config)
    }

    /// Discover and load config from the standard search path.
    ///
    /// Search order:
    ///
    /// 1. Explicit path (if `Some`)
    /// 2. `CORTEX_CONFIG` environment variable
    /// 3. `./cortex.toml`
    /// 4. `~/.config/emotiv/cortex.toml`
    /// 5. Environment variables only
    ///
    /// # Errors
    ///
    /// Returns [`CortexError::ConfigError`] if no credentials can be found
    /// from any source.
    pub fn discover(explicit_path: Option<&Path>) -> CortexResult<Self> {
        // 1. Explicit path
        if let Some(path) = explicit_path {
            return Self::from_file(path);
        }

        // 2. CORTEX_CONFIG env var
        if let Ok(path_str) = std::env::var("CORTEX_CONFIG") {
            let path = PathBuf::from(path_str);
            if path.exists() {
                return Self::from_file(&path);
            }
        }

        // 3. ./cortex.toml
        let local = PathBuf::from("cortex.toml");
        if local.exists() {
            return Self::from_file(&local);
        }

        // 4. ~/.config/emotiv/cortex.toml
        if let Some(home) = dirs_config_path() {
            if home.exists() {
                return Self::from_file(&home);
            }
        }

        // 5. Environment variables
        Self::from_env()
    }

    /// Convert into a [`crate::client::CortexClientConfig`] for use with
    /// the low-level [`crate::client::CortexClient`].
    pub fn to_client_config(&self) -> crate::client::CortexClientConfig {
        crate::client::CortexClientConfig {
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            license: self.license.clone().unwrap_or_default(),
            debit: self.debit,
            headset_id: self.headset_id.clone(),
            auto_create_session: self.auto_create_session,
            ws_url: self.cortex_url.clone(),
            debug_mode: self.debug_mode,
        }
    }
}

/// Returns `~/.config/emotiv/cortex.toml` if the home directory can be found.
fn dirs_config_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config").join("emotiv").join("cortex.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_defaults() {
        let c = CortexConfig::new("id", "secret");
        assert_eq!(c.client_id, "id");
        assert_eq!(c.client_secret, "secret");
        assert_eq!(c.cortex_url, CORTEX_WS_URL);
        assert!(c.reconnect.enabled);
        assert!(c.health.enabled);
        assert_eq!(c.reconnect.base_delay_secs, 1);
        assert_eq!(c.reconnect.max_delay_secs, 60);
        assert_eq!(c.health.interval_secs, 30);
    }

    #[test]
    fn test_to_client_config() {
        let c = CortexConfig::new("id", "secret");
        let cc = c.to_client_config();
        assert_eq!(cc.client_id, "id");
        assert_eq!(cc.client_secret, "secret");
        assert_eq!(cc.ws_url, CORTEX_WS_URL);
    }

    #[test]
    fn test_timeout_defaults() {
        let t = TimeoutConfig::default();
        assert_eq!(t.rpc_timeout_secs, 10);
        assert_eq!(t.subscribe_timeout_secs, 15);
        assert_eq!(t.headset_connect_timeout_secs, 30);
    }

    #[test]
    fn test_reconnect_defaults() {
        let r = ReconnectConfig::default();
        assert!(r.enabled);
        assert_eq!(r.base_delay_secs, 1);
        assert_eq!(r.max_delay_secs, 60);
        assert_eq!(r.max_attempts, 0); // unlimited
    }

    #[test]
    fn test_health_defaults() {
        let h = HealthConfig::default();
        assert!(h.enabled);
        assert_eq!(h.interval_secs, 30);
        assert_eq!(h.max_consecutive_failures, 3);
    }
}
