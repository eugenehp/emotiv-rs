//! # Error Types
//!
//! Semantic error types for the Emotiv Cortex API. Every variant carries
//! enough context to diagnose the problem without digging through logs.
//!
//! ## Error Code Mapping
//!
//! The Cortex API returns numeric error codes in JSON-RPC error responses.
//! [`CortexError::from_api_error`] maps known codes to semantic variants
//! with actionable error messages.
//!
//! ## Quick start
//!
//! ```no_run
//! use emotiv::error::{CortexError, CortexResult};
//!
//! fn handle(err: CortexError) {
//!     match err {
//!         CortexError::TokenExpired => eprintln!("Re-authenticate"),
//!         CortexError::NoHeadsetFound => eprintln!("Headset off?"),
//!         e if e.is_retryable() => eprintln!("Transient, retry"),
//!         e => eprintln!("Fatal: {e}"),
//!     }
//! }
//! ```

use thiserror::Error;

/// Convenient `Result` alias for Cortex operations.
pub type CortexResult<T> = std::result::Result<T, CortexError>;

/// All errors that can occur when interacting with the Emotiv Cortex API.
#[derive(Error, Debug)]
pub enum CortexError {
    // ─── Connection ─────────────────────────────────────────────────
    /// Failed to establish a WebSocket connection to the Cortex service.
    #[error("Failed to connect to Cortex at {url}: {reason}. Is the EMOTIV Launcher running?")]
    ConnectionFailed { url: String, reason: String },

    /// WebSocket connection was lost after being established.
    #[error("Connection to Cortex lost: {reason}")]
    ConnectionLost { reason: String },

    /// The client is not connected to the Cortex service.
    #[error("Not connected to Cortex")]
    NotConnected,

    // ─── Authentication ─────────────────────────────────────────────
    /// Authentication failed (invalid `client_id`/`client_secret` or expired token).
    #[error(
        "Authentication failed: {reason}. \
         Check your client_id and client_secret from the Emotiv Developer Portal."
    )]
    AuthenticationFailed { reason: String },

    /// The Cortex token has expired and needs to be refreshed.
    #[error("Cortex token expired — re-authentication required")]
    TokenExpired,

    /// Access denied — the user hasn't approved the app in the Emotiv Launcher.
    #[error("Access denied: {reason}. Approve the application in the EMOTIV Launcher.")]
    AccessDenied { reason: String },

    /// User is not logged in to EmotivID in the Launcher.
    #[error("User not logged in to EmotivID. Open the EMOTIV Launcher and sign in.")]
    UserNotLoggedIn,

    /// The application has not been approved in the EMOTIV Launcher.
    #[error(
        "Application not approved. \
         Open the EMOTIV Launcher and approve access for your app."
    )]
    NotApproved,

    // ─── License ────────────────────────────────────────────────────
    /// License expired, invalid, or missing for the requested operation.
    #[error("Emotiv license error: {reason}")]
    LicenseError { reason: String },

    // ─── Headset ────────────────────────────────────────────────────
    /// No headset found (either not paired or not powered on).
    #[error("No headset found. Ensure the headset is powered on and within range.")]
    NoHeadsetFound,

    /// The headset is being used by another session or application.
    #[error("Headset is in use by another session")]
    HeadsetInUse,

    /// Headset connection failed or the headset disconnected unexpectedly.
    #[error("Headset connection error: {reason}")]
    HeadsetError { reason: String },

    // ─── Session ────────────────────────────────────────────────────
    /// Session-related error (create, update, or close failed).
    #[error("Session error: {reason}")]
    SessionError { reason: String },

    // ─── Streams ────────────────────────────────────────────────────
    /// Subscribe/unsubscribe failed for the requested streams.
    #[error("Stream error: {reason}")]
    StreamError { reason: String },

    // ─── API ────────────────────────────────────────────────────────
    /// Raw Cortex API error that doesn't map to a more specific variant.
    #[error("Cortex API error {code}: {message}")]
    ApiError { code: i32, message: String },

    /// The Cortex service is still starting up — try again shortly.
    #[error("Cortex service is starting up — retry in a few seconds")]
    CortexStarting,

    /// The requested API method was not found (likely a version mismatch).
    #[error("API method not found: {method}")]
    MethodNotFound { method: String },

    // ─── Timeout ────────────────────────────────────────────────────
    /// An operation timed out waiting for a response.
    #[error("Operation timed out after {seconds}s")]
    Timeout { seconds: u64 },

    // ─── Retry ──────────────────────────────────────────────────────
    /// All retry attempts have been exhausted.
    #[error("Operation failed after {attempts} attempts: {last_error}")]
    RetriesExhausted {
        attempts: u32,
        last_error: Box<CortexError>,
    },

    // ─── Protocol ───────────────────────────────────────────────────
    /// Received an unexpected or malformed message from the Cortex service.
    #[error("Protocol error: {reason}")]
    ProtocolError { reason: String },

    // ─── Config ─────────────────────────────────────────────────────
    /// Configuration error (missing, malformed, or invalid values).
    #[error("Configuration error: {reason}")]
    ConfigError { reason: String },

    // ─── WebSocket / Transport ──────────────────────────────────────
    /// Low-level WebSocket transport error.
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    /// TLS/SSL error during connection.
    #[error("TLS error: {0}")]
    Tls(String),

    // ─── I/O ────────────────────────────────────────────────────────
    /// Filesystem or I/O error (config file reading, etc.).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl CortexError {
    /// Map a Cortex API error code and message to the most specific error variant.
    ///
    /// Known error codes from the Cortex v2 API (2026-02):
    ///
    /// | Code    | Variant |
    /// |---------|---------|
    /// | -32601  | [`MethodNotFound`](CortexError::MethodNotFound) |
    /// | -32001  | [`NoHeadsetFound`](CortexError::NoHeadsetFound) |
    /// | -32002  | [`LicenseError`](CortexError::LicenseError) |
    /// | -32004  | [`NoHeadsetFound`](CortexError::NoHeadsetFound) |
    /// | -32005  | [`SessionError`](CortexError::SessionError) |
    /// | -32012  | [`SessionError`](CortexError::SessionError) |
    /// | -32014  | [`AuthenticationFailed`](CortexError::AuthenticationFailed) |
    /// | -32015  | [`TokenExpired`](CortexError::TokenExpired) |
    /// | -32016  | [`StreamError`](CortexError::StreamError) |
    /// | -32021  | [`AuthenticationFailed`](CortexError::AuthenticationFailed) |
    /// | -32024  | [`LicenseError`](CortexError::LicenseError) |
    /// | -32033  | [`UserNotLoggedIn`](CortexError::UserNotLoggedIn) |
    /// | -32142  | [`NotApproved`](CortexError::NotApproved) |
    /// | -32152  | [`HeadsetError`](CortexError::HeadsetError) |
    ///
    /// # Examples
    ///
    /// ```
    /// use emotiv::error::CortexError;
    ///
    /// let err = CortexError::from_api_error(-32015, "token expired");
    /// assert!(matches!(err, CortexError::TokenExpired));
    ///
    /// let err = CortexError::from_api_error(-32001, "no headset");
    /// assert!(matches!(err, CortexError::NoHeadsetFound));
    /// ```
    pub fn from_api_error(code: i32, message: impl Into<String>) -> Self {
        let message = message.into();
        match code {
            -32601 => CortexError::MethodNotFound {
                method: message.clone(),
            },
            -32001 | -32004 => CortexError::NoHeadsetFound,
            -32002 | -32024 => CortexError::LicenseError { reason: message },
            -32005 | -32012 => CortexError::SessionError { reason: message },
            -32014 | -32021 => CortexError::AuthenticationFailed { reason: message },
            -32015 => CortexError::TokenExpired,
            -32016 => CortexError::StreamError { reason: message },
            -32033 => CortexError::UserNotLoggedIn,
            -32142 => CortexError::NotApproved,
            -32152 => CortexError::HeadsetError { reason: message },
            // Legacy Cortex codes
            -32102 => CortexError::NotApproved,
            -32122 => CortexError::CortexStarting,
            _ => CortexError::ApiError { code, message },
        }
    }

    /// Returns `true` if this error is transient and the operation can be retried.
    ///
    /// # Examples
    ///
    /// ```
    /// use emotiv::error::CortexError;
    ///
    /// assert!(CortexError::Timeout { seconds: 10 }.is_retryable());
    /// assert!(CortexError::CortexStarting.is_retryable());
    /// assert!(!CortexError::NoHeadsetFound.is_retryable());
    /// ```
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            CortexError::ConnectionLost { .. }
                | CortexError::Timeout { .. }
                | CortexError::CortexStarting
                | CortexError::WebSocket(_)
        )
    }

    /// Returns `true` if this error indicates the connection is dead
    /// and a reconnect is needed.
    ///
    /// # Examples
    ///
    /// ```
    /// use emotiv::error::CortexError;
    ///
    /// assert!(CortexError::NotConnected.is_connection_error());
    /// assert!(!CortexError::TokenExpired.is_connection_error());
    /// ```
    #[must_use]
    pub fn is_connection_error(&self) -> bool {
        matches!(
            self,
            CortexError::ConnectionFailed { .. }
                | CortexError::ConnectionLost { .. }
                | CortexError::NotConnected
                | CortexError::WebSocket(_)
        )
    }
}

// ─── From impls for external error types ────────────────────────────────────

impl From<tokio_tungstenite::tungstenite::Error> for CortexError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        CortexError::WebSocket(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_api_error_known_codes() {
        assert!(matches!(
            CortexError::from_api_error(-32001, "no headset"),
            CortexError::NoHeadsetFound
        ));
        assert!(matches!(
            CortexError::from_api_error(-32002, "invalid license"),
            CortexError::LicenseError { .. }
        ));
        assert!(matches!(
            CortexError::from_api_error(-32004, "headset unavailable"),
            CortexError::NoHeadsetFound
        ));
        assert!(matches!(
            CortexError::from_api_error(-32005, "session already exists"),
            CortexError::SessionError { .. }
        ));
        assert!(matches!(
            CortexError::from_api_error(-32014, "invalid token"),
            CortexError::AuthenticationFailed { .. }
        ));
        assert!(matches!(
            CortexError::from_api_error(-32015, "expired token"),
            CortexError::TokenExpired
        ));
        assert!(matches!(
            CortexError::from_api_error(-32016, "invalid stream"),
            CortexError::StreamError { .. }
        ));
        assert!(matches!(
            CortexError::from_api_error(-32033, "not logged in"),
            CortexError::UserNotLoggedIn
        ));
        assert!(matches!(
            CortexError::from_api_error(-32142, "not approved"),
            CortexError::NotApproved
        ));
    }

    #[test]
    fn test_from_api_error_unknown_code() {
        let err = CortexError::from_api_error(-99999, "something else");
        assert!(matches!(err, CortexError::ApiError { code: -99999, .. }));
    }

    #[test]
    fn test_is_retryable() {
        assert!(CortexError::Timeout { seconds: 10 }.is_retryable());
        assert!(CortexError::CortexStarting.is_retryable());
        assert!(CortexError::ConnectionLost { reason: "test".into() }.is_retryable());
        assert!(!CortexError::NoHeadsetFound.is_retryable());
        assert!(!CortexError::TokenExpired.is_retryable());
        assert!(!CortexError::NotApproved.is_retryable());
    }

    #[test]
    fn test_is_connection_error() {
        assert!(CortexError::NotConnected.is_connection_error());
        assert!(CortexError::ConnectionLost { reason: "test".into() }.is_connection_error());
        assert!(!CortexError::TokenExpired.is_connection_error());
        assert!(!CortexError::NoHeadsetFound.is_connection_error());
    }

    #[test]
    fn test_error_display_messages() {
        let err = CortexError::from_api_error(-32015, "token expired");
        assert!(err.to_string().contains("re-authentication"));

        let err = CortexError::from_api_error(-32033, "not logged in");
        assert!(err.to_string().contains("EmotivID"));

        let err = CortexError::from_api_error(-32142, "not approved");
        assert!(err.to_string().contains("EMOTIV Launcher"));
    }
}
