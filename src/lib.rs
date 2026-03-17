//! # emotiv
//!
//! Async Rust client for streaming EEG and BCI data from
//! [Emotiv](https://www.emotiv.com/) headsets via the
//! [Cortex API](https://emotiv.gitbook.io/cortex-api/) WebSocket (JSON-RPC 2.0).
//!
//! ## Supported hardware
//!
//! | Model | EEG ch | BCI | Notes |
//! |---|---|---|---|
//! | EPOC X | 14 | ✓ | Full EEG, motion, performance metrics, mental commands |
//! | EPOC+ | 14 | ✓ | Same protocol as EPOC X |
//! | Insight | 5 | ✓ | Lightweight 5-channel headset |
//! | EPOC Flex | 32 | ✓ | Research-grade flexible cap |
//!
//! ## API credentials
//!
//! To connect to a real headset you need a **Client ID** and **Client Secret**
//! from the Emotiv developer portal:
//!
//! 1. Install and log into the [EMOTIV Launcher](https://www.emotiv.com/products/emotiv-launcher).
//! 2. Create a Cortex App at <https://www.emotiv.com/my-account/cortex-apps/>.
//! 3. Pass the credentials via [`CortexClientConfig`](client::CortexClientConfig)
//!    or the `EMOTIV_CLIENT_ID` / `EMOTIV_CLIENT_SECRET` environment variables.
//!
//! On the first connection the Cortex service will prompt you to approve the app
//! inside the EMOTIV Launcher (one-time only).
//!
//! ## Quick start
//!
//! ```no_run
//! use emotiv::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = CortexClientConfig {
//!         client_id: std::env::var("EMOTIV_CLIENT_ID")
//!             .expect("set EMOTIV_CLIENT_ID"),
//!         client_secret: std::env::var("EMOTIV_CLIENT_SECRET")
//!             .expect("set EMOTIV_CLIENT_SECRET"),
//!         ..Default::default()
//!     };
//!     let client = CortexClient::new(config);
//!     let (mut rx, handle) = client.connect().await?;
//!
//!     while let Some(event) = rx.recv().await {
//!         match event {
//!             CortexEvent::SessionCreated(_) => {
//!                 handle.subscribe(&["eeg", "mot", "met", "pow"]).await?;
//!             }
//!             CortexEvent::Eeg(data) => {
//!                 println!("EEG: {:?}", &data.samples[..5.min(data.samples.len())]);
//!             }
//!             CortexEvent::Disconnected => break,
//!             _ => {}
//!         }
//!     }
//!     Ok(())
//! }
//! ```
//!
//! ## Simulation mode (feature = `simulate`)
//!
//! For testing without hardware or API keys, enable the `simulate` feature:
//!
//! ```toml
//! [dependencies]
//! emotiv = { version = "0.0.1", features = ["simulate"] }
//! ```
//!
//! Then use [`simulator::spawn_simulator`] to generate synthetic data
//! through the same [`CortexEvent`](types::CortexEvent) channel.
//!
//! ## Module overview
//!
//! | Module | Purpose |
//! |---|---|
//! | [`prelude`] | One-line glob import of the most commonly needed types |
//! | [`client`] | WebSocket connection, auth flow, and the [`CortexHandle`](client::CortexHandle) command API |
//! | [`types`] | All event and data types ([`CortexEvent`](types::CortexEvent), [`EegData`](types::EegData), etc.) |
//! | [`protocol`] | JSON-RPC request builders and Cortex API constants |
//! | [`simulator`] | Signal simulator for offline testing *(feature = `simulate`)* |

pub mod client;
pub mod config;
pub mod error;
pub mod health;
pub mod protocol;
pub mod reconnect;
pub mod retry;
#[cfg(feature = "simulate")]
pub mod simulator;
pub mod types;

/// Convenience re-exports for downstream crates.
///
/// ```
/// use emotiv::prelude::*;
/// ```
///
/// This brings in [`CortexClient`](client::CortexClient),
/// [`CortexClientConfig`](client::CortexClientConfig),
/// [`CortexHandle`](client::CortexHandle), all event/data types, and the
/// stream-name constants (`STREAM_EEG`, `STREAM_MOT`, …).
pub mod prelude {
    pub use crate::client::{CortexClient, CortexClientConfig, CortexHandle};
    pub use crate::config::CortexConfig;
    pub use crate::error::{CortexError, CortexResult};
    pub use crate::reconnect::{ConnectionEvent, ResilientClient};
    pub use crate::types::*;
    pub use crate::protocol::{
        EEG_FREQUENCY,
        STREAM_EEG, STREAM_MOT, STREAM_DEV, STREAM_MET, STREAM_POW,
        STREAM_COM, STREAM_FAC, STREAM_SYS,
    };
}
