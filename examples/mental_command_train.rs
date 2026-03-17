//! Mental command training — mirrors `cortex-example/python/mental_command_train.py`.
//!
//! Loads (or creates) a training profile, then trains neutral / push / pull
//! mental command actions one by one via the `sys` stream.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example mental_command_train
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
const ACTIONS: &[&str] = &["neutral", "push", "pull"];

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let client_id = std::env::var("EMOTIV_CLIENT_ID").expect("Set EMOTIV_CLIENT_ID");
    let client_secret = std::env::var("EMOTIV_CLIENT_SECRET").expect("Set EMOTIV_CLIENT_SECRET");

    let config = CortexClientConfig { client_id, client_secret, ..Default::default() };
    let client = CortexClient::new(config);
    let (mut rx, handle) = client.connect().await?;
    let mut action_idx = 0usize;

    while let Some(event) = rx.recv().await {
        match event {
            CortexEvent::SessionCreated(_) => { println!("Session created. Querying profiles..."); handle.query_profile().await?; }
            CortexEvent::ProfilesQueried(profiles) => {
                println!("Available profiles: {profiles:?}");
                if profiles.contains(&PROFILE_NAME.to_string()) { handle.get_current_profile().await?; }
                else { println!("Creating profile '{PROFILE_NAME}'..."); handle.setup_profile(PROFILE_NAME, "create").await?; }
            }
            CortexEvent::ProfileLoaded(is_loaded) => {
                if is_loaded { println!("Profile loaded. Subscribing to sys stream..."); handle.subscribe(&["sys"]).await?; }
                else { println!("Profile unloaded."); break; }
            }
            CortexEvent::DataLabels(labels) => {
                if labels.stream_name == "sys" {
                    println!("Sys stream subscribed. Starting training...");
                    if action_idx < ACTIONS.len() { handle.train("mentalCommand", ACTIONS[action_idx], "start").await?; }
                }
            }
            CortexEvent::Sys(data) => {
                if let Some(train_event) = data.events.get(1).and_then(|v| v.as_str()) {
                    let action = ACTIONS.get(action_idx).unwrap_or(&"?");
                    println!("Training event: {action} -> {train_event}");
                    match train_event {
                        "MC_Succeeded" => { handle.train("mentalCommand", action, "accept").await?; }
                        "MC_Failed" => { handle.train("mentalCommand", action, "reject").await?; }
                        "MC_Completed"|"MC_Rejected" => {
                            action_idx += 1;
                            if action_idx < ACTIONS.len() { handle.train("mentalCommand", ACTIONS[action_idx], "start").await?; }
                            else { println!("All actions trained. Saving profile..."); handle.setup_profile(PROFILE_NAME, "save").await?; }
                        }
                        _ => {}
                    }
                }
            }
            CortexEvent::ProfileSaved => { println!("Profile saved. Unloading..."); handle.setup_profile(PROFILE_NAME, "unload").await?; }
            CortexEvent::Error(msg) => eprintln!("Error: {msg}"),
            CortexEvent::Disconnected => { println!("Disconnected"); break; }
            _ => {}
        }
    }
    Ok(())
}
