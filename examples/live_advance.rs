//! Advanced live mental command — mirrors `cortex-example/python/live_advance.py`.
//!
//! Loads a trained profile, reads and sets mental command sensitivity, then
//! subscribes to the `com` stream to show live mental command detections.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example live_advance
//! ```
//!
//! # API credentials
//!
//! Requires `EMOTIV_CLIENT_ID` and `EMOTIV_CLIENT_SECRET` environment variables.
//! Create them at <https://www.emotiv.com/my-account/cortex-apps/>.

use anyhow::Result;

use emotiv::client::{CortexClient, CortexClientConfig};
use emotiv::types::*;

const PROFILE_NAME: &str = "rust_training_profile";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let client_id = std::env::var("EMOTIV_CLIENT_ID").expect("Set EMOTIV_CLIENT_ID");
    let client_secret = std::env::var("EMOTIV_CLIENT_SECRET").expect("Set EMOTIV_CLIENT_SECRET");

    let config = CortexClientConfig { client_id, client_secret, ..Default::default() };
    let client = CortexClient::new(config);
    let (mut rx, handle) = client.connect().await?;
    let mut sensitivity_got = false;

    while let Some(event) = rx.recv().await {
        match event {
            CortexEvent::SessionCreated(_) => { handle.query_profile().await?; }
            CortexEvent::ProfilesQueried(profiles) => {
                if profiles.contains(&PROFILE_NAME.to_string()) { handle.get_current_profile().await?; }
                else { handle.setup_profile(PROFILE_NAME, "create").await?; }
            }
            CortexEvent::ProfileLoaded(true) => { handle.get_mc_active_action(PROFILE_NAME).await?; }
            CortexEvent::McActiveActions(data) => { println!("Active actions: {data}"); handle.get_mc_sensitivity(PROFILE_NAME).await?; }
            CortexEvent::McSensitivity(data) => {
                println!("Sensitivity: {data}");
                if !sensitivity_got { sensitivity_got=true; handle.set_mc_sensitivity(PROFILE_NAME, &[7,7,5,5]).await?; }
                else { handle.setup_profile(PROFILE_NAME, "save").await?; }
            }
            CortexEvent::ProfileSaved => { println!("Profile saved. Subscribing to com..."); handle.subscribe(&["com"]).await?; }
            CortexEvent::MentalCommand(data) => { println!("mc data: action={:<10} power={:.3} time={:.3}", data.action, data.power, data.time); }
            CortexEvent::Error(msg) => eprintln!("Error: {msg}"),
            CortexEvent::Disconnected => { println!("Disconnected"); break; }
            _ => {}
        }
    }
    Ok(())
}
