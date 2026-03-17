//! Facial expression training — mirrors `cortex-example/python/facial_expression_train.py`.
//!
//! Loads (or creates) a training profile, then trains neutral / surprise / smile
//! facial expression actions one by one via the `sys` stream.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example facial_expression_train
//! ```
//!
//! # API credentials
//!
//! Requires `EMOTIV_CLIENT_ID` and `EMOTIV_CLIENT_SECRET` environment variables.
//! Create them at <https://www.emotiv.com/my-account/cortex-apps/>.

use anyhow::Result;

use emotiv::client::{CortexClient, CortexClientConfig};
use emotiv::types::*;

const PROFILE_NAME: &str = "rust_fe_training_profile";
const ACTIONS: &[&str] = &["neutral", "surprise", "smile"];

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
            CortexEvent::SessionCreated(_) => { handle.query_profile().await?; }
            CortexEvent::ProfilesQueried(profiles) => {
                if profiles.contains(&PROFILE_NAME.to_string()) { handle.get_current_profile().await?; }
                else { handle.setup_profile(PROFILE_NAME, "create").await?; }
            }
            CortexEvent::ProfileLoaded(is_loaded) => {
                if is_loaded { handle.subscribe(&["sys"]).await?; }
                else { println!("Profile unloaded."); break; }
            }
            CortexEvent::DataLabels(labels) => {
                if labels.stream_name == "sys" {
                    if action_idx < ACTIONS.len() { handle.train("facialExpression", ACTIONS[action_idx], "start").await?; }
                }
            }
            CortexEvent::Sys(data) => {
                if let Some(train_event) = data.events.get(1).and_then(|v| v.as_str()) {
                    let action = ACTIONS.get(action_idx).unwrap_or(&"?");
                    println!("Training event: {action} -> {train_event}");
                    match train_event {
                        "FE_Succeeded" => { handle.train("facialExpression", action, "accept").await?; }
                        "FE_Failed" => { handle.train("facialExpression", action, "reject").await?; }
                        "FE_Completed"|"FE_Rejected" => {
                            action_idx += 1;
                            if action_idx < ACTIONS.len() { handle.train("facialExpression", ACTIONS[action_idx], "start").await?; }
                            else { handle.setup_profile(PROFILE_NAME, "save").await?; }
                        }
                        _ => {}
                    }
                }
            }
            CortexEvent::ProfileSaved => { handle.setup_profile(PROFILE_NAME, "unload").await?; }
            CortexEvent::Error(msg) => eprintln!("Error: {msg}"),
            CortexEvent::Disconnected => { println!("Disconnected"); break; }
            _ => {}
        }
    }
    Ok(())
}
